//! **A `RigidBody` without a `Velocity` is still a body.**
//!
//! The ECS→PhysicsWorld gather queries `(RigidBody, Transform, Velocity)`, so a body missing the
//! velocity was left out of the simulation without a word. The natural way to write a floor —
//! `RigidBody::new_static()` and a collider, no velocity, because a floor does not move — was
//! therefore no floor at all: in `lobut` ten pins and a ball fell through it from the first frame.

use gizmo_core::world::World;
use gizmo_math::Vec3;
use gizmo_physics_core::{Collider, Transform};
use gizmo_physics_rigid::components::{RigidBody, Velocity};
use gizmo_physics_rigid::world::PhysicsWorld;

const DT: f32 = 1.0 / 120.0;

/// A static floor with no velocity holds a ball up.
#[test]
fn a_static_floor_without_a_velocity_holds_a_body_up() {
    let mut world = World::new();
    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));

    let floor = world.spawn();
    world.add_component(floor, Transform::new(Vec3::new(0.0, -0.5, 0.0)));
    world.add_component(floor, RigidBody::new_static());
    world.add_component(floor, Collider::box_collider(Vec3::new(5.0, 0.5, 5.0)));

    let ball = world.spawn();
    world.add_component(ball, Transform::new(Vec3::new(0.0, 0.6, 0.0)));
    world.add_component(ball, RigidBody::new(1.0, true));
    world.add_component(ball, Velocity::default());
    world.add_component(ball, Collider::sphere(0.5));

    for _ in 0..240 {
        gizmo_physics_rigid::system::physics_step_system(&world, DT);
        world.apply_commands();
    }
    let y = world.borrow::<Transform>().get(ball.id()).unwrap().position.y;
    assert!((y - 0.5).abs() < 0.05, "the ball should rest on the floor at y = 0.5, it is at {y}");
    assert!(
        world.borrow::<Velocity>().get(floor.id()).is_none(),
        "a static body is simulated as it is — nothing is added to it"
    );
}

/// A moving body without a velocity is given one and then falls like any other.
#[test]
fn a_moving_body_without_a_velocity_is_given_one_and_falls() {
    let mut world = World::new();
    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));

    let body = world.spawn();
    world.add_component(body, Transform::new(Vec3::new(0.0, 10.0, 0.0)));
    world.add_component(body, RigidBody::new(1.0, true));
    world.add_component(body, Collider::sphere(0.5));

    // The first step queues the velocity; the flush adds it.
    gizmo_physics_rigid::system::physics_step_system(&world, DT);
    world.apply_commands();
    assert!(world.borrow::<Velocity>().get(body.id()).is_some(), "no Velocity was added");

    for _ in 0..120 {
        gizmo_physics_rigid::system::physics_step_system(&world, DT);
        world.apply_commands();
    }
    let y = world.borrow::<Transform>().get(body.id()).unwrap().position.y;
    // One second of free fall from rest is 4.9 m; the first step it missed is 1/120 s.
    assert!((y - (10.0 - 4.9)).abs() < 0.3, "a second of free fall should end near y = 5.1, got {y}");
}
