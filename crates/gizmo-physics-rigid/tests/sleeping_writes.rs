//! A velocity written to a sleeping body must not become a stale impulse.
//!
//! # The defect
//!
//! Nothing reads a sleeping dynamic body's velocity. `advance_sleep_counter` — the only thing that
//! could wake a body because of its own speed — runs inside `integrate_velocities`, *below* that
//! function's `is_sleeping` early return, so the sleep state machine never runs on a sleeper. A
//! number written there is not integrated, not damped, and not spent. It waits.
//!
//! Then something unrelated wakes the body — an ended collision, an island mover, a joint, the
//! editor's button — and position integration picks the stored value up and applies it in full, on
//! that substep. The crate you nudged and gave up on launches when someone bumps the stack next to
//! it.
//!
//! The engine had already fixed one instance of this by waking at the writer (the explosion
//! system, pinned by `explosion.rs`). That made the contract in `RigidBody::wake_up`'s doc — change
//! a velocity, wake the body — true of one caller. `sync_bodies` now enforces it for all of them:
//! a velocity written to a sleeping dynamic body is **dropped** rather than banked, so a writer
//! that forgets loses the write immediately and visibly instead of arming a trap.
//!
//! # What each test would catch
//!
//! Both drive the real `physics_step_system`, so they exercise the ECS → `sync_bodies` → solver →
//! write-back round trip rather than `PhysicsWorld` in isolation. Removing the guard turns the
//! first one red; removing the wake in the writer turns the second one red.

use gizmo_core::world::World;
use gizmo_math::Vec3;
use gizmo_physics_core::{Collider, Transform};
use gizmo_physics_rigid::components::{RigidBody, Velocity};
use gizmo_physics_rigid::system::physics_step_system;
use gizmo_physics_rigid::world::PhysicsWorld;

/// Builds a floor and one unit box resting on it, stepped until the box is asleep.
fn settled_box() -> (World, gizmo_core::entity::Entity) {
    let mut world = World::new();
    // Without this resource `physics_step_system` warns and skips every frame, so nothing settles
    // and the precondition below fires — which is how the first version of this file failed.
    world.insert_resource(PhysicsWorld::new());

    let ground = world.spawn();
    world.add_component(ground, RigidBody::new_static());
    world.add_component(ground, Collider::box_collider(Vec3::new(50.0, 0.5, 50.0)));
    world.add_component(ground, Transform::new(Vec3::new(0.0, -0.5, 0.0)));
    world.add_component(ground, Velocity::default());

    let mut rb = RigidBody::new(1.0, true);
    rb.update_inertia_from_collider(&Collider::box_collider(Vec3::splat(0.5)));
    let body = world.spawn();
    world.add_component(body, rb);
    world.add_component(body, Collider::box_collider(Vec3::splat(0.5)));
    world.add_component(body, Transform::new(Vec3::new(0.0, 0.5, 0.0)));
    world.add_component(body, Velocity::default());

    // 240 Hz internally → 400 frames ≈ 1600 substeps against a 60-substep sleep requirement.
    // Same cadence as `explosion.rs`.
    for _ in 0..400 {
        physics_step_system(&world, 1.0 / 60.0);
    }
    (world, body)
}

fn x_of(world: &World, e: gizmo_core::entity::Entity) -> f32 {
    world.borrow::<Transform>().get(e.id()).expect("transform").position.x
}

fn is_asleep(world: &World, e: gizmo_core::entity::Entity) -> bool {
    world.borrow::<RigidBody>().get(e.id()).expect("body").is_sleeping
}

/// Writing a velocity to a sleeping body and walking away must not arm anything.
///
/// The assertion that fails without the guard is the **last** one: before the fix the 5 m/s sat in
/// the physics world until the wake, and the body then coasted metres from a push the player made
/// long before. With the guard the write is dropped, so waking the body later moves it by nothing.
#[test]
fn a_velocity_written_to_a_sleeping_body_does_not_fire_when_something_else_wakes_it() {
    let (mut world, body) = settled_box();
    assert!(
        is_asleep(&world, body),
        "scenario invalid: the box never fell asleep, so there is nothing to write into"
    );
    let x0 = x_of(&world, body);

    // Gameplay writes a velocity and does NOT wake the body — the mistake three writers in this
    // engine used to make.
    {
        let mut vels = world.borrow_mut::<Velocity>();
        vels.get_mut(body.id()).expect("velocity").linear = Vec3::new(5.0, 0.0, 0.0);
    }

    // One step: the write should have gone nowhere.
    physics_step_system(&world, 1.0 / 60.0);
    assert!(
        (x_of(&world, body) - x0).abs() < 1e-4,
        "a sleeping body moved from a velocity write that did not wake it"
    );

    // Now something unrelated wakes it — the editor's button, a neighbour, a blast.
    {
        let mut bodies = world.borrow_mut::<RigidBody>();
        bodies.get_mut(body.id()).expect("body").wake_up();
    }
    for _ in 0..60 {
        physics_step_system(&world, 1.0 / 60.0);
    }

    let travelled = (x_of(&world, body) - x0).abs();
    assert!(
        travelled < 0.05,
        "the body travelled {travelled:.3} m after being woken — the velocity written while it \
         slept was banked and fired as a stale impulse. It should have been dropped at the point \
         of the write."
    );
}

/// A writer that wakes the body gets its motion, immediately.
///
/// The other half of the contract, and the reason the guard is not simply "sleeping bodies ignore
/// gameplay": with the wake, the same write moves the body on the next step. Without this test the
/// guard could be "fixed" by dropping the write unconditionally and nothing would notice.
#[test]
fn the_same_write_works_when_the_writer_wakes_the_body() {
    let (mut world, body) = settled_box();
    assert!(is_asleep(&world, body), "scenario invalid: the box never fell asleep");
    let x0 = x_of(&world, body);

    {
        let mut bodies = world.borrow_mut::<RigidBody>();
        bodies.get_mut(body.id()).expect("body").wake_up();
    }
    {
        let mut vels = world.borrow_mut::<Velocity>();
        vels.get_mut(body.id()).expect("velocity").linear = Vec3::new(5.0, 0.0, 0.0);
    }

    for _ in 0..30 {
        physics_step_system(&world, 1.0 / 60.0);
    }

    let travelled = (x_of(&world, body) - x0).abs();
    assert!(
        travelled > 0.5,
        "the body only travelled {travelled:.3} m — a woken body must take the velocity it is \
         given, or the guard has turned into 'gameplay cannot push a settled object'"
    );
}

/// A sleeping neighbour in a solved island must not bank a velocity it will never spend.
///
/// # The half-awake island
///
/// The solver has no sleep check anywhere — a sleeping dynamic body still has finite inverse mass,
/// so it takes real impulses — and the island write-back used to store the result on every dynamic
/// member. Position integration then skips the sleeping ones. The velocity is banked, and fires
/// whenever something wakes the body.
///
/// Getting into that state is ordinary: the ended-collision path wakes both bodies of a pair
/// unconditionally, and an island only wakes its members wholesale when one of them is a *mover*
/// (speed above 0.05). Wake one body below that threshold and the island is solved with sleeping
/// members in it.
///
/// Gravity is off on purpose. At 240 Hz an awake resting body already carries `g·dt ≈ 0.0409`
/// against the 0.05 mover threshold, which leaves almost no headroom to place a sub-threshold
/// nudge — the scenario would be measuring the threshold, not the defect.
#[test]
fn a_sleeping_neighbour_does_not_bank_a_velocity_it_never_spends() {
    use gizmo_physics_core::BodyHandle;

    let mut pw = PhysicsWorld::new();
    pw.integrator.gravity = Vec3::ZERO;

    let collider = Collider::box_collider(Vec3::splat(0.5));
    let mut rb = RigidBody::new(1.0, true);
    rb.update_inertia_from_collider(&collider);

    // Two unit boxes overlapping slightly, so a manifold exists and they share an island.
    let a = BodyHandle::from_id(1);
    let b = BodyHandle::from_id(2);
    pw.add_body(a, rb, Transform::new(Vec3::new(0.0, 0.0, 0.0)), Velocity::default(), collider.clone());
    pw.add_body(b, rb, Transform::new(Vec3::new(0.98, 0.0, 0.0)), Velocity::default(), collider);

    for _ in 0..400 {
        pw.step(1.0 / 60.0).ok();
    }

    let idx_a = pw.entities.iter().position(|e| *e == a).expect("a");
    let idx_b = pw.entities.iter().position(|e| *e == b).expect("b");
    assert!(
        pw.rigid_bodies[idx_a].is_sleeping && pw.rigid_bodies[idx_b].is_sleeping,
        "scenario invalid: the pair never fell asleep, so there is no sleeping member to solve \
         around"
    );
    let v_b_before = pw.velocities[idx_b];

    // Wake A only, at a speed below the 0.05 mover threshold, so the island is solved without
    // waking B.
    pw.rigid_bodies[idx_a].wake_up();
    pw.velocities[idx_a].linear = Vec3::new(0.045, 0.0, 0.0);

    for _ in 0..20 {
        pw.step(1.0 / 60.0).ok();
    }

    assert!(
        pw.rigid_bodies[idx_b].is_sleeping,
        "scenario invalid: the island found a mover and woke everything, so nothing was solved \
         around a sleeper"
    );
    assert_eq!(
        pw.velocities[idx_b], v_b_before,
        "the sleeping neighbour banked a velocity from a solve it never integrates — it will fire \
         as a stale impulse the moment anything wakes it"
    );
}
