//! A body turning freely keeps its angular momentum and its energy, and its angular velocity
//! changes as Euler's equations say: `I·dω/dt = I·ω × ω` in its own frame.
//!
//! The integrator kept `ω` instead, with no gyroscopic term: a box spun about its intermediate axis,
//! 1 % off it, turned on unchanged for ever, where it should flip over every second or two (the
//! Dzhanibekov effect: here at 2.5 s and back at 6.4 s), its momentum's direction wobbling as it turned (2026-11-26). Now `ω` is
//! stepped by the implicit midpoint rule in the body's frame (`integrator::gyroscopic_change`),
//! which keeps the energy and `|L|` exactly.

use gizmo_math::{Mat3, Vec3};
use gizmo_physics_core::{BodyHandle, Collider, Transform};
use gizmo_physics_rigid::{PhysicsWorld, RigidBody, Velocity};

const HALF: Vec3 = Vec3::new(1.0, 0.5, 0.2);

fn spun(hz: u32, omega: Vec3, shape: Collider) -> PhysicsWorld {
    let mut w = PhysicsWorld::new().with_gravity(Vec3::ZERO).with_substep_rate(hz).unwrap();
    let mut rb = RigidBody::new(1.0, true);
    rb.update_inertia_from_collider(&shape);
    rb.angular_damping = 0.0;
    rb.linear_damping = 0.0;
    let mut v = Velocity::new(Vec3::ZERO);
    v.angular = omega;
    w.add_body(BodyHandle::from_id(0), rb, Transform::new(Vec3::ZERO), v, shape);
    w
}

fn momentum(w: &PhysicsWorld) -> Vec3 {
    let inv: Mat3 = w.rigid_bodies[0].inv_world_inertia_tensor(w.transforms[0].rotation);
    inv.inverse() * w.velocities[0].angular
}

fn energy(w: &PhysicsWorld) -> f32 {
    0.5 * w.velocities[0].angular.dot(momentum(w))
}

/// The angular velocity in the body's frame.
fn body_omega(w: &PhysicsWorld) -> Vec3 {
    w.transforms[0].rotation.inverse() * w.velocities[0].angular
}

/// Euler's equations in the body's frame, by fourth-order Runge–Kutta at a tenth of a millisecond:
/// the reference. The times `ω_y` changes sign, i.e. the body flips about its intermediate axis.
fn reference_flips(inertia: Vec3, omega: Vec3, seconds: f32) -> Vec<f32> {
    let f = |w: Vec3| (inertia * w).cross(w) / inertia;
    let dt = 1e-4f32;
    let (mut w, mut t, mut flips) = (omega, 0.0f32, Vec::new());
    while t < seconds {
        let k1 = f(w);
        let k2 = f(w + k1 * (dt / 2.0));
        let k3 = f(w + k2 * (dt / 2.0));
        let k4 = f(w + k3 * dt);
        let next = w + (k1 + 2.0 * k2 + 2.0 * k3 + k4) * (dt / 6.0);
        if next.y.signum() != w.y.signum() {
            flips.push(t);
        }
        (w, t) = (next, t + dt);
    }
    flips
}

/// A box spun 1 % off its intermediate axis flips over, when Euler's equations say, keeping its
/// energy and its momentum.
#[test]
fn a_box_spun_about_its_intermediate_axis_flips_as_eulers_equations_say() {
    let omega = Vec3::new(0.05, 5.0, 0.05);
    let shape = Collider::box_collider(HALF);
    let inertia = {
        let mut rb = RigidBody::new(1.0, true);
        rb.update_inertia_from_collider(&shape);
        rb.local_inertia
    };
    let expected = reference_flips(inertia, omega, 10.0);
    assert!(expected.len() >= 2, "premise: it flips over and back in ten seconds: {expected:?}");
    for (hz, timing) in [(60u32, 0.06f32), (240, 0.03)] {
        let mut w = spun(hz, omega, shape.clone());
        let (l0, e0) = (momentum(&w), energy(&w));
        let (mut flips, mut was) = (Vec::new(), body_omega(&w).y);
        let (mut worst_l, mut worst_e) = (0.0f32, 0.0f32);
        let dt = 1.0 / hz as f32;
        for step in 0..(10 * hz) {
            w.step(dt).unwrap();
            let now = body_omega(&w).y;
            if now.signum() != was.signum() {
                flips.push(step as f32 * dt);
            }
            was = now;
            worst_l = worst_l.max((momentum(&w).length() - l0.length()).abs() / l0.length());
            worst_e = worst_e.max((energy(&w) - e0).abs() / e0);
        }
        assert!(worst_e < 1e-3, "{hz} Hz: the energy moved {:.3} %", worst_e * 100.0);
        assert!(worst_l < 1e-3, "{hz} Hz: |L| moved {:.3} %", worst_l * 100.0);
        assert_eq!(flips.len(), expected.len(), "{hz} Hz: flipped at {flips:?}, Euler's equations at {expected:?}");
        for (k, (got, want)) in flips.iter().zip(&expected).enumerate() {
            assert!((got - want).abs() <= timing * want.max(1.0), "{hz} Hz: flip {k} at {got:.3} s, Euler's equations at {want:.3} s");
        }
    }
}

/// A body turning about a principal axis, and one with the same inertia about every axis, turns
/// on exactly as before: nothing in `ω × I·ω` to step.
#[test]
fn a_cube_and_a_principal_spin_turn_on_unchanged() {
    for (shape, omega) in [
        (Collider::box_collider(Vec3::splat(0.5)), Vec3::new(1.0, 2.0, 3.0)),
        (Collider::sphere(0.5), Vec3::new(-4.0, 0.5, 2.0)),
        (Collider::box_collider(HALF), Vec3::new(0.0, 0.0, 7.0)),
    ] {
        let mut w = spun(60, omega, shape);
        for _ in 0..120 {
            w.step(1.0 / 60.0).unwrap();
        }
        assert_eq!(w.velocities[0].angular, omega);
    }
}
