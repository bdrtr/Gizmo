//! A pendulum on a joint swings at its period and keeps its amplitude, whatever its bob.
//!
//! A joint's anchor is pinned by a point constraint. Until 2026-11-23 that was three rows along x,
//! y and z, each solved alone, and through the rotation the anchor's lever gives a body they are
//! coupled — by `m·L²/I`, 18 000 for a 4 mm bob 0.34 m from its pivot. The rows then did not
//! converge: that pendulum swung 33 % slower than its period and lost half its swing in thirty
//! seconds, and on a 1 m string 324 % slower. The point is now one 3×3 block
//! (`JointSolver::apply_point_constraint`).

use gizmo_math::{Quat, Vec3};
use gizmo_physics_core::{BodyHandle, Collider, Transform};
use gizmo_physics_rigid::joints::Joint;
use gizmo_physics_rigid::{PhysicsWorld, RigidBody, Velocity};

const G: f32 = 9.81;
const THETA0: f32 = 0.3;

/// A bob of radius `r` on a `length` joint from a static pivot at (0, 1, 0), released at `THETA0`,
/// undamped, for thirty seconds at the world's own substep. Returns its measured period over the
/// analytic one, less 1, and its last half-swing's amplitude.
fn swing(length: f32, r: f32, joint: fn(BodyHandle, BodyHandle, Vec3, Vec3) -> Joint) -> (f32, f32) {
    let pivot = Vec3::new(0.0, 1.0, 0.0);
    let mut w = PhysicsWorld::new().with_gravity(Vec3::new(0.0, -G, 0.0));
    let mut beam = RigidBody::new_static();
    beam.wake_up();
    w.add_body(BodyHandle::from_id(0), beam, Transform::new(pivot), Velocity::default(), Collider::box_collider(Vec3::splat(0.02)));
    let rot = Quat::from_rotation_x(-THETA0);
    let col = Collider::sphere(r);
    let mut rb = RigidBody::new(0.3, true);
    rb.linear_damping = 0.0;
    rb.angular_damping = 0.0;
    rb.update_inertia_from_collider(&col);
    w.add_body(BodyHandle::from_id(1), rb, Transform::new(pivot - rot * Vec3::Y * length).with_rotation(rot), Velocity::default(), col);
    w.joints.push(joint(BodyHandle::from_id(1), BodyHandle::from_id(0), Vec3::Y * length, Vec3::ZERO));
    let dt = 1.0 / 240.0;
    let (mut last, mut upward, mut t, mut peak, mut amplitude) = (THETA0, Vec::new(), 0.0f32, 0.0f32, 0.0f32);
    for _ in 0..240 * 30 {
        w.step(dt).expect("a step");
        t += dt;
        let d = w.transforms[1].position - pivot;
        let a = d.z.atan2(-d.y);
        peak = peak.max(a.abs());
        if (last < 0.0) != (a < 0.0) {
            if last < 0.0 {
                upward.push(t - dt + last / (last - a) * dt);
            }
            amplitude = peak;
            peak = 0.0;
        }
        last = a;
    }
    let measured = (upward[upward.len() - 1] - upward[0]) / (upward.len() - 1) as f32;
    // A compound pendulum (the bob's own inertia, 0.4·m·r², about its centre) at large amplitude.
    let t0 = std::f32::consts::TAU * ((length * length + 0.4 * r * r) / (G * length)).sqrt();
    let expected = t0 * (1.0 + THETA0 * THETA0 / 16.0 + 11.0 * THETA0.powi(4) / 3072.0);
    (measured / expected - 1.0, amplitude)
}

fn hinge(a: BodyHandle, b: BodyHandle, la: Vec3, lb: Vec3) -> Joint {
    Joint::hinge(a, b, la, lb, Vec3::X)
}

#[test]
fn a_pendulum_swings_at_its_period_whatever_its_bob() {
    for (name, joint) in [("ball socket", Joint::ball_socket as fn(_, _, _, _) -> Joint), ("hinge", hinge)] {
        for (length, r) in [(0.344, 0.04), (0.344, 0.004), (1.0, 0.004)] {
            let (period, amplitude) = swing(length, r, joint);
            assert!(period.abs() < 5e-4, "{name}, a {r} m bob on {length} m: period {:+.3} % off its analytic value", period * 100.0);
            assert!((amplitude / THETA0 - 1.0).abs() < 2e-3, "{name}, a {r} m bob on {length} m: amplitude {amplitude} after 30 s, released at {THETA0}");
        }
    }
}
