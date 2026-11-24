//! A sleeping body put somewhere by hand — its `Transform` or `Velocity` written through the ECS —
//! wakes, as Box2D's `SetTransform` wakes a body. A body new to the world keeps the sleep it was
//! spawned with.
//!
//! Until 2026-11-25 it stayed asleep where it was put: `tahterevalli` set its plank back level at the
//! end of a round, and when the plank had fallen asleep by then it hung level in the air.

use gizmo_core::world::World;
use gizmo_core::Entity;
use gizmo_math::Vec3;
use gizmo_physics_core::{Collider, Transform};
use gizmo_physics_rigid::components::{RigidBody, Velocity};
use gizmo_physics_rigid::system::physics_step_system;
use gizmo_physics_rigid::world::PhysicsWorld;

const DT: f32 = 1.0 / 60.0;

/// A box resting on the ground until it sleeps.
fn asleep_on_the_ground() -> (World, Entity) {
    let mut world = World::new();
    world.insert_resource(PhysicsWorld::new());
    let ground = world.spawn();
    world.add_component(ground, Transform::new(Vec3::new(0.0, -0.5, 0.0)));
    world.add_component(ground, RigidBody::new_static());
    world.add_component(ground, Collider::box_collider(Vec3::new(10.0, 0.5, 10.0)));
    let body = world.spawn();
    world.add_component(body, Transform::new(Vec3::new(0.0, 0.5, 0.0)));
    let mut rb = RigidBody::new(1.0, true);
    rb.calculate_box_inertia(1.0, 1.0, 1.0);
    world.add_component(body, rb);
    world.add_component(body, Velocity::default());
    world.add_component(body, Collider::box_collider(Vec3::splat(0.5)));
    for _ in 0..120 {
        physics_step_system(&world, DT);
        world.apply_commands();
    }
    assert!(world.query::<&RigidBody>().get(body).unwrap().is_sleeping, "the box fell asleep");
    (world, body)
}

fn height(world: &World, body: Entity) -> f32 {
    world.query::<&Transform>().get(body).unwrap().position.y
}

/// Lifted 5 cm — near enough that its pair with the ground stays a dormant one and never ends (an
/// ended contact wakes its bodies on its own): it falls back.
#[test]
fn a_sleeping_body_put_up_in_the_air_wakes_and_falls() {
    let (mut world, body) = asleep_on_the_ground();
    world.query_mut::<&mut Transform>().get_mut(body).unwrap().position.y = 0.55;
    for _ in 0..30 {
        physics_step_system(&world, DT);
        world.apply_commands();
    }
    let y = height(&world, body);
    assert!(y < 0.51, "the box hangs where it was put: y = {y}");
}

#[test]
fn a_sleeping_body_set_moving_wakes() {
    let (mut world, body) = asleep_on_the_ground();
    world.query_mut::<&mut Velocity>().get_mut(body).unwrap().linear = Vec3::new(0.0, 6.0, 0.0);
    physics_step_system(&world, DT);
    world.apply_commands();
    assert!(height(&world, body) > 0.55, "the box was set moving and did not move");
}

/// A body spawned asleep in the air sleeps there: it is new to the world, put there whole, not moved.
#[test]
fn a_body_spawned_asleep_stays_asleep() {
    let (mut world, _) = asleep_on_the_ground();
    let hung = world.spawn();
    world.add_component(hung, Transform::new(Vec3::new(3.0, 5.0, 0.0)));
    let mut rb = RigidBody::new(1.0, true);
    rb.calculate_box_inertia(1.0, 1.0, 1.0);
    rb.is_sleeping = true;
    world.add_component(hung, rb);
    world.add_component(hung, Velocity::default());
    world.add_component(hung, Collider::box_collider(Vec3::splat(0.5)));
    for _ in 0..30 {
        physics_step_system(&world, DT);
        world.apply_commands();
    }
    assert!((height(&world, hung) - 5.0).abs() < 1e-5, "it was spawned asleep at 5 m and moved");
}
