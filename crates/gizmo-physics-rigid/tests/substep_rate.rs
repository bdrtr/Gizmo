//! A world's substep rate (`PhysicsWorld::with_substep_rate`): chosen when the world is built, 240 Hz
//! unless asked otherwise, and everything the world keeps in its units — the accumulator, the sleep
//! counters, a snapshot — follows it.

use gizmo_math::Vec3;
use gizmo_physics_core::{BodyHandle, Collider, Transform};
use gizmo_physics_rigid::{PhysicsWorld, RigidBody, SubstepRateError, Velocity, DEFAULT_SUBSTEP_RATE, SUBSTEP_RATES};

fn falling_ball(world: &mut PhysicsWorld) {
    let mut rb = RigidBody::new(1.0, true);
    rb.calculate_sphere_inertia(0.5);
    rb.linear_damping = 0.0;
    world.add_body(BodyHandle::from_id(1), rb, Transform::new(Vec3::new(0.0, 100.0, 0.0)), Velocity::default(), Collider::sphere(0.5));
}

fn resting_box(world: &mut PhysicsWorld) {
    world.add_body(
        BodyHandle::from_id(0),
        RigidBody::new_static(),
        Transform::new(Vec3::new(0.0, -1.0, 0.0)),
        Velocity::default(),
        Collider::box_collider(Vec3::new(20.0, 1.0, 20.0)),
    );
    let mut rb = RigidBody::new(1.0, true);
    rb.calculate_box_inertia(1.0, 1.0, 1.0);
    world.add_body(BodyHandle::from_id(1), rb, Transform::new(Vec3::new(0.0, 0.5, 0.0)), Velocity::default(), Collider::box_collider(Vec3::splat(0.5)));
}

#[test]
fn a_new_world_steps_at_240_hz_and_its_substep_is_the_old_constant_to_the_bit() {
    let world = PhysicsWorld::new();
    assert_eq!(world.substep_rate(), DEFAULT_SUBSTEP_RATE);
    assert_eq!(DEFAULT_SUBSTEP_RATE, 240);
    // `1.0 / 240.0` was a `const f32`: the default world must step by exactly those bits, or every
    // recorded hash moves.
    assert_eq!(world.substep_dt().to_bits(), (1.0f32 / 240.0).to_bits());
}

#[test]
fn a_rate_outside_the_accepted_range_is_refused_and_says_why() {
    for hz in [*SUBSTEP_RATES.start(), 60, 120, 240, *SUBSTEP_RATES.end()] {
        let world = PhysicsWorld::new().with_substep_rate(hz).expect("an accepted rate");
        assert_eq!(world.substep_rate(), hz);
    }
    for hz in [0, SUBSTEP_RATES.start() - 1, SUBSTEP_RATES.end() + 1] {
        let err = PhysicsWorld::new().with_substep_rate(hz).err().expect("refused");
        assert_eq!(err, SubstepRateError::OutOfRange { hz });
        assert!(err.to_string().contains(&hz.to_string()), "the message names the rate: {err}");
    }
}

/// The ceiling on substeps in one `step` is a time, so a quarter-second frame — the longest a step
/// takes in — is paid in full at any rate.
#[test]
fn the_longest_frame_a_step_takes_is_paid_in_full_at_any_rate() {
    for hz in [30u32, 60, 120, 240, 960] {
        let mut world = PhysicsWorld::new().with_substep_rate(hz).expect("rate");
        falling_ball(&mut world);
        world.step(10.0).expect("step"); // clamped to a quarter second
        let v = world.velocities[0].linear.y;
        // The whole substeps a quarter second holds; the part of one left waits for the next call.
        let ran = (0.25 / world.substep_dt() + 1e-4).floor() * world.substep_dt();
        assert!((v + 9.81 * ran).abs() < 2e-3, "{hz} Hz: {v} m/s after a clamped frame, not {}", -9.81 * ran);
        assert!(world.accumulator < world.substep_dt(), "{hz} Hz: {} s of debt", world.accumulator);
    }
}

#[test]
fn a_snapshot_puts_the_world_back_at_the_rate_it_was_taken_at() {
    let mut slow = PhysicsWorld::new().with_substep_rate(60).expect("rate");
    falling_ball(&mut slow);
    let snap = slow.snapshot();
    let mut world = PhysicsWorld::new();
    falling_ball(&mut world);
    world.restore_snapshot(&snap);
    assert_eq!(world.substep_rate(), 60, "the accumulator and the counters in it are in 60 Hz units");
    assert_eq!(world.state_hash(), slow.state_hash());
}

/// Two peers built at different rates diverge from the first step; the hash tells them apart before
/// they have moved at all.
#[test]
fn worlds_at_two_rates_never_hash_alike() {
    let mut a = PhysicsWorld::new();
    let mut b = PhysicsWorld::new().with_substep_rate(120).expect("rate");
    falling_ball(&mut a);
    falling_ball(&mut b);
    assert_ne!(a.state_hash(), b.state_hash());
}

/// What sleep was blind to below about 200 Hz: a box at rest on the ground sleeps at 60 and 120 Hz
/// as it does at 240, after the same quarter second of stillness.
#[test]
fn a_box_at_rest_sleeps_at_every_rate() {
    for hz in [60u32, 120, 240] {
        let mut world = PhysicsWorld::new().with_substep_rate(hz).expect("rate");
        resting_box(&mut world);
        assert_eq!(world.sleep_steps(), RigidBody::sleep_steps(1.0 / hz as f32));
        let asleep_at = (0..240).find(|_| {
            world.step(1.0 / 60.0).expect("step");
            world.rigid_bodies[1].is_sleeping
        });
        let frame = asleep_at.unwrap_or_else(|| panic!("{hz} Hz: the box never slept (|v| = {})", world.velocities[1].linear.length()));
        assert!(frame < 60, "{hz} Hz: asleep only at frame {frame}");
        let y = world.transforms[1].position.y;
        assert!((y - 0.5).abs() < 0.02, "{hz} Hz: the box rests at {y}");
    }
}
