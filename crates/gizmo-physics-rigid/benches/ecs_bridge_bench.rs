//! The ECS bridge — the engine's last large unmeasured piece.
//!
//! `benches/step_bench.rs` drives [`PhysicsWorld`] directly and never goes through
//! [`physics_step_system`], so everything between the ECS and the simulation had no benchmark at
//! all. That gap is why C3 (the O(N²)→O(N) writeback fix in `system.rs`) shipped as a complexity
//! argument with no measured speed-up: there was nothing to measure it with.
//!
//! # What the two groups mean
//!
//! Running the bridge alone tells you a number but not whether it is large. So this file measures
//! the same scene twice:
//!
//! - **`ecs_bridge`** — `physics_step_system(&world, dt)`: query the ECS, gather colliders,
//!   `sync_bodies`, step, write the results back into the ECS.
//! - **`physics_world_direct`** — `PhysicsWorld::step(dt)` on an identical set of bodies, with no
//!   ECS in the picture at all.
//!
//! **The difference between the two IS the bridge.** Both do the same simulation work, so the
//! solver, broadphase and integrator cancel out.
//!
//! # Why this scene
//!
//! Bodies are spread far enough apart that no pair ever reaches the narrowphase, and gravity is
//! left on with no floor, so they accelerate forever. That combination is deliberate:
//!
//! - no contacts → the solver has nothing to do, so it cannot drown out the bridge (the mistake
//!   `step_bench.rs`'s `dense_contacts` group documents, where 91% of the time turned out to be
//!   solver);
//! - always moving → nothing ever sleeps, so the per-body cost stays in the measurement rather
//!   than being optimised away by the island-collective sleep path.
//!
//! Read this as an upper bound on how much of a frame the bridge can be, not as a typical frame.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use gizmo_core::world::World;
use gizmo_math::Vec3;
use gizmo_physics_core::{BodyHandle, Collider, Transform};
use gizmo_physics_rigid::components::{RigidBody, Velocity};
use gizmo_physics_rigid::system::physics_step_system;
use gizmo_physics_rigid::PhysicsWorld;
use std::hint::black_box;

const DT: f32 = 1.0 / 60.0;

/// Lattice position for body `i`, spaced so no two AABBs ever meet.
fn lattice(i: u32, side: u32) -> Vec3 {
    let (x, z) = ((i % side) as f32, (i / side) as f32);
    Vec3::new(x * 4.0, 5.0, z * 4.0)
}

fn body(half: f32) -> (RigidBody, Collider) {
    let mut rb = RigidBody::new(1.0, true);
    rb.wake_up();
    let col = Collider::box_collider(Vec3::splat(half));
    rb.update_inertia_from_collider(&col);
    (rb, col)
}

/// `n` falling bodies in an ECS world, with `PhysicsWorld` as a resource.
fn scene_ecs(n: u32) -> World {
    let mut world = World::new();
    world.insert_resource(PhysicsWorld::new());
    let side = (n as f32).sqrt().ceil() as u32;
    for i in 0..n {
        let (rb, col) = body(0.5);
        let e = world.spawn();
        world.add_component(e, Transform::new(lattice(i, side)));
        world.add_component(e, rb);
        world.add_component(e, Velocity::default());
        world.add_component(e, col);
    }
    world
}

/// The same `n` bodies, built straight into a `PhysicsWorld` — no ECS.
fn scene_direct(n: u32) -> PhysicsWorld {
    let mut world = PhysicsWorld::new();
    let side = (n as f32).sqrt().ceil() as u32;
    for i in 0..n {
        let (rb, col) = body(0.5);
        world.add_body(
            BodyHandle::from_id(i),
            rb,
            Transform::new(lattice(i, side)),
            Velocity::default(),
            col,
        );
    }
    world
}

fn ecs_bridge(c: &mut Criterion) {
    let mut g = c.benchmark_group("ecs_bridge");
    g.sample_size(20);
    for &n in &[64u32, 256, 1024, 4096] {
        g.throughput(Throughput::Elements(n as u64));
        g.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.iter_batched(
                || scene_ecs(n),
                |world| {
                    physics_step_system(&world, DT);
                    black_box(())
                },
                criterion::BatchSize::LargeInput,
            );
        });
    }
    g.finish();
}

fn physics_world_direct(c: &mut Criterion) {
    let mut g = c.benchmark_group("physics_world_direct");
    g.sample_size(20);
    for &n in &[64u32, 256, 1024, 4096] {
        g.throughput(Throughput::Elements(n as u64));
        g.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.iter_batched(
                || scene_direct(n),
                |mut w| black_box(w.step(DT).ok()),
                criterion::BatchSize::LargeInput,
            );
        });
    }
    g.finish();
}

criterion_group!(benches, ecs_bridge, physics_world_direct);
criterion_main!(benches);
