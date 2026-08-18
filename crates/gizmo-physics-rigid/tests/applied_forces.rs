//! The applied-force channel: `RigidBody::force_accumulator` / `torque_accumulator`, and the
//! `Integrator` helpers that write velocity directly.
//!
//! This channel keeps producing defects for one reason, written down in docs/ENGINE.md §8: **it
//! has no production caller.** The engine's own systems never write to the accumulators, so
//! nothing in the workspace exercised the path a game's thruster, wind volume or conveyor takes —
//! and two separate defects lived there undisturbed until a measurement was written for something
//! else. The first was a 4× frame-rate dependence (fixed 2026-08-18, guarded in `friction_hold.rs`);
//! the second is the sleeping body below.
//!
//! Everything here is arithmetic on one or two bodies, so it runs in milliseconds and asserts
//! numbers rather than impressions.

use gizmo_core::world::World;
use gizmo_math::{Quat, Vec3};
use gizmo_physics_core::{Collider, Transform};
use gizmo_physics_rigid::components::{RigidBody, Velocity};
use gizmo_physics_rigid::integrator::Integrator;
use gizmo_physics_rigid::system::physics_step_system;
use gizmo_physics_rigid::world::PhysicsWorld;

const DT: f32 = 1.0 / 60.0;

/// A 1 kg box resting on a wide static plate, stepped long enough to fall asleep.
fn settled_box() -> (World, gizmo_core::Entity) {
    let mut w = World::new();
    w.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));

    let g = w.spawn();
    w.add_component(g, Transform::new(Vec3::new(0.0, -0.5, 0.0)));
    w.add_component(g, RigidBody::new_static());
    w.add_component(g, Velocity::default());
    w.add_component(g, Collider::box_collider(Vec3::new(20.0, 0.5, 20.0)));

    let b = w.spawn();
    w.add_component(b, Transform::new(Vec3::new(0.0, 0.25, 0.0)));
    let mut rb = RigidBody::new(1.0, true);
    let collider = Collider::box_collider(Vec3::splat(0.25));
    rb.update_inertia_from_collider(&collider);
    w.add_component(b, rb);
    w.add_component(b, Velocity::default());
    w.add_component(b, collider);

    // 10 s: settling takes a moment and sleeping needs 60 further qualifying steps.
    for _ in 0..600 {
        physics_step_system(&w, DT);
    }
    (w, b)
}

/// REGRESSION (measured 2026-08-18). A force written into `force_accumulator` must **wake** the
/// body it is written to.
///
/// Before the fix a settled 1 kg box given 50 N sideways for two seconds moved **0.0000 m** —
/// not less than it should have, *nothing*, silently, and for as long as it stayed asleep. The
/// integrator returns early on `is_sleeping`, so the accumulator was simply never read; and
/// nothing woke the body, because a force is not a velocity change and the sleep test only looks
/// at velocity. A thruster or a conveyor therefore stopped working at the moment its target came
/// to rest — the moment it is most obviously meant to work.
///
/// Every sibling path already kept this contract: `PhysicsWorld::apply_impulse` and `apply_force`
/// take `&mut RigidBody` in order to wake it (with a comment saying why), and the explosion system
/// wakes what it moves. The accumulators are public fields, so they were the one way in that had
/// nothing standing there.
#[test]
fn a_force_wakes_the_body_it_is_applied_to() {
    let (mut w, b) = settled_box();
    assert!(
        w.borrow::<RigidBody>().get(b.id()).expect("the box").is_sleeping,
        "the harness must actually get the box to sleep, or this test measures nothing"
    );
    let start = w.borrow::<Transform>().get(b.id()).expect("the box").position;

    for _ in 0..120 {
        {
            let mut bodies = w.borrow_mut::<RigidBody>();
            if let Some(mut rb) = bodies.get_mut(b.id()) {
                // Deliberately NO `wake_up()`: that is the call this test exists to make
                // unnecessary.
                rb.force_accumulator.x += 50.0;
            }
        }
        physics_step_system(&w, DT);
    }

    let moved = (w.borrow::<Transform>().get(b.id()).expect("the box").position - start).length();
    println!("[wake] 50 N for 2 s on a sleeping box: {moved:.4} m");
    assert!(
        moved > 1.0,
        "50 N for two seconds moved a 1 kg box {moved:.4} m — the force is being swallowed by \
         the sleep check again"
    );
}

/// The angular half of the same contract: a torque must wake a sleeping body too.
#[test]
fn a_torque_wakes_the_body_it_is_applied_to() {
    let (mut w, b) = settled_box();
    assert!(w.borrow::<RigidBody>().get(b.id()).expect("the box").is_sleeping);

    for _ in 0..120 {
        {
            let mut bodies = w.borrow_mut::<RigidBody>();
            if let Some(mut rb) = bodies.get_mut(b.id()) {
                rb.torque_accumulator.y += 5.0;
            }
        }
        physics_step_system(&w, DT);
    }

    let spin = w.borrow::<Velocity>().get(b.id()).expect("the box").angular.length();
    println!("[wake] 5 N·m for 2 s on a sleeping box: {spin:.4} rad/s");
    assert!(spin > 0.1, "a torque on a sleeping body produced {spin:.4} rad/s");
}

/// A torque is a torque at every frame rate — the angular twin of the force test in
/// `friction_hold.rs`, and the guard the angular channel did not have.
///
/// The defect that test was written for drained the accumulators on the **first substep** of each
/// frame, so a 1/60 s frame (four 1/240 s substeps) delivered a quarter of what was asked for.
/// Both accumulators are drained by the same line, so both were wrong and only one is guarded —
/// which means a future edit could restore the bug on the angular half and every test would pass.
///
/// No collider, no gravity, no contact: this is the torque channel and nothing else.
#[test]
fn a_torque_produces_the_same_angular_acceleration_at_every_frame_rate() {
    fn spin_after_one_second(dt: f32) -> f32 {
        let mut w = World::new();
        w.insert_resource(PhysicsWorld::new().with_gravity(Vec3::ZERO));
        let b = w.spawn();
        w.add_component(b, Transform::new(Vec3::ZERO));
        let mut rb = RigidBody::new(1.0, true);
        rb.local_inertia = Vec3::splat(1.0); // a unit sphere's worth, so ω = τ·t exactly
        rb.use_gravity = false;
        w.add_component(b, rb);
        w.add_component(b, Velocity::default());
        w.add_component(b, Collider::box_collider(Vec3::splat(0.25)));

        let steps = (1.0 / dt).round() as usize;
        for _ in 0..steps {
            {
                let mut bodies = w.borrow_mut::<RigidBody>();
                if let Some(mut rb) = bodies.get_mut(b.id()) {
                    rb.torque_accumulator.y += 2.0; // N·m, held for the whole second
                }
            }
            physics_step_system(&w, dt);
        }
        w.borrow::<Velocity>().get(b.id()).expect("the body").angular.y
    }

    let rates = [1.0 / 240.0, 1.0 / 120.0, 1.0 / 60.0, 1.0 / 30.0];
    let spins: Vec<f32> = rates.iter().map(|dt| spin_after_one_second(*dt)).collect();
    println!("[torque] ω after 1 s of 2 N·m at 240/120/60/30 fps: {spins:?}");

    let max = spins.iter().cloned().fold(f32::MIN, f32::max);
    let min = spins.iter().cloned().fold(f32::MAX, f32::min);
    let spread = (max - min) / max;
    assert!(
        spread < 0.02,
        "the same torque gave different spins at different frame rates (spread {spread:.3}, \
         {spins:?}) — the accumulator is being drained per substep again"
    );
}

/// `Integrator::apply_torque` must use the **world-space** inverse inertia tensor, and the test is
/// built so that a body-space shortcut cannot pass it.
///
/// The tensor is a property of the body's shape in its own frame; a torque is given in world
/// space. Rotating the body changes which of its axes the torque is pushing on, so the tensor has
/// to be rotated with it (`R·I⁻¹·Rᵀ`). A shortcut that uses the local diagonal directly is only
/// right while the body is unrotated or its inertia is isotropic — a long thin box turned on its
/// side then spins on the wrong axis, at the wrong rate. The comment in `integrator.rs` says that
/// was once a real defect; nothing checked it.
///
/// The body here is a rod along its own +Y (thin about that axis, heavy about X and Z), rotated
/// 90° about Z so the rod points along world +X. A torque about world X therefore spins it about
/// its own long axis, i.e. through the SMALL inertia. The body-space answer would divide by the
/// large one — an order of magnitude out, in the same direction, which is exactly the shape a
/// sloppy assertion misses.
#[test]
fn a_torque_is_resisted_by_the_inertia_the_body_actually_presents() {
    let mut rb = RigidBody::new(2.0, true);
    let rod = Collider::box_collider(Vec3::new(0.05, 1.0, 0.05)); // half-extents: long in Y
    rb.update_inertia_from_collider(&rod);
    rb.wake_up();

    let (i_long, i_across) = (rb.local_inertia.y, rb.local_inertia.x);
    assert!(
        i_across > i_long * 10.0,
        "the harness needs a genuinely anisotropic body: I_across {i_across} vs I_long {i_long}"
    );

    // Lay the rod along world +X.
    let transform = Transform::new(Vec3::ZERO)
        .with_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2));
    let mut vel = Velocity::default();

    let torque = Vec3::new(1.0, 0.0, 0.0); // N·m about world X
    let dt = 0.01;
    Integrator::apply_torque(&rb, &transform, &mut vel, torque, dt);

    let world_space_answer = torque.x * dt / i_long; // spins about its own long axis
    let body_space_answer = torque.x * dt / i_across; // what the shortcut would give
    println!(
        "[tensor] ω_x = {:.5} (world-space {:.5}, body-space shortcut {:.5})",
        vel.angular.x, world_space_answer, body_space_answer
    );

    assert!(
        (vel.angular.x - world_space_answer).abs() < world_space_answer * 0.02,
        "expected {world_space_answer:.5} rad/s about world X, got {:.5}",
        vel.angular.x
    );
    assert!(
        (vel.angular.x - body_space_answer).abs() > body_space_answer,
        "the measurement must be able to tell the two apart, or it is not testing anything"
    );
    // And nothing may appear on the axes the torque did not touch.
    assert!(
        vel.angular.y.abs() < 1e-6 && vel.angular.z.abs() < 1e-6,
        "a torque about world X produced off-axis spin: {:?}",
        vel.angular
    );
}

/// `Integrator::apply_force` and `apply_torque` are no-ops on a body that is not dynamic — the
/// guard every one of these helpers opens with, and one nothing covered.
#[test]
fn a_static_body_is_not_moved_by_a_force_or_a_torque() {
    let rb = RigidBody::new_static();
    let transform = Transform::new(Vec3::ZERO);
    let mut vel = Velocity::default();

    Integrator::apply_force(&rb, &mut vel, Vec3::new(1_000.0, 0.0, 0.0), 1.0);
    Integrator::apply_torque(&rb, &transform, &mut vel, Vec3::new(1_000.0, 0.0, 0.0), 1.0);

    assert_eq!(vel.linear, Vec3::ZERO, "a static body does not accelerate");
    assert_eq!(vel.angular, Vec3::ZERO, "and does not spin");
}
