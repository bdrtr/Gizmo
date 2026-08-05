//! The engine's first physics benchmark. Until this landed there were none.
//!
//! `gizmo-math` and `gizmo-core` had benches; the four physics crates had zero, which means
//! every performance number in `docs/ENGINE.md` was unreproducible and any change to the
//! solver, broadphase or narrowphase was a change nobody could measure. This session rewrote
//! a good deal of the joint solver and could not answer "did that get slower?" — that gap is
//! what this file closes.
//!
//! # What each scene isolates
//!
//! `PhysicsWorld::step` runs broadphase → narrowphase → solver → integrate, so a single
//! whole-world benchmark cannot tell you which of them moved. The scenes below are built so
//! that one phase dominates each:
//!
//! - **broadphase** — many bodies far enough apart that no pair ever reaches narrowphase.
//!   What is being measured is pair-finding over the AABB tree.
//! - **dense contacts** — the same body count packed so most pairs overlap. NOTE: this does
//!   NOT isolate narrowphase, and the first run proved it. Measured with the engine's own
//!   `PhysicsMetrics`, the split at 1024 bodies is broadphase 25 ms, narrowphase 36 ms,
//!   solver 669 ms — the SOLVER is 91% of it. There is no way to isolate narrowphase through
//!   `step`, because every contact it generates is then solved. Read this group as
//!   "solver under a large contact island", which is what it is.
//! - **solver** — a settled stack, where broad and narrow phases are cheap and stable and the
//!   iteration count over persistent contacts dominates.
//! - **joints** — a hanging chain, which has no contacts at all, so it is the joint solver
//!   almost alone.
//! - **full_step** — a realistic mixed scene, as the composite sanity check.
//!
//! # Reading the numbers
//!
//! Absolute timings are machine-specific and this repository does not commit them as a gate —
//! `.cargo/config.toml` alone (`lto=off`, `codegen-units=4`, capped jobs) makes them a lower
//! bound rather than a measurement of the engine's ceiling. What is worth comparing is the
//! same scene before and after a change, on the same machine, and the SHAPE across body
//! counts: broadphase and narrowphase should scale sub-quadratically, and if one starts
//! looking quadratic that is the finding.
//!
//! # What the first run found
//!
//! In the dense-contact scene the contact COUNT scales linearly with N (~20 contacts per body
//! at every size), but the solver's cost PER CONTACT does not hold constant:
//!
//! ```text
//!   N=64     1631 contacts   20.84 µs/contact
//!   N=256    4847 contacts   21.63 µs/contact
//!   N=1024  20754 contacts   32.26 µs/contact
//! ```
//!
//! `island_count` stays at 4 throughout, so the island is getting bigger and per-contact cost
//! grows with it. Not the adaptive iteration count — that scene runs at zero gravity, so
//! support depth stays under the `island_depth >= 5` threshold and the sweep count is fixed.
//! The cause is unidentified; it is where C2/C4 should start.
//!
//! Every scene is deterministic — fixed positions, no RNG — so run-to-run variation is the
//! machine, not the input.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use gizmo_math::Vec3;
use gizmo_physics_core::{BodyHandle, Collider, PhysicsMaterial, Transform};
use gizmo_physics_rigid::{Joint, JointData, PhysicsWorld, RigidBody, Velocity};
use std::hint::black_box;

const DT: f32 = 1.0 / 60.0;

fn ground(world: &mut PhysicsWorld) {
    let mut g = RigidBody::new_static();
    g.wake_up();
    world.add_body(
        BodyHandle::from_id(0),
        g,
        Transform::new(Vec3::new(0.0, -1.0, 0.0)),
        Velocity::default(),
        Collider::box_collider(Vec3::new(200.0, 1.0, 200.0)),
    );
}

fn dynamic_box(world: &mut PhysicsWorld, id: u32, pos: Vec3, half: f32) {
    let mut rb = RigidBody::new(1.0, true);
    rb.wake_up();
    let col = Collider::box_collider(Vec3::splat(half)).with_material(PhysicsMaterial::default());
    rb.update_inertia_from_collider(&col);
    world.add_body(
        BodyHandle::from_id(id),
        rb,
        Transform::new(pos),
        Velocity::default(),
        col,
    );
}

/// `n` bodies on a wide lattice, spaced far enough that no two AABBs ever meet. Broadphase
/// walks every body; narrowphase and the solver get nothing to do.
fn scene_broadphase(n: u32) -> PhysicsWorld {
    let mut world = PhysicsWorld::new().with_gravity(Vec3::ZERO);
    ground(&mut world);
    let side = (n as f32).sqrt().ceil() as u32;
    for i in 0..n {
        let (x, z) = ((i % side) as f32, (i / side) as f32);
        dynamic_box(&mut world, i + 1, Vec3::new(x * 4.0, 5.0, z * 4.0), 0.5);
    }
    world
}

/// The same `n`, packed so neighbours overlap — roughly 20 contacts per body at every size.
///
/// Do not read this as a narrowphase measurement: the engine's own `PhysicsMetrics` puts 91%
/// of the time in the solver at 1024 bodies. It is the solver under one large contact island.
fn scene_narrowphase(n: u32) -> PhysicsWorld {
    let mut world = PhysicsWorld::new().with_gravity(Vec3::ZERO);
    ground(&mut world);
    let side = (n as f32).sqrt().ceil() as u32;
    for i in 0..n {
        let (x, z) = ((i % side) as f32, (i / side) as f32);
        dynamic_box(&mut world, i + 1, Vec3::new(x * 0.9, 5.0, z * 0.9), 0.5);
    }
    world
}

/// A settled tower. Contacts persist frame to frame, so warm starting and the iteration loop
/// dominate rather than contact discovery.
fn scene_solver_stack(height: u32) -> PhysicsWorld {
    let mut world = PhysicsWorld::new();
    ground(&mut world);
    for i in 0..height {
        dynamic_box(&mut world, i + 1, Vec3::new(0.0, 0.5 + i as f32, 0.0), 0.5);
    }
    // Let it settle so the benchmark measures steady-state solving, not the drop.
    for _ in 0..120 {
        world.step(DT).ok();
    }
    world
}

/// A hanging chain. No contacts at all, so this is the joint solver on its own.
fn scene_joint_chain(links: u32) -> PhysicsWorld {
    let mut world = PhysicsWorld::new();
    let mut anchor = RigidBody::new_static();
    anchor.wake_up();
    world.add_body(
        BodyHandle::from_id(0),
        anchor,
        Transform::new(Vec3::new(0.0, 50.0, 0.0)),
        Velocity::default(),
        Collider::sphere(0.05),
    );
    for i in 1..=links {
        let mut rb = RigidBody::new(1.0, true);
        rb.wake_up();
        let col = Collider::box_collider(Vec3::splat(0.05));
        rb.update_inertia_from_collider(&col);
        world.add_body(
            BodyHandle::from_id(i),
            rb,
            Transform::new(Vec3::new(0.0, 50.0 - i as f32, 0.0)),
            Velocity::default(),
            col,
        );
        let mut j = Joint::rope(
            BodyHandle::from_id(i - 1),
            BodyHandle::from_id(i),
            Vec3::ZERO,
            Vec3::ZERO,
            1.0,
        );
        if let JointData::Distance(ref mut d) = j.data {
            d.min_length = 1.0; // rigid rod, so the solver has real work
        }
        world.joints.push(j);
    }
    for _ in 0..60 {
        world.step(DT).ok();
    }
    world
}

/// Boxes raining onto the ground: broad, narrow, solver and integration all active.
fn scene_mixed(n: u32) -> PhysicsWorld {
    let mut world = PhysicsWorld::new();
    ground(&mut world);
    let side = (n as f32).sqrt().ceil() as u32;
    for i in 0..n {
        let (x, z) = ((i % side) as f32, (i / side) as f32);
        dynamic_box(
            &mut world,
            i + 1,
            Vec3::new(x * 1.1 - 10.0, 1.0 + (i % 7) as f32, z * 1.1 - 10.0),
            0.5,
        );
    }
    for _ in 0..30 {
        world.step(DT).ok();
    }
    world
}

/// Each iteration REBUILDS its scene and steps it once.
///
/// Stepping a single world thousands of times measures a different simulation every time —
/// bodies settle, fall asleep, and the cost collapses toward zero, which reads as a speed-up
/// that is really the scene going quiet. `iter_batched` excludes the setup closure from the
/// timing, so rebuilding per iteration costs wall-clock but keeps every measured step running
/// against identical state.
///
/// `PhysicsWorld` is not `Clone`, so rebuilding is also the only option available — and it is
/// the honest one regardless.
fn broadphase(c: &mut Criterion) {
    let mut g = c.benchmark_group("broadphase_no_contacts");
    g.sample_size(30);
    for &n in &[64u32, 256, 1024] {
        g.throughput(Throughput::Elements(n as u64));
        g.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.iter_batched(
                || scene_broadphase(n),
                |mut w| black_box(w.step(DT).ok()),
                criterion::BatchSize::LargeInput,
            );
        });
    }
    g.finish();
}

fn dense_contacts(c: &mut Criterion) {
    let mut g = c.benchmark_group("dense_contacts_solver_bound");
    g.sample_size(30);
    for &n in &[64u32, 256, 1024] {
        g.throughput(Throughput::Elements(n as u64));
        g.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.iter_batched(
                || scene_narrowphase(n),
                |mut w| black_box(w.step(DT).ok()),
                criterion::BatchSize::LargeInput,
            );
        });
    }
    g.finish();
}

fn solver(c: &mut Criterion) {
    let mut g = c.benchmark_group("solver_settled_stack");
    g.sample_size(20);
    for &h in &[8u32, 24, 48] {
        g.throughput(Throughput::Elements(h as u64));
        g.bench_with_input(BenchmarkId::from_parameter(h), &h, |b, &h| {
            b.iter_batched(
                || scene_solver_stack(h),
                |mut w| black_box(w.step(DT).ok()),
                criterion::BatchSize::LargeInput,
            );
        });
    }
    g.finish();
}

fn joints(c: &mut Criterion) {
    let mut g = c.benchmark_group("joints_hanging_chain");
    g.sample_size(20);
    for &links in &[8u32, 32, 128] {
        g.throughput(Throughput::Elements(links as u64));
        g.bench_with_input(BenchmarkId::from_parameter(links), &links, |b, &links| {
            b.iter_batched(
                || scene_joint_chain(links),
                |mut w| black_box(w.step(DT).ok()),
                criterion::BatchSize::LargeInput,
            );
        });
    }
    g.finish();
}

fn full_step(c: &mut Criterion) {
    let mut g = c.benchmark_group("full_step_mixed");
    g.sample_size(20);
    for &n in &[128u32, 512] {
        g.throughput(Throughput::Elements(n as u64));
        g.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.iter_batched(
                || scene_mixed(n),
                |mut w| black_box(w.step(DT).ok()),
                criterion::BatchSize::LargeInput,
            );
        });
    }
    g.finish();
}

criterion_group!(benches, broadphase, dense_contacts, solver, joints, full_step);
criterion_main!(benches);
