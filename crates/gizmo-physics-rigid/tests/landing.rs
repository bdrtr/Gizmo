//! A body landing on the ground meets it, at any substep rate, instead of sinking a substep's travel
//! into it and being pushed back out: the narrow phase finds the contacts the substep's motion
//! closes before they touch (`pipeline::near_contacts`), and the solve lets each close by exactly
//! its gap.
//!
//! Measured 2026-11-24, before: a 50 kg box falling 2 m sank 6 cm at 60 Hz; a box dropped from 5 m
//! came to rest 1.35 mm into the ground at 240 Hz (`golden_box_settling_on_the_ground`).

use gizmo_math::Vec3;
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
