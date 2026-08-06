//! Counts heap allocations per `PhysicsWorld::step`, so a change whose entire claim is "fewer
//! allocations" can be stated as a number rather than argued from the source.
//!
//! This is deliberately NOT a timing benchmark. `benches/step_bench.rs` documents why one would
//! not work here: no committed scene isolates the narrowphase, and at 1024 bodies the split is
//! broadphase 25 ms / narrowphase 36 ms / solver 669 ms, so removing a 272-byte allocation per
//! contact pair is real but far below wall-clock noise. The allocation COUNT is exact, is the
//! thing that actually changed, and is reproducible run to run.
//!
//! Run: `cargo run --release -p gizmo-physics-rigid --example alloc_census`

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use gizmo_math::Vec3;
use gizmo_physics_core::{BodyHandle, Collider, PhysicsMaterial, Transform};
use gizmo_physics_rigid::{PhysicsWorld, RigidBody, Velocity};

static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicUsize = AtomicUsize::new(0);

struct Counting;

// SAFETY: every method forwards to `System` with the same arguments it was given; the only added
// work is a pair of relaxed atomic counters, which cannot affect allocator correctness.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(l.size(), Ordering::Relaxed);
        unsafe { System.alloc(l) }
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(l.size(), Ordering::Relaxed);
        unsafe { System.alloc_zeroed(l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, new_size: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(new_size, Ordering::Relaxed);
        unsafe { System.realloc(p, l, new_size) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

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
    world.add_body(BodyHandle::from_id(id), rb, Transform::new(pos), Velocity::default(), col);
}

/// A tower that is still awake and in contact — the shape `headless_stress_test` runs, and the
/// one where every contact is box-box or box-plane.
fn scene_stack(height: u32) -> PhysicsWorld {
    let mut world = PhysicsWorld::new();
    ground(&mut world);
    for i in 0..height {
        dynamic_box(&mut world, i + 1, Vec3::new(0.0, 0.5 + i as f32, 0.0), 0.5);
    }
    world
}

/// The packed raft from `step_bench.rs::scene_narrowphase` — ~20 contacts per body, zero gravity
/// so it never settles and never sleeps.
fn scene_raft(n: u32) -> PhysicsWorld {
    let mut world = PhysicsWorld::new().with_gravity(Vec3::ZERO);
    ground(&mut world);
    let side = (n as f32).sqrt().ceil() as u32;
    for i in 0..n {
        let (x, z) = ((i % side) as f32, (i / side) as f32);
        dynamic_box(&mut world, i + 1, Vec3::new(x * 0.9, 5.0, z * 0.9), 0.5);
    }
    world
}

fn measure(name: &str, mut world: PhysicsWorld, warmup: u32, steps: u32) {
    for _ in 0..warmup {
        world.step(DT).ok();
    }
    ALLOCS.store(0, Ordering::Relaxed);
    BYTES.store(0, Ordering::Relaxed);
    for _ in 0..steps {
        world.step(DT).ok();
    }
    let allocs = ALLOCS.load(Ordering::Relaxed);
    let bytes = BYTES.load(Ordering::Relaxed);
    println!(
        "{name:<24} {:>12} allocs  {:>10.1} /step  {:>12} bytes  {:>10.0} B/step",
        allocs,
        allocs as f64 / steps as f64,
        bytes,
        bytes as f64 / steps as f64,
    );
}

fn main() {
    println!("{:<24} {:>12} {:>16} {:>12} {:>16}", "scene", "allocs", "per step", "bytes", "per step");
    measure("stack/8 (awake)", scene_stack(8), 0, 60);
    measure("stack/24 (awake)", scene_stack(24), 0, 60);
    measure("stack/24 (settled)", scene_stack(24), 120, 60);
    measure("raft/64", scene_raft(64), 2, 30);
    measure("raft/256", scene_raft(256), 2, 30);
}
