//! Where a body's child colliders are simulated: where the render draws them.
//!
//! A child's `Transform` is local to its parent — `TransformPropagateSystem` draws it at
//! `parent · local`, and the editor writes children that way. The physics bridge read it as a world
//! pose and subtracted the body's position from it, so a child's shape was simulated wherever that
//! arithmetic put it: under a body at (10, 3, 0), a floor parented one metre down was ten metres
//! off, and a ball dropped onto it fell through; as a compound body moved, its children's shapes
//! slid the other way.
use gizmo_core::component::{Children, Parent};
use gizmo_core::entity::Entity;
use gizmo_core::world::World;
use gizmo_math::{Quat, Vec3};
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

fn spawn(world: &mut World, transform: Transform, rb: RigidBody, collider: Collider) -> Entity {
    let e = world.spawn();
    world.add_component(e, transform);
    world.add_component(e, rb);
    world.add_component(e, Velocity::default());
    world.add_component(e, collider);
    e
}

/// A child collider at `local` under `parent`, linked both ways as `HierarchyExt::add_child` links.
fn child(world: &mut World, parent: Entity, local: Transform, collider: Collider) -> Entity {
    let c = world.spawn();
    world.add_component(c, local);
    world.add_component(c, collider);
    world.add_component(c, Parent(parent));
    world.add_component(parent, Children(vec![c]));
    c
}

fn ball(world: &mut World, at: Vec3) -> Entity {
    spawn(world, Transform::new(at), RigidBody::new(1.0, true), Collider::sphere(0.25))
}

fn y(world: &World, e: Entity) -> f32 {
    world.query::<&Transform>().get(e).unwrap().position.y
}

/// A floor parented one metre under a static body at (10, 3, 0) holds a ball dropped onto it.
#[test]
fn a_child_collider_is_where_its_parent_puts_it() {
    let mut world = world();
    let body = spawn(&mut world, Transform::new(Vec3::new(10.0, 3.0, 0.0)), RigidBody::new_static(), Collider::sphere(0.1));
    child(&mut world, body, Transform::new(Vec3::new(0.0, -1.0, 0.0)), Collider::box_collider(Vec3::new(2.0, 0.1, 2.0)));
    let b = ball(&mut world, Vec3::new(10.5, 6.0, 0.5));
    run(&mut world, 180);
    // The floor's top is at 3 − 1 + 0.1.
    assert!((y(&world, b) - 2.35).abs() < 0.03, "the ball is at y {}, the floor holds it at 2.35", y(&world, b));
}

/// The parent's rotation turns its child's offset, and its scale stretches it — and the child's
/// shape — as the render does (`scaled_children.rs` for the shapes).
#[test]
fn a_turned_and_scaled_parent_places_its_child_as_the_render_does() {
    let mut world = world();
    let turned = Transform::new(Vec3::ZERO)
        .with_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2))
        .with_scale(Vec3::splat(2.0));
    let body = spawn(&mut world, turned, RigidBody::new_static(), Collider::sphere(0.1));
    // Local (1, −1, 0) → scaled (2, −2, 0) → turned a quarter about +Y → world (0, −2, −2).
    child(&mut world, body, Transform::new(Vec3::new(1.0, -1.0, 0.0)), Collider::box_collider(Vec3::new(0.5, 0.1, 0.5)));
    let held = ball(&mut world, Vec3::new(0.0, 1.0, -2.0));
    let unturned = ball(&mut world, Vec3::new(2.0, 1.0, 0.0));
    run(&mut world, 180);
    // The box, twice its written size under the 2× parent, has its top at −2 + 0.2.
    assert!((y(&world, held) - (-1.55)).abs() < 0.03, "over (0, −2, −2): y {}", y(&world, held));
    assert!(y(&world, unturned) < -5.0, "nothing is at (2, −2, 0), where an unturned offset would be: y {}", y(&world, unturned));
}

/// A moving compound body keeps its children's shapes with it: a small box with a wide base
/// parented half a metre under it, dropped ten metres from the origin, lands on the base.
#[test]
fn a_falling_compound_body_lands_on_its_child() {
    let mut world = world();
    spawn(&mut world, Transform::new(Vec3::new(0.0, -0.5, 0.0)), RigidBody::new_static(), Collider::box_collider(Vec3::new(20.0, 0.5, 20.0)));
    let body = spawn(&mut world, Transform::new(Vec3::new(10.0, 3.0, 0.0)), RigidBody::new(2.0, true), Collider::box_collider(Vec3::splat(0.2)));
    child(&mut world, body, Transform::new(Vec3::new(0.0, -0.5, 0.0)), Collider::box_collider(Vec3::new(1.0, 0.1, 1.0)));
    run(&mut world, 240);
    // The base rests on the ground (centre at 0.1), the body half a metre above it.
    let p = world.query::<&Transform>().get(body).unwrap().position;
    assert!((p.y - 0.6).abs() < 0.03 && (p.x - 10.0).abs() < 0.05, "the body came to rest at {p:?}");
}
