//! A collider with no `RigidBody` is static geometry.
//!
//! As in every engine with the body/collider split (Unity's static collider, Rapier's collider with
//! no parent body). The editor's ➕ Collider adds just one, the collider overlay drew them as static,
//! and the character controller has always stood on them. The rigid world left them out: a ball
//! dropped on a floor that was only a collider fell straight through it (−40.8 m after three
//! seconds) while a character walked on the same floor.
//!
//! What is *not* static geometry stays out: a collider under a body is part of that body's
//! compound, and one with a `Velocity` is moved by something else (a character controller).
use gizmo_core::component::{Children, Parent};
use gizmo_core::entity::Entity;
use gizmo_core::world::World;
use gizmo_math::Vec3;
use gizmo_physics_core::{Collider, Transform};
use gizmo_physics_rigid::components::{RigidBody, Velocity};
use gizmo_physics_rigid::system::physics_step_system;
use gizmo_physics_rigid::world::PhysicsWorld;

fn world() -> World {
    let mut world = World::new();
    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));
    world
}

fn run(world: &mut World, steps: usize) {
    for _ in 0..steps {
        world.tick += 1;
        physics_step_system(world, 1.0 / 60.0);
    }
}

/// A floor that is a `Transform` and a `Collider` and nothing else.
fn floor(world: &mut World, at: Vec3) -> Entity {
    let e = world.spawn();
    world.add_component(e, Transform::new(at));
    world.add_component(e, Collider::box_collider(Vec3::new(5.0, 0.1, 5.0)));
    e
}

fn ball(world: &mut World, at: Vec3) -> Entity {
    let e = world.spawn();
    world.add_component(e, Transform::new(at));
    world.add_component(e, RigidBody::new(1.0, true));
    world.add_component(e, Velocity::default());
    world.add_component(e, Collider::sphere(0.25));
    e
}

fn y(world: &World, e: Entity) -> f32 {
    world.query::<&Transform>().get(e).unwrap().position.y
}

fn simulated(world: &World, e: Entity) -> bool {
    world.get_resource::<PhysicsWorld>().unwrap().entity_index_map.get(&e.id()).is_some()
}

#[test]
fn a_ball_rests_on_a_floor_that_is_only_a_collider() {
    let mut world = world();
    let f = floor(&mut world, Vec3::ZERO);
    let b = ball(&mut world, Vec3::new(0.0, 2.0, 0.0));
    run(&mut world, 180);
    assert!((y(&world, b) - 0.35).abs() < 0.03, "the ball is at y {}; the floor's top holds it at 0.35", y(&world, b));
    assert!(world.query::<&RigidBody>().get(f).is_none(), "the floor was given no RigidBody: it stays what it was authored as");
}

/// A trigger volume that is only a collider reports a body entering it, and does not stop it.
#[test]
fn a_trigger_that_is_only_a_collider_reports_what_enters_it() {
    let mut world = world();
    let zone = world.spawn();
    world.add_component(zone, Transform::new(Vec3::new(0.0, 1.0, 0.0)));
    world.add_component(zone, Collider::box_collider(Vec3::splat(0.5)).with_trigger(true));
    let b = ball(&mut world, Vec3::new(0.0, 3.0, 0.0));
    let mut saw_trigger = false;
    for _ in 0..90 {
        run(&mut world, 1);
        saw_trigger |= !world.get_resource::<PhysicsWorld>().unwrap().trigger_events.is_empty();
    }
    assert!(saw_trigger, "the ball fell through the zone and no TriggerEvent was raised");
    assert!(y(&world, b) < -1.0, "a trigger does not hold the ball: y {}", y(&world, b));
}

/// A collider grouped under a parent that is no body (an editor group) is simulated where the group
/// puts it, and goes with the group when the group moves.
#[test]
fn a_grouped_collider_is_simulated_where_its_group_puts_it() {
    let mut world = world();
    let group = world.spawn();
    world.add_component(group, Transform::new(Vec3::new(10.0, 0.0, 0.0)));
    let f = floor(&mut world, Vec3::new(0.0, -1.0, 0.0)); // local to the group
    world.add_component(f, Parent(group));
    world.add_component(group, Children(vec![f]));
    let b = ball(&mut world, Vec3::new(10.0, 2.0, 0.0));
    run(&mut world, 180);
    assert!((y(&world, b) - (-0.65)).abs() < 0.03, "on the grouped floor at y −0.9: y {}", y(&world, b));

    // The group moves away; its floor goes with it, and the ball falls.
    world.query_mut::<&mut Transform>().get_mut(group).unwrap().position.x = 30.0;
    run(&mut world, 60);
    assert!(y(&world, b) < -2.0, "the floor stayed behind its group: the ball is at y {}", y(&world, b));
}

/// A collider under a body is part of that body's compound, not a second, static body.
#[test]
fn a_collider_under_a_body_is_not_a_body_of_its_own() {
    let mut world = world();
    let body = ball(&mut world, Vec3::new(0.0, 5.0, 0.0));
    let part = world.spawn();
    world.add_component(part, Transform::new(Vec3::new(0.0, -0.5, 0.0)));
    world.add_component(part, Collider::sphere(0.2));
    world.add_component(part, Parent(body));
    // A second part listed under the body with no `Parent` of its own: the compound walk follows
    // `Children`, so it is a part as well.
    let listed = world.spawn();
    world.add_component(listed, Transform::new(Vec3::new(0.0, 0.5, 0.0)));
    world.add_component(listed, Collider::sphere(0.2));
    world.add_component(body, Children(vec![part, listed]));
    run(&mut world, 2);
    assert!(simulated(&world, body));
    assert!(!simulated(&world, part), "the body's part was simulated as a body of its own");
    assert!(!simulated(&world, listed), "a part listed only in the body's Children was simulated as a body of its own");
}

/// A collider with a `Velocity` and no body is moved by something else — a character controller —
/// and is not static geometry.
#[test]
fn a_collider_with_a_velocity_is_not_static_geometry() {
    let mut world = world();
    let walker = world.spawn();
    world.add_component(walker, Transform::new(Vec3::new(0.0, 1.0, 0.0)));
    world.add_component(walker, Velocity::default());
    world.add_component(walker, Collider::capsule(0.3, 0.6));
    let f = floor(&mut world, Vec3::ZERO);
    run(&mut world, 2);
    assert!(simulated(&world, f));
    assert!(!simulated(&world, walker), "a collider moved by its own velocity was made static geometry");
}

/// A static collider taken off leaves the world, and one added joins it.
#[test]
fn a_collider_taken_off_leaves_the_world() {
    let mut world = world();
    let f = floor(&mut world, Vec3::ZERO);
    run(&mut world, 2);
    assert!(simulated(&world, f));
    world.remove_component::<Collider>(f);
    run(&mut world, 1);
    assert!(!simulated(&world, f), "the floor's collider is gone and its body stayed");
}
