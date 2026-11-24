//! A convex hull comes to rest on a face, and sleeps.
//!
//! A hull against a box or another hull went to GJK/EPA, which gives one contact point a step. A hull
//! resting on a face had one support at a time, wandering between its corners, and rocked for ever:
//! not one of these six hulls slept in ten seconds, each still turning at 0.02–0.15 rad/s and sunk
//! 6–12 mm (2026-11-24). The faces are clipped to a patch now (`narrowphase::clip_polyhedra`).

use gizmo_math::{Quat, Vec3};
use gizmo_physics_core::{BodyHandle, Collider, Transform};
use gizmo_physics_rigid::{PhysicsWorld, RigidBody, Velocity};

#[test]
fn an_irregular_hull_dropped_on_the_floor_comes_to_rest_and_sleeps() {
    let mut rng = 0x1234_5678u32;
    let mut roll = || {
        rng ^= rng << 13;
        rng ^= rng >> 17;
        rng ^= rng << 5;
        rng as f32 / u32::MAX as f32
    };
    for trial in 0..6 {
        let points: Vec<Vec3> = (0..10).map(|_| Vec3::new(roll() - 0.5, (roll() - 0.5) * 0.6, (roll() - 0.5) * 0.3)).collect();
        let mut world = PhysicsWorld::new();
        world.add_body(
            BodyHandle::from_id(0),
            RigidBody::new_static(),
            Transform::new(Vec3::new(0.0, -0.5, 0.0)),
            Velocity::default(),
            Collider::box_collider(Vec3::new(20.0, 0.5, 20.0)),
        );
        let hull = Collider::convex_hull(&points);
        let mut rb = RigidBody::new(3.0, true);
        rb.update_inertia_from_collider(&hull);
        let at = Transform::new(Vec3::new(0.0, 0.6, 0.0)).with_rotation(Quat::from_rotation_z(0.3 * trial as f32));
        world.add_body(BodyHandle::from_id(1), rb, at, Velocity::default(), hull);
        let asleep_at = (0..600).find(|_| {
            world.step(1.0 / 60.0).expect("step");
            world.rigid_bodies[1].is_sleeping
        });
        assert!(asleep_at.is_some(), "hull {trial}: never slept in ten seconds (ω = {})", world.velocities[1].angular.length());
        let lowest = world.colliders[1].compute_aabb(world.transforms[1].position, world.transforms[1].rotation).min.y;
        assert!(lowest > -0.01, "hull {trial}: rests {lowest} m into the floor");
    }
}

/// A cube built as a hull, resting on a box, touches it at its four corners — not at one.
#[test]
fn a_cube_shaped_hull_on_a_box_touches_at_four_points() {
    let corners: Vec<Vec3> = (0..8).map(|i| Vec3::new(if i & 1 == 0 { -0.5 } else { 0.5 }, if i & 2 == 0 { -0.5 } else { 0.5 }, if i & 4 == 0 { -0.5 } else { 0.5 })).collect();
    let hull = Collider::convex_hull(&corners);
    let floor = Collider::box_collider(Vec3::new(5.0, 0.5, 5.0));
    let contacts = gizmo_physics_core::NarrowPhase::test_collision_manifold(
        &hull.shape,
        Vec3::new(0.0, 0.49, 0.0),
        Quat::IDENTITY,
        &floor.shape,
        Vec3::new(0.0, -0.5, 0.0),
        Quat::IDENTITY,
    );
    assert_eq!(contacts.len(), 4, "{contacts:?}");
    for c in &contacts {
        assert!((c.penetration - 0.01).abs() < 1e-3, "each corner 1 cm in: {}", c.penetration);
        assert!(c.normal.y < -0.99, "the normal from the hull down into the floor: {:?}", c.normal);
    }
}
