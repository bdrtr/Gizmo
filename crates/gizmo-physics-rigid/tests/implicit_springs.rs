//! Springs, suspensions, torsional springs and drives are solved implicitly
//! (`JointSolver::implicit_spring_impulse`), so they hold at any substep rate.
//!
//! They were explicit, `(k·x + c·v)·dt` once a substep, which diverges once
//! `(ω·dt)² + 2·c·k_inv·dt > 4`. Every spring in the engine's tests passed that at the world's 240 Hz;
//! the ones here do not at 60 — each was measured to blow up with the explicit form there — and settle
//! with the implicit one at every rate.

use gizmo_math::{Quat, Vec3};
use gizmo_physics_core::{BodyHandle, Collider, Transform};
use gizmo_physics_rigid::{D6Motion, Joint, JointData, PhysicsWorld, RigidBody, Velocity};

const RATES: [u32; 3] = [60, 120, 240];

fn world(hz: u32) -> PhysicsWorld {
    PhysicsWorld::new().with_gravity(Vec3::ZERO).with_substep_rate(hz).expect("rate")
}

fn anchor(world: &mut PhysicsWorld, id: u32, pos: Vec3) {
    world.add_body(BodyHandle::from_id(id), RigidBody::new_static(), Transform::new(pos), Velocity::default(), Collider::sphere(0.2));
}

fn dyn_box(world: &mut PhysicsWorld, id: u32, pos: Vec3, half: f32) {
    let mut rb = RigidBody::new(1.0, false);
    rb.linear_damping = 0.0;
    rb.angular_damping = 0.0;
    let col = Collider::box_collider(Vec3::splat(half));
    rb.update_inertia_from_collider(&col);
    world.add_body(BodyHandle::from_id(id), rb, Transform::new(pos), Velocity::default(), col);
}

/// A 20 000 N/m spring on a 1 kg box hung from a fixed point, pulled 0.1 m past its rest length and
/// let go: ω = 141 rad/s, `ω·dt` 2.4 at 60 Hz, past the explicit form's limit of 2. It must neither
/// grow nor leave its rest length by more than it started.
#[test]
fn a_stiff_spring_stays_bounded_at_every_rate() {
    for hz in RATES {
        let mut w = world(hz);
        anchor(&mut w, 1, Vec3::ZERO);
        dyn_box(&mut w, 2, Vec3::new(1.1, 0.0, 0.0), 0.1);
        w.joints.push(Joint::spring(BodyHandle::from_id(1), BodyHandle::from_id(2), Vec3::ZERO, Vec3::ZERO, 1.0, 20_000.0, 1.0));
        let mut worst = 0.0f32;
        for _ in 0..240 {
            w.step(1.0 / 60.0).expect("step");
            let x = w.transforms[1].position.length() - 1.0;
            assert!(x.is_finite(), "{hz} Hz: the spring blew up");
            worst = worst.max(x.abs());
        }
        assert!(worst <= 0.1001, "{hz} Hz: the spring stretched to {worst} m from a 0.1 m start");
        assert!(w.velocities[1].linear.length() < 5.0, "{hz} Hz: still at {} m/s", w.velocities[1].linear.length());
    }
}

/// `joints_behavior`'s hinge torsional spring (k 30, c 8 on a 0.6 m, 1 kg box): `c·dt/I` is 0.56 at
/// 240 Hz and 2.22 at 60 — over 2, so the explicit damping alone flipped the box's spin every
/// substep and grew it. It settles at its rest angle at every rate.
#[test]
fn a_damped_torsional_spring_settles_at_every_rate() {
    for hz in RATES {
        let mut w = world(hz);
        anchor(&mut w, 1, Vec3::ZERO);
        dyn_box(&mut w, 2, Vec3::new(0.5, 0.0, 0.0), 0.3);
        let mut j = Joint::hinge(BodyHandle::from_id(1), BodyHandle::from_id(2), Vec3::new(0.5, 0.0, 0.0), Vec3::ZERO, Vec3::Z);
        if let JointData::Hinge(ref mut d) = j.data {
            d.use_torsional_spring = true;
            d.torsional_stiffness = 30.0;
            d.torsional_damping = 8.0;
            d.rest_angle = 0.8;
        }
        w.joints.push(j);
        for _ in 0..200 {
            w.step(1.0 / 60.0).expect("step");
        }
        let JointData::Hinge(d) = w.joints[0].data else { unreachable!() };
        assert!((d.current_angle - 0.8).abs() < 0.05, "{hz} Hz: settled at {} rad, not 0.8", d.current_angle);
        assert!(w.velocities[1].angular.length() < 0.05, "{hz} Hz: still spinning at {}", w.velocities[1].angular.length());
    }
}

/// A stiff angular drive with no force limit (the default): it holds its target at every rate.
#[test]
fn a_stiff_angular_drive_holds_its_target_at_every_rate() {
    for hz in RATES {
        let mut w = world(hz);
        anchor(&mut w, 1, Vec3::ZERO);
        dyn_box(&mut w, 2, Vec3::ZERO, 0.3);
        let mut j = Joint::d6(BodyHandle::from_id(1), BodyHandle::from_id(2), Vec3::ZERO, Vec3::ZERO);
        if let JointData::D6(ref mut d) = j.data {
            // Free about Z, so the drive is all that holds it (a locked axis would fight the drive).
            d.angular[2] = D6Motion::Free;
            d.angular_drives[2].enabled = true;
            d.angular_drives[2].stiffness = 400.0;
            d.angular_drives[2].damping = 30.0;
            d.angular_drives[2].target_position = 0.6;
        }
        w.joints.push(j);
        for _ in 0..240 {
            w.step(1.0 / 60.0).expect("step");
            assert!(w.velocities[1].angular.is_finite(), "{hz} Hz: the drive blew up");
        }
        let angle = 2.0 * (w.transforms[1].rotation * Quat::IDENTITY).z.asin();
        assert!((angle - 0.6).abs() < 0.05, "{hz} Hz: the drive holds {angle} rad, not 0.6");
    }
}
