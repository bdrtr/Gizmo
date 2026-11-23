//! A sphere's contact with a convex shape is its centre's closest point: on a flat face, straight
//! under the centre and along the face's normal, whichever of the pair is A.
//!
//! The pair went to GJK/EPA, whose contact point and normal are the polytope's estimates: for a
//! ball resting on a box's top the point sat 2.7 mm off the centre, the normal was tilted
//! 4·10⁻⁵ rad, and swapping the pair did not mirror it. A torque at every bounce — the ball rolled
//! away (`gizmo-physics-rigid/tests/ball_drop.rs`).

use gizmo_math::{Quat, Vec3};
use gizmo_physics_core::narrowphase::NarrowPhase;
use gizmo_physics_core::{Collider, ColliderShape};

fn floors() -> Vec<(&'static str, ColliderShape)> {
    let corners: Vec<Vec3> = [-1.0f32, 1.0]
        .iter()
        .flat_map(|&x| [-1.0f32, 1.0].iter().flat_map(move |&y| [-1.0f32, 1.0].iter().map(move |&z| Vec3::new(5.0 * x, 0.2 * y, 5.0 * z))))
        .collect();
    vec![
        ("a box", Collider::box_collider(Vec3::new(5.0, 0.2, 5.0)).shape),
        ("a hull", Collider::convex_hull(&corners).shape),
        ("a cylinder's face", Collider::cylinder(5.0, 0.2).shape),
    ]
}

#[test]
fn a_ball_on_a_flat_face_touches_it_straight_under_its_centre() {
    let ball = Collider::sphere(0.25).shape;
    let at = Vec3::new(0.3, 0.44, -0.7);
    for (name, floor) in floors() {
        for (order, contacts, normal) in [
            ("ball first", NarrowPhase::test_collision_manifold(&ball, at, Quat::IDENTITY, &floor, Vec3::ZERO, Quat::IDENTITY), Vec3::NEG_Y),
            ("floor first", NarrowPhase::test_collision_manifold(&floor, Vec3::ZERO, Quat::IDENTITY, &ball, at, Quat::IDENTITY), Vec3::Y),
        ] {
            assert_eq!(contacts.len(), 1, "{name}, {order}: one contact");
            let c = contacts[0];
            assert!((c.point - Vec3::new(at.x, 0.2, at.z)).length() < 1e-5, "{name}, {order}: under the centre, at {}", c.point);
            assert!((c.normal - normal).length() < 1e-5, "{name}, {order}: along the face's normal, {}", c.normal);
            assert!((c.penetration - 0.01).abs() < 1e-4, "{name}, {order}: 1 cm deep, {}", c.penetration);
        }
    }
}
