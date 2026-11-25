//! A body an impact sets going is not carried through what it was not known to be heading for.
//!
//! The narrow phase looks where bodies are going, and a body standing still is going nowhere:
//! struck, the solve gives it the striker's speed and it travels on it in the same substep, past
//! neighbours it has no contact with. At 60 Hz a ball struck at 160 m/s went through a 4 cm wall it
//! stood in front of; a row of touching balls struck at 320 ended with two in one place; and a ball
//! sliding on a hill ran out of one face of it into the next (2026-11-25). Now it is looked at
//! again after the solve and held at what it reached (`PhysicsWorld::hold_set_going`).

use gizmo_math::{Quat, Vec3};
use gizmo_physics_core::components::{CombineMode, PhysicsMaterial};
use gizmo_physics_core::{BodyHandle, Collider, CollisionLayer, Transform};
use gizmo_physics_rigid::joints::Joint;
use gizmo_physics_rigid::{PhysicsWorld, RigidBody, Velocity};

fn elastic() -> PhysicsMaterial {
    PhysicsMaterial { restitution: 1.0, static_friction: 0.0, dynamic_friction: 0.0, restitution_combine: CombineMode::Max, ..Default::default() }
}

fn world(hz: u32) -> PhysicsWorld {
    PhysicsWorld::new().with_gravity(Vec3::ZERO).with_substep_rate(hz).unwrap()
}

fn ball(w: &mut PhysicsWorld, id: u32, at: Vec3, v: Vec3, r: f32) {
    let c = Collider::sphere(r).with_material(elastic());
    let mut rb = RigidBody::new(1.0, false);
    rb.update_inertia_from_collider(&c);
    w.add_body(BodyHandle::from_id(id), rb, Transform::new(at), Velocity::new(v), c);
}

fn fixed(w: &mut PhysicsWorld, id: u32, t: Transform, c: Collider) {
    w.add_body(BodyHandle::from_id(id), RigidBody::new_static(), t, Velocity::default(), c.with_material(elastic()));
}

/// A 0.5 m ball struck at `speed` by another, in front of `wall` `behind` it and turned by `turn`:
/// whether either ball got to the far side of it.
fn struck_through(hz: u32, speed: f32, behind: f32, turn: f32, wall: &dyn Fn() -> Collider) -> bool {
    let mut w = world(hz);
    ball(&mut w, 0, Vec3::X * -2.02, Vec3::X * speed, 0.5);
    ball(&mut w, 1, Vec3::ZERO, Vec3::ZERO, 0.5);
    let mut t = Transform::new(Vec3::X * (0.52 + behind) / turn.cos());
    t.rotation = Quat::from_rotation_z(turn);
    let (at, normal) = (t.position, t.rotation * Vec3::X);
    fixed(&mut w, 2, t, wall());
    for _ in 0..60 {
        w.step(1.0 / 60.0).unwrap();
        if (0..2).any(|i| (w.transforms[i].position - at).dot(normal) > 0.0 && w.transforms[i].position.length() < 15.0) {
            return true;
        }
    }
    false
}

#[test]
fn a_struck_ball_does_not_go_through_the_wall_behind_it() {
    let boxed = || Collider::box_collider(Vec3::new(0.02, 20.0, 20.0));
    let mesh = || {
        let v = [(-20.0, -20.0), (20.0, -20.0), (20.0, 20.0), (-20.0, 20.0)].map(|(y, z)| Vec3::new(0.0, y, z)).to_vec();
        Collider::trimesh(v, vec![0, 1, 2, 0, 2, 3])
    };
    for (what, wall) in [("box", &boxed as &dyn Fn() -> Collider), ("mesh", &mesh)] {
        for hz in [60, 120, 240] {
            for speed in [20.0, 80.0, 160.0, 320.0] {
                for behind in [0.02, 0.3, 1.0] {
                    for turn in [0.0, 0.5] {
                        assert!(!struck_through(hz, speed, behind, turn, wall), "{hz} Hz, {speed} m/s, a {what} wall {behind} m behind, turned {turn}");
                    }
                }
            }
        }
    }
}

/// A row of touching balls asleep: the pairs of it are not even looked at by the narrow phase.
#[test]
fn a_sleeping_row_keeps_its_order_when_struck() {
    for hz in [60, 240] {
        for speed in [80.0, 320.0] {
            let mut w = world(hz);
            for i in 1..5u32 {
                ball(&mut w, i, Vec3::X * (i as f32 - 1.0), Vec3::ZERO, 0.5);
            }
            for _ in 0..240 {
                w.step(1.0 / 60.0).unwrap();
            }
            assert!(w.rigid_bodies.iter().all(|b| b.is_sleeping), "premise: the row is awake");
            ball(&mut w, 0, Vec3::X * -4.0, Vec3::X * speed, 0.5);
            for _ in 0..120 {
                w.step(1.0 / 60.0).unwrap();
                let row: Vec<(u32, f32)> =
                    (0..5).map(|id| (id, w.transforms[*w.entity_index_map.get(&id).unwrap()].position.x)).collect();
                for pair in row.windows(2) {
                    assert!(pair[1].1 - pair[0].1 > 0.3, "{hz} Hz, {speed} m/s: {row:?}");
                }
            }
        }
    }
}

/// Held, a body keeps the speed it was given, and passes it on: nothing is lost in the row but what
/// damping takes, and nothing made.
#[test]
fn a_struck_row_keeps_its_momentum() {
    for hz in [60, 240] {
        for speed in [80.0, 320.0] {
            let mut w = world(hz);
            ball(&mut w, 0, Vec3::X * -2.02, Vec3::X * speed, 0.5);
            for i in 1..5u32 {
                ball(&mut w, i, Vec3::X * (i as f32 - 1.0) * 1.01, Vec3::ZERO, 0.5);
            }
            for _ in 0..30 {
                w.step(1.0 / 60.0).unwrap();
            }
            let momentum: f32 = w.velocities.iter().map(|v| v.linear.x).sum();
            let energy: f32 = w.velocities.iter().map(|v| v.linear.length_squared()).sum::<f32>() / (speed * speed);
            assert!((momentum / speed - 1.0).abs() < 0.01, "{hz} Hz, {speed} m/s: momentum {momentum}");
            assert!(energy <= 1.001, "{hz} Hz, {speed} m/s: energy {energy} times what it was");
        }
    }
}

/// What a body does not collide with does not hold it: another layer, a trigger, a body it is
/// jointed to with collision off.
#[test]
fn what_a_struck_ball_does_not_collide_with_does_not_hold_it() {
    let run = |what: &str| {
        let mut w = world(60);
        ball(&mut w, 0, Vec3::X * -2.02, Vec3::X * 160.0, 0.5);
        ball(&mut w, 1, Vec3::ZERO, Vec3::ZERO, 0.5);
        let mut wall = Collider::box_collider(Vec3::new(0.02, 20.0, 20.0));
        match what {
            "trigger" => wall = wall.with_trigger(true),
            "layer" => wall = wall.with_layer(CollisionLayer { layer: 3, mask: 0 }),
            _ => {}
        }
        fixed(&mut w, 2, Transform::new(Vec3::X * 0.82), wall);
        if what == "joint" {
            let mut joint = Joint::spring(BodyHandle::from_id(1), BodyHandle::from_id(2), Vec3::ZERO, Vec3::ZERO, 0.0, 0.0, 0.0);
            joint.collision_enabled = false;
            w.joints.push(joint);
        }
        for _ in 0..4 {
            w.step(1.0 / 60.0).unwrap();
        }
        w.transforms[1].position.x
    };
    assert!(run("solid") < 0.82, "premise: the wall holds it");
    for what in ["trigger", "layer", "joint"] {
        assert!(run(what) > 2.0, "{what}: held at {}", run(what));
    }
}

/// Level shots at a 3 m hill, as a mesh and as a height field: a ball is turned by it or passes
/// over it, and never ends up inside. Before, 22 % did at 240 Hz and 60 % at 60 in a mesh of it,
/// 9 % and 45 % in a field — of boxes and capsules among them; those still can, spinning.
#[test]
fn a_fast_ball_does_not_go_into_a_hill() {
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
    let mut seed = 3u32;
    let mut u = || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (seed >> 8) as f32 / (1u32 << 24) as f32
    };
    for (what, ground) in [("mesh", mesh), ("field", field)] {
        for hz in [60, 240] {
            for _ in 0..120 {
                let (speed, r) = (20.0 + 600.0 * u() * u(), 0.05 + 0.3 * u());
                let from = Vec3::new(-9.0 + u(), r + 0.05 + 2.0 * u(), u());
                let mut w = PhysicsWorld::new().with_substep_rate(hz).unwrap();
                w.add_body(BodyHandle::from_id(1), RigidBody::new_static(), Transform::new(Vec3::ZERO), Velocity::default(), ground.clone());
                let c = Collider::sphere(r);
                let mut rb = RigidBody::new(1.0, true);
                rb.update_inertia_from_collider(&c);
                w.add_body(BodyHandle::from_id(2), rb, Transform::new(from), Velocity::new(Vec3::X * speed), c);
                for _ in 0..40 {
                    w.step(1.0 / 60.0).unwrap();
                    let p = w.transforms[1].position;
                    assert!(
                        p.x.abs() > 19.0 || p.z.abs() > 19.0 || p.y > hill(p.x) - 0.02,
                        "{what}, {hz} Hz: a {r:.2} m ball at {speed:.0} m/s from {from:?} is {:.2} m under the hill at {p:?}",
                        hill(p.x) - p.y
                    );
                }
            }
        }
    }
}
