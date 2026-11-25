//! A fast body shot at a wall does not pass through it, whatever the angle, the turn of the wall or
//! of the body, with CCD or without.
//!
//! Whether a pair is fast — too fast for discrete detection, so it takes a speculative contact — is
//! judged against the two shapes' thickness. That thickness was the smallest half-extent of each
//! shape's world bounds, and a wall turned a few degrees has bounds as thick as it is wide: a body
//! shot at it was judged slow, met it only overlapping past its middle, and the shortest way out was
//! the far side. Of 6000 seeded shots per rate, 26 % went through at 60 Hz and 49 % at 240 Hz
//! without CCD, 0.3 % and 0.7 % with it (2026-11-25). Now the thickness is the shape's own
//! (`ColliderShape::half_thickness`), and none does.

use gizmo_math::{Quat, Vec3};
use gizmo_physics_core::{BodyHandle, Collider, Transform};
use gizmo_physics_rigid::{PhysicsWorld, RigidBody, Velocity};

fn seeded(seed: &mut u32) -> f32 {
    *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    (*seed >> 8) as f32 / (1u32 << 24) as f32
}

/// One seeded shot: a sphere, a box or a capsule, 5–35 cm, turned every way, at 50–3000 m/s, at a
/// wall 4–80 cm thick tilted up to ±35° and within 60° of square on. Whether it came out of the far
/// face within the wall's span.
fn goes_through(seed: &mut u32, hz: u32, ccd: bool) -> Option<String> {
    let mut u = || seeded(seed);
    let half = 0.02 + 0.38 * u();
    let wall_turn = Quat::from_euler(gizmo_math::EulerRot::XYZ, 0.6 * (u() - 0.5), 1.2 * (u() - 0.5), 1.2 * (u() - 0.5));
    let speed = 50.0 + 2950.0 * u() * u();
    let kind = (u() * 3.0) as u32;
    let size = 0.05 + 0.3 * u();
    let body_turn = Quat::from_euler(gizmo_math::EulerRot::XYZ, 6.3 * u(), 6.3 * u(), 6.3 * u());
    let normal = wall_turn * Vec3::X;
    let aside = Vec3::new(0.0, u() - 0.5, u() - 0.5) * 1.6;
    let dir = (normal + wall_turn * aside).normalize();

    let mut w = PhysicsWorld::new().with_substep_rate(hz).unwrap().with_gravity(Vec3::ZERO);
    let mut wall = Transform::new(Vec3::ZERO);
    wall.rotation = wall_turn;
    w.add_body(BodyHandle::from_id(1), RigidBody::new_static(), wall, Velocity::default(), Collider::box_collider(Vec3::new(half, 20.0, 20.0)));
    let shape = match kind {
        0 => Collider::sphere(size),
        1 => Collider::box_collider(Vec3::new(size, size * 0.7, size * 0.4)),
        _ => Collider::capsule(size * 0.5, size),
    };
    let mut rb = RigidBody::new(1.0, true);
    rb.update_inertia_from_collider(&shape);
    rb.ccd_enabled = ccd;
    let mut at = Transform::new(-dir * 8.0);
    at.rotation = body_turn;
    w.add_body(BodyHandle::from_id(2), rb, at, Velocity::new(dir * speed), shape);
    for _ in 0..40 {
        w.step(1.0 / 60.0).unwrap();
        let i = *w.entity_index_map.get(&2).unwrap();
        let local = wall_turn.inverse() * w.transforms[i].position;
        if local.x > half && local.y.abs() < 19.0 && local.z.abs() < 19.0 {
            return Some(format!("{} {size:.3} m at {speed:.0} m/s into a wall {:.3} m thick", ["sphere", "box", "capsule"][kind as usize], 2.0 * half));
        }
    }
    None
}

#[test]
fn no_seeded_shot_goes_through_a_turned_wall() {
    for ccd in [false, true] {
        for hz in [60, 240] {
            let mut seed = 7 + hz;
            let through: Vec<String> = (0..600).filter_map(|_| goes_through(&mut seed, hz, ccd)).collect();
            assert!(through.is_empty(), "{hz} Hz, CCD {ccd}: {} of 600 went through, e.g. {:?}", through.len(), &through[..through.len().min(3)]);
        }
    }
}

/// A fast body straight onto the ridge of a roof, a mesh and a height field: it meets neither
/// slope along its face, its deepest point towards each coming down past the ridge, so the ridge
/// itself is swept for. The roof slopes 1 in 2, its ridge along z at the top.
#[test]
fn no_shot_goes_through_a_ridge() {
    let roof = |x: f32| -0.5 * x.abs();
    let mesh = {
        let v = [(-8.0, -8.0), (0.0, -8.0), (8.0, -8.0), (-8.0, 8.0), (0.0, 8.0), (8.0, 8.0)].map(|(x, z)| Vec3::new(x, roof(x), z)).to_vec();
        Collider::trimesh(v, vec![0, 3, 1, 1, 3, 4, 1, 4, 2, 2, 4, 5])
    };
    let field = Collider::heightfield((0..17).flat_map(|_| (0..17).map(|x| roof(x as f32 - 8.0))).collect(), 17, 17, Vec3::ONE);
    for (what, ground) in [("mesh", mesh), ("field", field)] {
        for hz in [60, 240] {
            for speed in [60.0, 300.0, 1000.0] {
                for off in [0.0, 0.01, 0.03] {
                    for (shape, r) in [(Collider::sphere(0.1), 0.1), (Collider::box_collider(Vec3::splat(0.15)), 0.15)] {
                        let mut w = PhysicsWorld::new().with_substep_rate(hz).unwrap().with_gravity(Vec3::ZERO);
                        w.add_body(BodyHandle::from_id(1), RigidBody::new_static(), Transform::new(Vec3::ZERO), Velocity::default(), ground.clone());
                        let mut rb = RigidBody::new(1.0, true);
                        rb.update_inertia_from_collider(&shape);
                        w.add_body(BodyHandle::from_id(2), rb, Transform::new(Vec3::new(off, 6.0, 0.3)), Velocity::new(Vec3::Y * -speed), shape);
                        for _ in 0..20 {
                            w.step(1.0 / 60.0).unwrap();
                            let p = w.transforms[1].position;
                            assert!(
                                p.x.abs() > 7.5 || p.y > roof(p.x) - 0.02,
                                "{what}, {hz} Hz: {r} m at {speed} m/s, {off} m off the ridge, is {:.2} m under the roof at {p:?}",
                                roof(p.x) - p.y
                            );
                        }
                    }
                }
            }
        }
    }
}

/// A fast ball onto a gently rolling mesh, whose every fold is under the 5° an edge is known for a
/// ridge from: straight down on its vertices, its edges and its faces, none goes through. Between
/// two faces 2° apart there are 5 mm a 0.15 m ball's deepest point comes down on neither, and of
/// 2000 dropped on one at 60 m/s, 36 fell through at 60 Hz and 2 at 240.
#[test]
fn no_shot_goes_through_a_gently_folded_mesh() {
    let n = 12u32;
    let at = |k: u32| k as f32 - n as f32 / 2.0;
    let height = |x: f32, z: f32| 0.15 * ((x * 0.5).sin() + (z * 0.4).cos());
    let (mut v, mut idx, w) = (Vec::new(), Vec::new(), n + 1);
    for z in 0..=n {
        for x in 0..=n {
            v.push(Vec3::new(at(x), height(at(x), at(z)), at(z)));
        }
    }
    for z in 0..n {
        for x in 0..n {
            let a = z * w + x;
            idx.extend([a, a + w, a + 1, a + 1, a + w, a + w + 1]);
        }
    }
    let ground = Collider::trimesh(v, idx);
    let mut seed = 21u32;
    for hz in [60, 240] {
        for k in 0..600 {
            let (r, speed) = (0.05 + 0.2 * seeded(&mut seed), 40.0 + 200.0 * seeded(&mut seed));
            // A vertex, the middle of an edge (along x, along z, across the cell), or anywhere.
            let cell = Vec3::new((seeded(&mut seed) * 8.0).floor() - 4.0, 0.0, (seeded(&mut seed) * 8.0).floor() - 4.0);
            let within = match k % 5 {
                0 => Vec3::ZERO,
                1 => Vec3::X * 0.5,
                2 => Vec3::Z * 0.5,
                3 => Vec3::new(0.5, 0.0, 0.5),
                _ => Vec3::new(seeded(&mut seed), 0.0, seeded(&mut seed)),
            };
            let off = Vec3::new(seeded(&mut seed) - 0.5, 0.0, seeded(&mut seed) - 0.5) * 0.02;
            let from = cell + within + off + Vec3::Y * 3.0;
            let c = Collider::sphere(r);
            let mut rb = RigidBody::new(1.0, true);
            rb.update_inertia_from_collider(&c);
            let mut w = PhysicsWorld::new().with_substep_rate(hz).unwrap().with_gravity(Vec3::ZERO);
            w.add_body(BodyHandle::from_id(1), RigidBody::new_static(), Transform::new(Vec3::ZERO), Velocity::default(), ground.clone());
            w.add_body(BodyHandle::from_id(2), rb, Transform::new(from), Velocity::new(Vec3::Y * -speed), c);
            for _ in 0..12 {
                w.step(1.0 / 60.0).unwrap();
                let p = w.transforms[1].position;
                assert!(p.y > -0.3, "{hz} Hz, shot {k}: a {r:.2} m ball at {speed:.0} m/s from {from:?} fell through, at {p:?}");
            }
        }
    }
}
