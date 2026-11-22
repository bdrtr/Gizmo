//! A collider under a scaled parent takes the parent's scale, as its mesh does.
//!
//! The rule, the render's: an entity's own `Transform::scale` is its mesh's and stays out of its
//! collider — a root body's collider is written in metres beside a scaled mesh, everywhere in the
//! engine — and the scales of its ancestors apply to both. The offsets already followed them; the
//! shapes did not, so a child collider written like its mesh came out the mesh's size divided by
//! the scale above it. Each test drops a ball where the scaled shape is and the unscaled one is
//! not: red on the old code, where the ball fell past it.

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

fn under(world: &mut World, parent: Entity, local: Transform, collider: Collider) -> Entity {
    let c = world.spawn();
    world.add_component(c, local);
    world.add_component(c, collider);
    world.add_component(c, Parent(parent));
    world.add_component(parent, Children(vec![c]));
    c
}

/// A ball at x = 2.5, above a slab whose collider is ±1 in its own frame under a parent scaled 3×
/// in x and z: the scaled slab reaches ±3, the unscaled one ±1.
fn ball(world: &mut World) -> Entity {
    let b = world.spawn();
    world.add_component(b, Transform::new(Vec3::new(2.5, 2.0, 0.0)));
    world.add_component(b, RigidBody::new(1.0, true));
    world.add_component(b, Velocity::default());
    world.add_component(b, Collider::sphere(0.25));
    b
}

fn y(world: &World, e: Entity) -> f32 {
    world.query::<&Transform>().get(e).unwrap().position.y
}

const SCALE: Vec3 = Vec3::new(3.0, 1.0, 3.0);

/// A slab under a body scaled 3×: part of the body's compound, at the body's scale.
#[test]
fn a_childs_collider_under_a_scaled_body_takes_its_scale() {
    let mut world = world();
    let body = world.spawn();
    world.add_component(body, Transform::new(Vec3::ZERO).with_scale(SCALE));
    world.add_component(body, RigidBody::new_static());
    world.add_component(body, Velocity::default());
    world.add_component(body, Collider::sphere(0.1));
    under(&mut world, body, Transform::default(), Collider::box_collider(Vec3::new(1.0, 0.1, 1.0)));
    let b = ball(&mut world);
    run(&mut world, 120);
    assert!((y(&world, b) - 0.35).abs() < 0.05, "the ball rests on the scaled slab (top 0.1): y = {}", y(&world, b));
}

/// A slab under a bare, scaled parent: static geometry, at the parent's scale.
#[test]
fn a_static_collider_under_a_scaled_parent_takes_its_scale() {
    let mut world = world();
    let parent = world.spawn();
    world.add_component(parent, Transform::new(Vec3::ZERO).with_scale(SCALE));
    under(&mut world, parent, Transform::default(), Collider::box_collider(Vec3::new(1.0, 0.1, 1.0)));
    let b = ball(&mut world);
    run(&mut world, 120);
    assert!((y(&world, b) - 0.35).abs() < 0.05, "the ball rests on the scaled slab: y = {}", y(&world, b));
}

/// A trigger under a body scaled 3×: entered where the scaled zone is.
#[test]
fn a_trigger_under_a_scaled_body_takes_its_scale() {
    let mut world = world();
    let body = world.spawn();
    world.add_component(body, Transform::new(Vec3::ZERO).with_scale(SCALE));
    world.add_component(body, RigidBody::new_static());
    world.add_component(body, Velocity::default());
    world.add_component(body, Collider::sphere(0.1));
    let zone = under(&mut world, body, Transform::default(), Collider::box_collider(Vec3::new(1.0, 1.0, 1.0)).with_trigger(true));
    let b = ball(&mut world);
    let mut entered = false;
    for _ in 0..60 {
        run(&mut world, 1);
        let pw = world.get_resource::<PhysicsWorld>().unwrap();
        entered |= pw.trigger_events().iter().any(|e| e.trigger_entity.id() == zone.id() && e.other_entity.id() == b.id());
    }
    assert!(entered, "the ball falls into the scaled zone (to ±3 in x), which the unscaled one (±1) misses");
}
