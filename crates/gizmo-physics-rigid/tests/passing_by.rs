//! A fast body that passes by something it does not touch is not touched by it.
//!
//! A pair too fast for discrete detection takes a speculative contact, and that was made from where
//! the two were: closing along the axis between them fast enough to meet within the substep. A
//! ball flying past a box closes on its near corner until it is beside it, so it was stopped at the
//! corner and bounced off what it never reached — at 60 Hz a 0.1 m ball at 20 m/s off a 0.6 m crate
//! it was to pass 10 cm clear of; at 240 Hz any faster than 48 m/s (2026-11-25). The contact is now
//! where the paths meet, or none (`Gjk::conservative_advancement`).

use gizmo_math::Vec3;
use gizmo_physics_core::components::{CombineMode, PhysicsMaterial};
use gizmo_physics_core::{BodyHandle, Collider, Transform};
use gizmo_physics_rigid::{PhysicsWorld, RigidBody, Velocity};

/// Elastic, so a contact that should not be shows in the velocity.
fn elastic() -> PhysicsMaterial {
    PhysicsMaterial { restitution: 1.0, static_friction: 0.0, dynamic_friction: 0.0, restitution_combine: CombineMode::Max, ..Default::default() }
}

/// A 0.1 m ball at `v` along +x for a fifth of a second, past `by`: where it is and how it moves.
fn fly(hz: u32, v: Vec3, ccd: bool, by: Option<(Collider, Vec3)>) -> (Vec3, Vec3) {
    let mut w = PhysicsWorld::new().with_gravity(Vec3::ZERO).with_substep_rate(hz).unwrap();
    let ball = Collider::sphere(0.1).with_material(elastic());
    let mut rb = RigidBody::new(1.0, false);
    rb.update_inertia_from_collider(&ball);
    rb.ccd_enabled = ccd;
    w.add_body(BodyHandle::from_id(1), rb, Transform::new(Vec3::ZERO), Velocity::new(v), ball);
    if let Some((collider, at)) = by {
        w.add_body(BodyHandle::from_id(2), RigidBody::new_static(), Transform::new(at), Velocity::default(), collider.with_material(elastic()));
    }
    for _ in 0..12 {
        w.step(1.0 / 60.0).unwrap();
    }
    (w.transforms[0].position, w.velocities[0].linear)
}

#[test]
fn a_fast_ball_passes_a_box_it_does_not_touch() {
    for hz in [60, 240] {
        for speed in [10.0, 20.0, 40.0, 160.0, 320.0] {
            for ccd in [false, true] {
                let v = Vec3::X * speed;
                let free = fly(hz, v, ccd, None);
                for half in [0.3, 2.0] {
                    for clear in [0.005, 0.02, 0.1, 0.4] {
                        let at = Vec3::new(0.5 + half, 0.1 + clear + half, 0.0);
                        let past = fly(hz, v, ccd, Some((Collider::box_collider(Vec3::splat(half)), at)));
                        assert!(
                            (past.0 - free.0).length() < 1e-3 && (past.1 - free.1).length() < 1e-2,
                            "{hz} Hz, {speed} m/s, CCD {ccd}, {clear} m clear of a {} m box: {past:?}, alone {free:?}",
                            2.0 * half
                        );
                    }
                }
            }
        }
    }
}

/// And one that does hit the box's corner still does: 5 cm into its path, off the corner's line.
#[test]
fn a_fast_ball_that_does_reach_the_box_hits_it() {
    for hz in [60, 240] {
        for speed in [20.0, 160.0, 320.0] {
            let at = Vec3::new(0.8, 0.1 - 0.05 + 0.3, 0.0);
            let (_, v) = fly(hz, Vec3::X * speed, false, Some((Collider::box_collider(Vec3::splat(0.3)), at)));
            assert!(v.y < -0.1 * speed, "{hz} Hz, {speed} m/s: past the corner it reached, at {v:?}");
        }
    }
}

/// Past the border of a mesh platform, 20 cm over it and coming down to its level beyond it.
#[test]
fn a_fast_ball_passes_the_border_of_a_mesh_it_comes_down_beyond() {
    let platform = || {
        let v = [(-4.0, -2.0), (0.0, -2.0), (0.0, 2.0), (-4.0, 2.0)].map(|(x, z)| Vec3::new(x, 0.0, z)).to_vec();
        Collider::trimesh(v, vec![0, 2, 1, 0, 3, 2])
    };
    for hz in [60, 240] {
        for speed in [40.0, 160.0] {
            let v = Vec3::new(1.0, -0.3, 0.0).normalize() * speed;
            let at = Vec3::new(0.3, -0.25, 0.0);
            let (free, past) = (fly(hz, v, true, None), fly(hz, v, true, Some((platform(), at))));
            // The platform's border is 0.3 m ahead and 0.25 m down; the ball's underside is 0.19
            // down when it gets there.
            assert!((past.1 - free.1).length() < 1e-2, "{hz} Hz, {speed} m/s: {past:?}, alone {free:?}");
        }
    }
}

/// A fast ball rolling across the seam of two flat meshes goes on as on one: a border is where a
/// mesh may go on, so a body sunk a hair into the first does not trip on the second's.
#[test]
fn a_fast_ball_rolls_over_the_seam_of_two_meshes() {
    let floor = |x0: f32, x1: f32| {
        let v = [(x0, -5.0), (x1, -5.0), (x1, 5.0), (x0, 5.0)].map(|(x, z)| Vec3::new(x, 0.0, z)).to_vec();
        Collider::trimesh(v, vec![0, 2, 1, 0, 3, 2])
    };
    for hz in [60, 240] {
        for speed in [20.0, 60.0, 200.0] {
            for ccd in [false, true] {
                for sunk in [0.0, 0.002] {
                    let mut w = PhysicsWorld::new().with_substep_rate(hz).unwrap();
                    for (id, f) in [(2, floor(-40.0, 0.0)), (3, floor(0.0, 400.0))] {
                        w.add_body(BodyHandle::from_id(id), RigidBody::new_static(), Transform::new(Vec3::ZERO), Velocity::default(), f);
                    }
                    let c = Collider::sphere(0.1);
                    let mut rb = RigidBody::new(1.0, true);
                    rb.update_inertia_from_collider(&c);
                    rb.ccd_enabled = ccd;
                    let mut v = Velocity::new(Vec3::X * speed);
                    v.angular = Vec3::Z * (-speed / 0.1);
                    w.add_body(BodyHandle::from_id(1), rb, Transform::new(Vec3::new(-1.0 - speed * 0.1, 0.1 - sunk, 0.0)), v, c);
                    for _ in 0..30 {
                        w.step(1.0 / 60.0).unwrap();
                        let i = *w.entity_index_map.get(&1).unwrap();
                        let (p, v) = (w.transforms[i].position, w.velocities[i].linear);
                        assert!((p.y - 0.1).abs() < 0.006 && v.x > 0.9 * speed, "{hz} Hz, {speed} m/s, CCD {ccd}, sunk {sunk}: at {p:?} moving at {v:?}");
                    }
                }
            }
        }
    }
}
