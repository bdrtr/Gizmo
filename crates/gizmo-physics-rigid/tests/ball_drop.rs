//! A ball dropped straight onto a floor bounces straight and rests where it fell.
//!
//! It rolled away diagonally: the sphere's contact came from GJK/EPA, off the centre and tilted, so
//! every bounce spun it. Measured on the code before, a 0.25 m ball dropped from 3 m ended 0.82 m
//! away on a box or hull floor and 0.49 m away on a cylinder's face; a 0.5 m ball, 1.54 m.

use gizmo_core::world::World;
use gizmo_math::Vec3;
use gizmo_physics_core::{Collider, Transform};
use gizmo_physics_rigid::components::{RigidBody, Velocity};
use gizmo_physics_rigid::system::physics_step_system;
use gizmo_physics_rigid::world::PhysicsWorld;

fn rest_of_a_drop(floor: Collider, radius: f32) -> Vec3 {
    let mut world = World::new();
    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));
    let f = world.spawn();
    world.add_component(f, Transform::new(Vec3::ZERO));
    world.add_component(f, floor);
    let ball = world.spawn();
    world.add_component(ball, Transform::new(Vec3::new(0.0, 3.0, 0.0)));
    world.add_component(ball, RigidBody::new(1.0, true));
    world.add_component(ball, Velocity::default());
    world.add_component(ball, Collider::sphere(radius));
    for _ in 0..240 {
        world.tick += 1;
        physics_step_system(&world, 1.0 / 60.0);
    }
    world.query::<&Transform>().get(ball).map(|t| t.position).expect("the ball")
}

#[test]
fn a_ball_dropped_straight_rests_where_it_fell() {
    let corners: Vec<Vec3> = [-1.0f32, 1.0]
        .iter()
        .flat_map(|&x| [-1.0f32, 1.0].iter().flat_map(move |&y| [-1.0f32, 1.0].iter().map(move |&z| Vec3::new(5.0 * x, 0.2 * y, 5.0 * z))))
        .collect();
    for (name, floor) in [
        ("box", Collider::box_collider(Vec3::new(5.0, 0.2, 5.0))),
        ("hull", Collider::convex_hull(&corners)),
        ("cylinder", Collider::cylinder(5.0, 0.2)),
    ] {
        for radius in [0.25, 0.5] {
            let at = rest_of_a_drop(floor.clone(), radius);
            assert!(at.x.abs() < 1e-3 && at.z.abs() < 1e-3, "on a {name}, a {radius} m ball rolled to {at}");
            assert!((at.y - (0.2 + radius)).abs() < 0.01, "on a {name}, a {radius} m ball rests on its top: y {}", at.y);
        }
    }
}
