//! A shape resting on a floor rests on what of it lies along the floor — a face, a flat end, a side
//! — not on one point of it: on a plane, a triangle mesh, a heightfield and a box, at 60 and 240 Hz.
//!
//! GJK/EPA gives one point a step, and every triangle of a surface is tested on its own. A box inside
//! one mesh triangle had a single support, two across an edge, and rocked on them; at 60 Hz a 10 cm
//! box dropped on a mesh floor took back less than gravity's `g·dt` a step, sank 16 cm a second, and
//! fell through once its centre crossed the plane. A standing or lying cylinder and a lying capsule
//! fell through the mesh too, and sank 7 to 14 cm into a plane or a box; a convex hull sank 7 cm into
//! a plane (all 2026-11-25). The 240 Hz default's four-times-tighter step had hidden it. What faces
//! the floor is now its contact patch (`NarrowPhase`'s `support_feature`).

use gizmo_math::{Quat, Vec3};
use gizmo_physics_core::{BodyHandle, Collider, Transform};
use gizmo_physics_rigid::{PhysicsWorld, RigidBody, Velocity};

/// A flat mesh floor of unit squares, each cut along its diagonal, around the origin.
fn mesh_floor() -> Collider {
    let n = 20;
    let mut v = Vec::new();
    let mut idx = Vec::new();
    for j in 0..=n {
        for i in 0..=n {
            v.push(Vec3::new(i as f32 - n as f32 / 2.0, 0.0, j as f32 - n as f32 / 2.0));
        }
    }
    let row = (n + 1) as u32;
    for j in 0..n as u32 {
        for i in 0..n as u32 {
            let a = j * row + i;
            idx.extend_from_slice(&[a, a + row, a + row + 1, a, a + row + 1, a + 1]);
        }
    }
    Collider::trimesh(v, idx)
}

/// A flat heightfield floor of the same size.
fn terrain_floor() -> Collider {
    Collider::heightfield(vec![0.0; 21 * 21], 21, 21, Vec3::new(20.0, 1.0, 20.0))
}

/// Where a 10 cm box dropped on `floor` from 45 cm up, at `(x, z)`, is after 5 s at `hz`: its
/// height, its tilt (radians) and whether it fell asleep.
fn dropped_box(floor: Collider, floor_at: Vec3, hz: u32, x: f32, z: f32) -> (f32, f32, bool) {
    dropped(Collider::box_collider(Vec3::splat(0.05)), floor, floor_at, hz, x, z)
}

/// `dropped_box` for any 10 cm shape.
fn dropped(shape: Collider, floor: Collider, floor_at: Vec3, hz: u32, x: f32, z: f32) -> (f32, f32, bool) {
    dropped_turned(shape, Quat::IDENTITY, floor, floor_at, hz, x, z)
}

/// `dropped` for a shape let go turned by `turn`; the tilt is measured from that.
fn dropped_turned(shape: Collider, turn: Quat, floor: Collider, floor_at: Vec3, hz: u32, x: f32, z: f32) -> (f32, f32, bool) {
    let mut w = PhysicsWorld::new().with_substep_rate(hz).unwrap();
    w.add_body(BodyHandle::from_id(1), RigidBody::new_static(), Transform::new(floor_at), Velocity::default(), floor);
    let mut rb = RigidBody::new(1.0, true);
    rb.update_inertia_from_collider(&shape);
    let mut at = Transform::new(Vec3::new(x, 0.5, z));
    at.rotation = turn;
    w.add_body(BodyHandle::from_id(2), rb, at, Velocity::default(), shape);
    for _ in 0..300 {
        w.step(1.0 / 60.0).unwrap();
    }
    let i = *w.entity_index_map.get(&2).unwrap();
    let t = &w.transforms[i];
    let tilt = (t.rotation * Vec3::Y).angle_between(turn * Vec3::Y);
    (t.position.y, tilt, w.rigid_bodies[i].is_sleeping)
}

/// A hexagonal prism 10 cm tall, 12 cm across: its base is a face of six corners.
fn hex_prism() -> Collider {
    let corners: Vec<Vec3> = (0..6)
        .flat_map(|k| {
            let a = k as f32 * std::f32::consts::TAU / 6.0;
            [Vec3::new(0.06 * a.cos(), -0.05, 0.06 * a.sin()), Vec3::new(0.06 * a.cos(), 0.05, 0.06 * a.sin())]
        })
        .collect();
    Collider::convex_hull(&corners)
}

#[test]
fn a_box_dropped_on_a_mesh_rests_flat_at_every_rate() {
    // Inside one triangle, across a diagonal, and over a vertex where six triangles meet.
    for (x, z) in [(0.3, 0.2), (0.5, 0.5), (0.0, 0.0)] {
        for hz in [60, 240] {
            let (y, tilt, asleep) = dropped_box(mesh_floor(), Vec3::ZERO, hz, x, z);
            assert!((y - 0.05).abs() < 0.003, "{hz} Hz at ({x}, {z}): the box rests at y = {y}, not on the floor (0.05)");
            assert!(tilt < 0.01, "{hz} Hz at ({x}, {z}): the box rests tilted {tilt} rad");
            assert!(asleep, "{hz} Hz at ({x}, {z}): the box is still moving after 5 s");
        }
    }
}

#[test]
fn a_box_dropped_on_a_heightfield_rests_flat_at_every_rate() {
    for (x, z) in [(0.3, 0.2), (0.5, 0.5), (0.0, 0.0)] {
        for hz in [60, 240] {
            let (y, tilt, asleep) = dropped_box(terrain_floor(), Vec3::new(-10.0, 0.0, -10.0), hz, x, z);
            assert!((y - 0.05).abs() < 0.003, "{hz} Hz at ({x}, {z}): the box rests at y = {y}, not on the floor (0.05)");
            assert!(tilt < 0.01, "{hz} Hz at ({x}, {z}): the box rests tilted {tilt} rad");
            assert!(asleep, "{hz} Hz at ({x}, {z}): the box is still moving after 5 s");
        }
    }
}

#[test]
fn a_hull_dropped_on_a_mesh_rests_flat() {
    for hz in [60, 240] {
        let (y, tilt, asleep) = dropped(hex_prism(), mesh_floor(), Vec3::ZERO, hz, 0.3, 0.2);
        assert!((y - 0.05).abs() < 0.003, "{hz} Hz: the hull rests at y = {y}, not on the floor (0.05)");
        assert!(tilt < 0.01, "{hz} Hz: the hull rests tilted {tilt} rad");
        assert!(asleep, "{hz} Hz: the hull is still moving after 5 s");
    }
}

#[test]
fn a_box_on_a_mesh_triangle_has_its_four_corners_in_contact() {
    use gizmo_physics_core::narrowphase::NarrowPhase;
    let shape = Collider::box_collider(Vec3::splat(0.05)).shape;
    let floor = mesh_floor().shape;
    // 5 mm into the floor, inside one triangle, and turned about the vertical.
    let turn = Quat::from_rotation_y(0.3);
    let contacts = NarrowPhase::test_collision_manifold(&shape, Vec3::new(0.3, 0.045, 0.2), turn, &floor, Vec3::ZERO, Quat::IDENTITY);
    assert_eq!(contacts.len(), 4, "{contacts:?}");
    for c in &contacts {
        assert!((c.penetration - 0.005).abs() < 1e-4, "depth {} at {:?}", c.penetration, c.point);
        assert!(c.normal.dot(-Vec3::Y) > 0.9999, "normal {:?}", c.normal);
        let corner = turn.inverse() * (c.point - Vec3::new(0.3, 0.045, 0.2));
        assert!((corner.x.abs() - 0.05).abs() < 1e-3 && (corner.z.abs() - 0.05).abs() < 1e-3, "not a corner: {corner:?}");
    }
}

/// Every round shape, and a hull, on every kind of floor: at the height of what it rests on, not
/// tilted, and asleep.
#[test]
fn round_shapes_rest_on_their_end_or_their_side_on_every_floor() {
    let lying = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
    // The shape, how it is let go, and the height its origin rests at.
    let shapes: [(&str, Collider, Quat, f32); 5] = [
        ("standing cylinder", Collider::cylinder(0.06, 0.05), Quat::IDENTITY, 0.05),
        ("lying cylinder", Collider::cylinder(0.05, 0.1), lying, 0.05),
        ("lying capsule", Collider::capsule(0.05, 0.1), lying, 0.05),
        ("cone", Collider::cone(0.08, 0.06), Quat::IDENTITY, 0.06),
        ("hexagonal hull", hex_prism(), Quat::IDENTITY, 0.05),
    ];
    let floors: [(&str, Collider, Vec3); 4] = [
        ("plane", Collider::plane(Vec3::Y, 0.0), Vec3::ZERO),
        ("mesh", mesh_floor(), Vec3::ZERO),
        ("heightfield", terrain_floor(), Vec3::new(-10.0, 0.0, -10.0)),
        ("box", Collider::box_collider(Vec3::new(10.0, 0.5, 10.0)), Vec3::new(0.0, -0.5, 0.0)),
    ];
    for (name, shape, turn, rest) in &shapes {
        for (floor_name, floor, floor_at) in &floors {
            for hz in [60, 240] {
                let (y, tilt, asleep) = dropped_turned(shape.clone(), *turn, floor.clone(), *floor_at, hz, 0.3, 0.2);
                let at = format!("{name} on a {floor_name} at {hz} Hz");
                assert!((y - rest).abs() < 0.004, "{at}: rests at y = {y}, not on the floor ({rest})");
                assert!(tilt < 0.01, "{at}: rests tilted {tilt} rad");
                // A lying cylinder on a mesh at 240 Hz rolled away at 4.5 mm/s: one end lay on a
                // triangle's edge, and the patch there took EPA's normal, 0.6° off the face's.
                assert!(asleep, "{at}: still moving after 5 s");
            }
        }
    }
}

/// A ball on a mesh or a heightfield touches it straight below its centre, and stays where it lands.
/// EPA's point against a triangle was up to 1.6 mm off to the side; the normal pushed there turned
/// the ball, and it rolled away and never slept.
#[test]
fn a_ball_dropped_on_a_mesh_or_a_heightfield_stays_where_it_lands() {
    let floors: [(&str, Collider, Vec3); 2] = [("mesh", mesh_floor(), Vec3::ZERO), ("heightfield", terrain_floor(), Vec3::new(-10.0, 0.0, -10.0))];
    for (floor_name, floor, floor_at) in floors {
        for hz in [60, 240] {
            for (x, z) in [(0.3, 0.2), (0.7, 0.2), (0.4, 0.8)] {
                let mut w = PhysicsWorld::new().with_substep_rate(hz).unwrap();
                w.add_body(BodyHandle::from_id(1), RigidBody::new_static(), Transform::new(floor_at), Velocity::default(), floor.clone());
                let shape = Collider::sphere(0.05);
                let mut rb = RigidBody::new(1.0, true);
                rb.update_inertia_from_collider(&shape);
                w.add_body(BodyHandle::from_id(2), rb, Transform::new(Vec3::new(x, 0.5, z)), Velocity::default(), shape);
                for _ in 0..300 {
                    w.step(1.0 / 60.0).unwrap();
                }
                let i = *w.entity_index_map.get(&2).unwrap();
                let p = w.transforms[i].position;
                let at = format!("a ball on a {floor_name} at {hz} Hz from ({x}, {z})");
                assert!(Vec3::new(p.x - x, 0.0, p.z - z).length() < 1e-3, "{at}: rolled to ({}, {})", p.x, p.z);
                assert!((p.y - 0.05).abs() < 0.004, "{at}: rests at y = {}", p.y);
                assert!(w.rigid_bodies[i].is_sleeping, "{at}: still moving after 5 s");
            }
        }
    }
}

/// The four points kept of a symmetric patch hold its centre: a hexagonal hull on a plane rests on
/// four of its six corners round its middle, not on half its base.
#[test]
fn the_four_contacts_kept_of_a_hexagon_hold_its_centre() {
    use gizmo_physics_core::narrowphase::NarrowPhase;
    let at = Vec3::new(0.3, 0.045, 0.2);
    let contacts = NarrowPhase::test_collision_manifold(&hex_prism().shape, at, Quat::IDENTITY, &Collider::plane(Vec3::Y, 0.0).shape, Vec3::ZERO, Quat::IDENTITY);
    assert_eq!(contacts.len(), 4, "{contacts:?}");
    let centre = contacts.iter().map(|c| c.point).sum::<Vec3>() / 4.0;
    assert!(Vec3::new(centre.x - at.x, 0.0, centre.z - at.z).length() < 1e-3, "the four contacts are centred on {centre:?}, the hull on {at:?}");
}
