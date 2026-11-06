//! A character walks through trigger colliders and reports the ones it is inside
//! (`CharacterController::triggers`).
//!
//! Before it, the controller swept against every collider alike: a character walking into a door
//! sensor (`Collider::with_trigger(true)`) stopped at it as at a wall (x 1.20 in front of a sensor
//! at x 2), and since a character without a `RigidBody` is not in the physics world, the world's
//! `trigger_events()` never named it (0 events over 2 s) — a pressure plate could not be built.

#![cfg(feature = "ecs")]

use gizmo_core::entity::Entity;
use gizmo_core::world::World;
use gizmo_math::Vec3;
use gizmo_physics_core::components::CharacterController;
use gizmo_physics_core::{BodyHandle, Collider, Transform};
use gizmo_physics_rigid::components::{RigidBody, Velocity};
use gizmo_physics_rigid::world::PhysicsWorld;

fn body(world: &mut World, at: Vec3, collider: Collider) -> Entity {
    let e = world.spawn();
    world.add_component(e, Transform::new(at));
    world.add_component(e, RigidBody::new_static());
    world.add_component(e, Velocity::default());
    world.add_component(e, collider);
    e
}

fn character(world: &mut World, at: Vec3, walk: Vec3) -> Entity {
    let p = world.spawn();
    world.add_component(p, Transform::new(at));
    world.add_component(p, CharacterController { target_velocity: walk, ..Default::default() });
    world.add_component(p, Velocity::default());
    world.add_component(p, Collider::capsule(0.3, 0.4));
    p
}

fn step(world: &mut World) {
    let dt = 1.0 / 60.0;
    gizmo_physics_dynamics::character_controller_system(world, dt);
    gizmo_physics_rigid::system::physics_step_system(world, dt);
    world.apply_commands();
}

fn position(world: &World, e: Entity) -> Vec3 {
    world.query::<&Transform>().get(e).unwrap().position
}

#[test]
fn a_character_walks_through_a_trigger_and_reports_it_while_inside() {
    let mut world = World::new();
    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));
    body(&mut world, Vec3::new(0.0, -0.5, 0.0), Collider::box_collider(Vec3::new(20.0, 0.5, 20.0)));
    let sensor = body(
        &mut world,
        Vec3::new(2.0, 1.0, 0.0),
        Collider::box_collider(Vec3::new(0.5, 1.0, 0.5)).with_trigger(true),
    );
    let p = character(&mut world, Vec3::new(0.0, 0.7, 0.0), Vec3::new(3.0, 0.0, 0.0));
    let (mut first, mut last) = (f32::MAX, f32::MIN);
    for _ in 0..120 {
        step(&mut world);
        let x = position(&world, p).x;
        let triggers = world.query::<&CharacterController>().get(p).unwrap().triggers.clone();
        if !triggers.is_empty() {
            assert_eq!(triggers, vec![BodyHandle::from_id(sensor.id())]);
            first = first.min(x);
            last = last.max(x);
        }
    }
    assert!(position(&world, p).x > 5.5, "the sensor stopped the character at x = {:.2}", position(&world, p).x);
    // Inside exactly while the capsule (radius 0.3) overlaps the box (half-width 0.5 at x = 2).
    assert!((first - 1.2).abs() < 0.06 && (last - 2.8).abs() < 0.06, "reported inside for x {first:.2}..{last:.2}");
}

#[test]
fn a_trigger_is_not_ground() {
    // A sensor slab under the feet and no floor: the character falls through it.
    let mut world = World::new();
    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));
    body(&mut world, Vec3::new(0.0, 0.0, 0.0), Collider::box_collider(Vec3::new(2.0, 0.1, 2.0)).with_trigger(true));
    let p = character(&mut world, Vec3::new(0.0, 0.85, 0.0), Vec3::ZERO);
    for _ in 0..60 {
        step(&mut world);
    }
    let y = position(&world, p).y;
    assert!(y < -1.0, "the character stood on a trigger: y = {y:.2}");
    assert!(world.query::<&CharacterController>().get(p).unwrap().triggers.is_empty());
}
