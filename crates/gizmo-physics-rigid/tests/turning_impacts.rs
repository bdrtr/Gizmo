//! A fast body that does not meet what it strikes through its centre lands as a rigid impact does:
//! on the points where it meets, turned by them, and then held where its turn takes it.
//!
//! A pair closing faster than its size allows took one speculative contact at the moving body's
//! centre, which stopped the centre and turned nothing. A box dropped at 60 m/s on its corner came
//! back straight up at 60 Hz, without its spin and its slide; a capsule did at every rate where it
//! was fast. And what a fast impact did set turning was looked at only where the substep ended: a
//! box turned 1.3 rad in a substep and ended 63 cm in the ground, at 200 m/s it went through
//! (2026-11-26). Now the body is held where it meets (`PhysicsWorld::hold_set_going`), the impact
//! is solved in the next substep on its real points, and a body that turns is followed along its
//! turn — by the narrow phase (`turning_contacts`) and by the hold (`held_turning`).

use gizmo_math::{Quat, Vec3};
use gizmo_physics_core::components::{CombineMode, PhysicsMaterial};
use gizmo_physics_core::{BodyHandle, Collider, Transform};
use gizmo_physics_rigid::{PhysicsWorld, RigidBody, Velocity};

fn material(restitution: f32) -> PhysicsMaterial {
    PhysicsMaterial { restitution, static_friction: 0.5, dynamic_friction: 0.5, restitution_combine: CombineMode::Max, ..Default::default() }
}

/// A 1 kg body of `shape` turned `tilt` about z, 3 m over a floor, falling at `speed` with a third
/// of it sideways, in a world without gravity at `hz`. The floor is a box whose top is at 0.
fn dropped(hz: u32, shape: Collider, tilt: f32, speed: f32, restitution: f32) -> PhysicsWorld {
    let mut w = PhysicsWorld::new().with_gravity(Vec3::ZERO).with_substep_rate(hz).unwrap();
    let c = shape.with_material(material(restitution));
    let mut rb = RigidBody::new(1.0, false);
    rb.update_inertia_from_collider(&c);
    let mut t = Transform::new(Vec3::new(0.0, 3.0, 0.0));
    t.rotation = Quat::from_rotation_z(tilt);
    w.add_body(BodyHandle::from_id(0), rb, t, Velocity::new(Vec3::new(speed * 0.3, -speed, 0.0)), c);
    let floor = Collider::box_collider(Vec3::new(50.0, 0.5, 50.0)).with_material(material(restitution));
    w.add_body(BodyHandle::from_id(1), RigidBody::new_static(), Transform::new(Vec3::new(0.0, -0.5, 0.0)), Velocity::default(), floor);
    w
}

/// The body's velocity and spin once the first impact has taken a share of its fall: stepped at its
/// own rate until it does.
fn after_first_impact(w: &mut PhysicsWorld, hz: u32, speed: f32) -> (Vec3, Vec3) {
    for _ in 0..hz {
        w.step(1.0 / hz as f32).unwrap();
        let v = w.velocities[0];
        if v.linear.y > -0.99 * speed {
            return (v.linear, v.angular);
        }
    }
    panic!("never landed");
}

/// The box 1 m × 0.4 m (× 0.6) turned 0.4 rad lands on one corner. With no bounce and friction that
/// holds (it does, at μ 0.5), the rigid impact stops that corner dead: the impulse `J = K⁻¹·(−v)`,
/// `K` the corner's mass matrix in the plane of the turn.
#[test]
fn a_box_struck_on_its_corner_turns_as_a_rigid_impact_does() {
    let (half, tilt) = (Vec3::new(0.5, 0.2, 0.3), 0.4f32);
    let inertia = (4.0 * half.x * half.x + 4.0 * half.y * half.y) / 12.0;
    let corner = Quat::from_rotation_z(tilt) * Vec3::new(-half.x, -half.y, 0.0);
    let (rx, ry) = (corner.x, corner.y);
    let k = [[1.0 + ry * ry / inertia, -rx * ry / inertia], [-rx * ry / inertia, 1.0 + rx * rx / inertia]];
    let det = k[0][0] * k[1][1] - k[0][1] * k[1][0];
    // A fast pair at each of these: the corner meets the floor where it is held.
    for (hz, speed) in [(60, 60.0f32), (60, 200.0), (240, 200.0)] {
        let v = Vec3::new(speed * 0.3, -speed, 0.0);
        let j = Vec3::new(k[1][1] * -v.x - k[0][1] * -v.y, -k[1][0] * -v.x + k[0][0] * -v.y, 0.0) / det;
        assert!(j.x.abs() <= 0.5 * j.y, "premise: the corner sticks");
        let expected = (v + j, (rx * j.y - ry * j.x) / inertia);
        let mut w = dropped(hz, Collider::box_collider(half), tilt, speed, 0.0);
        let (linear, angular) = after_first_impact(&mut w, hz, speed);
        let at = format!("{hz} Hz, {speed} m/s");
        assert!((linear - expected.0).length() < 0.01 * speed, "{at}: leaves the floor at {linear:?}, a rigid impact at {:?}", expected.0);
        assert!((angular.z - expected.1).abs() < 0.01 * expected.1.abs(), "{at}: turns {angular:?}, a rigid impact {}", expected.1);
    }
}

/// A capsule landing end first bounces off with the spin its end gives it, alike at every rate —
/// the rate's share is only in which substep it lands.
#[test]
fn a_capsule_landing_end_first_bounces_alike_at_every_rate() {
    for speed in [20.0f32, 60.0, 200.0] {
        let fine = after_first_impact(&mut dropped(960, Collider::capsule(0.1, 0.4), 0.4, speed, 0.5), 960, speed);
        for hz in [60, 240] {
            let (linear, angular) = after_first_impact(&mut dropped(hz, Collider::capsule(0.1, 0.4), 0.4, speed, 0.5), hz, speed);
            assert!((linear - fine.0).length() < 0.02 * speed, "{hz} Hz, {speed} m/s: bounces at {linear:?}, at 960 Hz {:?}", fine.0);
            assert!((angular - fine.1).length() < 0.02 * fine.1.length(), "{hz} Hz, {speed} m/s: turns {angular:?}, at 960 Hz {:?}", fine.1);
        }
    }
}

/// Every drop of the box and the capsule above, at every rate and speed: nothing ends in the floor
/// past a few centimetres, or goes through. Held where it met with its speed, and the impact solved
/// there, a box turns fast: followed only along its way, not its turn, it turned 1.3 rad into the
/// ground in the next substep at 60 m/s and 60 Hz, its other corner 63 cm deep, and at 200 m/s it
/// went through (2026-11-26, before `held_turning`).
#[test]
fn a_turning_impact_does_not_end_in_the_floor() {
    for (what, shape, rest) in [("box", Collider::box_collider(Vec3::new(0.5, 0.2, 0.3)), 0.2f32), ("capsule", Collider::capsule(0.1, 0.4), 0.1)] {
        for restitution in [0.0, 0.5] {
            for speed in [20.0f32, 60.0, 200.0] {
                for hz in [60, 240] {
                    let mut w = dropped(hz, shape.clone(), 0.4, speed, restitution);
                    for _ in 0..hz {
                        w.step(1.0 / hz as f32).unwrap();
                        let y = w.transforms[0].position.y;
                        assert!(y > rest - 0.1, "{what}, {hz} Hz, {speed} m/s, e {restitution}: its centre is at {y}, resting at {rest}");
                    }
                }
            }
        }
    }
}

/// Boxes and capsules shot level at a 3 m hill of a mesh and of a height field, turned every way
/// and spinning: each is turned by the hill or passes over it, and none ends up inside. Balls do not
/// (`struck.rs`); boxes and capsules still could, spinning.
#[test]
fn a_spinning_box_or_capsule_does_not_go_into_a_hill() {
    let hill = |x: f32| 3.0 * (-(x * x) / 8.0).exp();
    let n = 80u32;
    let at = |k: u32| (k as f32 / n as f32 - 0.5) * 40.0;
    let mesh = {
        let (mut v, mut idx, w) = (Vec::new(), Vec::new(), n + 1);
        for z in 0..=n {
            for x in 0..=n {
                v.push(Vec3::new(at(x), hill(at(x)), at(z)));
            }
        }
        for z in 0..n {
            for x in 0..n {
                let a = z * w + x;
                idx.extend([a, a + w, a + 1, a + 1, a + w, a + w + 1]);
            }
        }
        Collider::trimesh(v, idx)
    };
    let field = Collider::heightfield((0..=n).flat_map(|_| (0..=n).map(|x| hill(at(x)))).collect(), n + 1, n + 1, Vec3::new(0.5, 1.0, 0.5));
    let mut seed = 5u32;
    let mut u = || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (seed >> 8) as f32 / (1u32 << 24) as f32
    };
    let mut inside = Vec::new();
    for (what, ground) in [("mesh", mesh), ("field", field)] {
        for hz in [60, 240] {
            for _ in 0..100 {
                let (speed, size) = (20.0 + 600.0 * u() * u(), 0.05 + 0.25 * u());
                let shape = if u() < 0.5 { Collider::box_collider(Vec3::new(size, size * 0.6, size * 0.4)) } else { Collider::capsule(size * 0.4, size) };
                let turn = Quat::from_euler(gizmo_math::EulerRot::XYZ, 6.3 * u(), 6.3 * u(), 6.3 * u());
                let spin = Vec3::new(u() - 0.5, u() - 0.5, u() - 0.5) * 100.0;
                let from = Vec3::new(-9.0 + u(), size + 0.05 + 2.0 * u(), u());
                let mut w = PhysicsWorld::new().with_substep_rate(hz).unwrap();
                w.add_body(BodyHandle::from_id(1), RigidBody::new_static(), Transform::new(Vec3::ZERO), Velocity::default(), ground.clone());
                let mut rb = RigidBody::new(1.0, true);
                rb.update_inertia_from_collider(&shape);
                let mut t = Transform::new(from);
                t.rotation = turn;
                let mut v = Velocity::new(Vec3::X * speed);
                v.angular = spin;
                w.add_body(BodyHandle::from_id(2), rb, t, v, shape);
                for _ in 0..40 {
                    w.step(1.0 / 60.0).unwrap();
                    let p = w.transforms[1].position;
                    if p.x.abs() < 19.0 && p.z.abs() < 19.0 && p.y < hill(p.x) - 0.02 {
                        inside.push(format!("{what}, {hz} Hz: {size:.2} m at {speed:.0} m/s from {from:?}, {:.2} m under at {p:?}", hill(p.x) - p.y));
                        break;
                    }
                }
            }
        }
    }
    assert!(inside.is_empty(), "{} of 400 ended inside the hill, e.g. {:?}", inside.len(), &inside[..inside.len().min(3)]);
}
