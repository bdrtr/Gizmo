//! A body landing on the ground meets it, at any substep rate, instead of sinking a substep's travel
//! into it and being pushed back out: the narrow phase finds the contacts the substep's motion
//! closes before they touch (`pipeline::near_contacts`), and the solve lets each close by exactly
//! its gap.
//!
//! Measured 2026-11-24, before: a 50 kg box falling 2 m sank 6 cm at 60 Hz; a box dropped from 5 m
//! came to rest 1.35 mm into the ground at 240 Hz (`golden_box_settling_on_the_ground`).

use gizmo_math::{Quat, Vec3};
use gizmo_physics_core::{BodyHandle, Collider, PhysicsMaterial, Transform};
use gizmo_physics_rigid::{PhysicsWorld, RigidBody, Velocity};

fn drop_box(hz: u32, height: f32, restitution: f32) -> (PhysicsWorld, f32) {
    let mut w = PhysicsWorld::new().with_substep_rate(hz).expect("rate");
    // Both of the same material, so the pair's restitution is `restitution` however it combines.
    let material = PhysicsMaterial { restitution, ..PhysicsMaterial::default() };
    w.add_body(
        BodyHandle::from_id(1),
        RigidBody::new_static(),
        Transform::new(Vec3::new(0.0, -0.5, 0.0)),
        Velocity::default(),
        Collider::box_collider(Vec3::new(20.0, 0.5, 20.0)).with_material(material),
    );
    let mut rb = RigidBody::new(50.0, true);
    let col = Collider::box_collider(Vec3::splat(0.25)).with_material(material);
    rb.update_inertia_from_collider(&col);
    w.add_body(BodyHandle::from_id(2), rb, Transform::new(Vec3::new(0.0, 0.25 + height, 0.0)), Velocity::default(), col);
    let mut deepest = 0.0f32;
    for _ in 0..120 {
        w.step(1.0 / 60.0).expect("step");
        deepest = deepest.max(0.25 - w.transforms[1].position.y);
    }
    (w, deepest)
}

/// Falling 2 m, 6.3 m/s at the ground — 10 cm a substep at 60 Hz — the box sinks no more than the
/// solver lets a resting contact sink, and rests on the surface, at every rate.
#[test]
fn a_falling_box_lands_on_the_ground_at_every_rate() {
    for hz in [60u32, 120, 240] {
        let (w, deepest) = drop_box(hz, 2.0, 0.0);
        assert!(deepest < 0.005, "{hz} Hz: the box sank {deepest} m into the ground");
        let y = w.transforms[1].position.y;
        assert!((y - 0.25).abs() < 0.005, "{hz} Hz: the box rests at {y}");
    }
}

/// The same box with a restitution of 0.8 bounces back up to most of 0.8² of its height: the bounce
/// is added once the box has reached the ground, not taken out by the gap it still had.
#[test]
fn a_box_landing_on_a_speculative_contact_bounces_in_full() {
    for hz in [60u32, 240] {
        let mut w = drop_box(hz, 2.0, 0.8).0;
        // Back to the start: watch the first bounce's apex.
        w.transforms[1].position = Vec3::new(0.0, 2.25, 0.0);
        w.velocities[1] = Velocity::default();
        let (mut landed, mut apex) = (false, 0.0f32);
        for _ in 0..120 {
            w.step(1.0 / 60.0).expect("step");
            let y = w.transforms[1].position.y;
            landed |= y < 0.3;
            if landed {
                apex = apex.max(y - 0.25);
            }
        }
        assert!(apex > 0.8 * 0.8 * 2.0 * 0.85, "{hz} Hz: the box bounced back to {apex} m of 2");
    }
}

/// A 0.1 m ball dropped from `height` onto a ramp tilted `tilt_deg`, at `hz`: its speed along the
/// ramp just before and just after the first landing, and how fast its underside slips after it.
fn landing_on_a_ramp(tilt_deg: f32, height: f32, hz: u32) -> (f32, f32, f32) {
    let mut w = PhysicsWorld::new().with_substep_rate(hz).expect("rate");
    let tilt = Quat::from_rotation_z(tilt_deg.to_radians());
    let mut ramp = Transform::new(Vec3::new(0.0, -0.5, 0.0));
    ramp.rotation = tilt;
    w.add_body(BodyHandle::from_id(1), RigidBody::new_static(), ramp, Velocity::default(), Collider::box_collider(Vec3::new(10.0, 0.5, 10.0)));
    let ball = Collider::sphere(0.1);
    let mut rb = RigidBody::new(1.0, true);
    rb.update_inertia_from_collider(&ball);
    w.add_body(BodyHandle::from_id(2), rb, Transform::new(Vec3::new(0.0, height, 0.2)), Velocity::default(), ball);
    let (up, along) = (tilt * Vec3::Y, tilt * Vec3::X);
    let mut before = Vec3::ZERO;
    for _ in 0..600 {
        w.step(1.0 / 60.0).expect("step");
        let v = w.velocities[1];
        if v.linear.dot(up) > 0.5 && before.dot(up) < -0.5 {
            let slip = (v.linear + v.angular.cross(-up * 0.1)).dot(along);
            return (before.dot(along), v.linear.dot(along), slip);
        }
        before = v.linear;
    }
    panic!("{tilt_deg}° from {height} m at {hz} Hz: never left the ramp");
}

/// A ball landing on a ramp leaves it rolling: the friction of the impact (μ 0.5, enough here)
/// takes the slip at its underside out, which leaves a solid ball 5/7 of its speed along the ramp.
///
/// The contact the substep's motion closes was taken where the substep's end has the pair, and
/// deep there its point says nothing about where the two meet: dropped 9.9 m onto 30° at 60 Hz the
/// ball's centre ended 7.5 cm inside the ramp, the point was put on the ramp's face, and brought
/// back with the ball it sat above the ball's centre — the landing spun the ball the wrong way
/// (−53 rad/s; slipping at 9.5 m/s). Dropped 3 m onto 10° it left at 0.54 m/s of the 0.91 rolling
/// gives, and 30 m onto 30° at 240 Hz at 6.1 of 8.5 (2026-11-25).
#[test]
fn a_ball_landing_on_a_ramp_leaves_it_rolling() {
    for hz in [60u32, 240] {
        for (tilt, height) in [(5.0, 10.0), (10.0, 3.0), (30.0, 3.0), (30.0, 9.9), (45.0, 10.0)] {
            let (before, after, slip) = landing_on_a_ramp(tilt, height, hz);
            let rolling = before * 5.0 / 7.0;
            assert!(
                (after - rolling).abs() <= 0.06 * rolling.abs() && slip.abs() <= 0.06 * before.abs(),
                "{hz} Hz, {tilt}° from {height} m: along the ramp {before:.2} -> {after:.2} m/s (rolling: {rolling:.2}), slipping at {slip:.2}"
            );
        }
    }
    // At 240 Hz a 30 m drop is not a fast pair, and lands the same way.
    let (before, after, _) = landing_on_a_ramp(30.0, 30.0, 240);
    assert!((after - before * 5.0 / 7.0).abs() <= 0.06 * (before * 5.0 / 7.0).abs(), "240 Hz, 30° from 30 m: {before:.2} -> {after:.2} m/s");
}
