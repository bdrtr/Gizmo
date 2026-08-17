//! Terrain has to be concave, and that is the whole test.
//!
//! Any convex treatment of a heightfield — GJK against its support function, a convex hull of its
//! samples, its own AABB — produces the same wrong answer: a lid across every valley. A body
//! dropped into a dip lands on that lid, metres above the ground, and the symptom in a game is
//! "the car floats over the ditch". So the first test here drops a box into a valley and measures
//! where it comes to rest, against a floor that only exists if the shape is dispatched per cell.
//!
//! Driven through the real ECS path (`physics_step_system`), like `analytical.rs`.

use gizmo_core::world::World;
use gizmo_math::{Quat, Vec3};
use gizmo_physics_core::components::{CombineMode, PhysicsMaterial};
use gizmo_physics_core::{Collider, Transform};
use gizmo_physics_rigid::components::{RigidBody, Velocity};
use gizmo_physics_rigid::system::physics_step_system;
use gizmo_physics_rigid::world::PhysicsWorld;

const DT: f32 = 1.0 / 240.0;

fn scene() -> World {
    let mut w = World::new();
    w.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));
    w
}

fn grippy() -> PhysicsMaterial {
    PhysicsMaterial {
        restitution: 0.0,
        static_friction: 0.9,
        dynamic_friction: 0.8,
        friction_combine: CombineMode::Max,
        restitution_combine: CombineMode::Min,
        ..Default::default()
    }
}

/// A static terrain body at the origin.
fn terrain(world: &mut World, heights: Vec<f32>, rows: u32, cols: u32, scale: Vec3) {
    let e = world.spawn();
    let mut collider = Collider::heightfield(heights, rows, cols, scale);
    collider.material = grippy();
    world.add_component(e, Transform::new(Vec3::ZERO));
    world.add_component(e, RigidBody::new_static());
    world.add_component(e, Velocity::default());
    world.add_component(e, collider);
}

fn drop_box(world: &mut World, half: Vec3, position: Vec3) -> u32 {
    let e = world.spawn();
    let mut collider = Collider::box_collider(half);
    collider.material = grippy();
    let mut rb = RigidBody::new(1.0, true); // (mass, use_gravity)
    rb.update_inertia_from_collider(&collider);
    world.add_component(e, Transform::new(position));
    world.add_component(e, rb);
    world.add_component(e, Velocity::default());
    world.add_component(e, collider);
    e.id()
}

fn step(world: &World, n: usize) {
    for _ in 0..n {
        physics_step_system(world, DT);
    }
}

fn y_of(world: &World, id: u32) -> f32 {
    world.borrow::<Transform>().get(id).unwrap().position.y
}

/// A 5×5 field, 2 m cells, flat at height 4 except for a single sample pushed down to 0 in the
/// middle — a valley whose floor is 4 m below its rim.
fn valley() -> Vec<f32> {
    let mut heights = vec![4.0; 25];
    heights[2 * 5 + 2] = 0.0;
    heights
}

/// **The concavity test.** A box dropped into the dip has to come to rest *in* it. Treat the
/// terrain as convex — a hull, a support function, an AABB — and it rests on the rim's plane
/// instead, four metres up, which is the "my car floats over the ditch" bug.
#[test]
fn a_box_dropped_into_a_valley_rests_on_its_floor_not_on_a_lid() {
    let mut w = scene();
    terrain(&mut w, valley(), 5, 5, Vec3::new(2.0, 1.0, 2.0));
    let half = Vec3::splat(0.25);
    let id = drop_box(&mut w, half, Vec3::new(0.0, 6.0, 0.0));

    step(&w, 1200); // 5 s

    let y = y_of(&w, id);
    assert!(
        y < 4.0 - 0.5,
        "it stopped at {y}, which is the rim's height — the valley was treated as filled in"
    );
    assert!(
        y > -1.0,
        "and it did not fall through the surface: {y}"
    );
}

/// The other half of the same property: on flat terrain a box rests exactly on the surface, so
/// the per-cell dispatch is not merely letting things through.
#[test]
fn a_box_rests_on_flat_terrain_at_the_surface() {
    let mut w = scene();
    terrain(&mut w, vec![3.0; 25], 5, 5, Vec3::new(2.0, 1.0, 2.0));
    let half = Vec3::splat(0.5);
    let id = drop_box(&mut w, half, Vec3::new(0.0, 6.0, 0.0));

    step(&w, 960); // 4 s

    let y = y_of(&w, id);
    assert!(
        (y - (3.0 + half.y)).abs() < 0.1,
        "rests on the surface at {}, got {y}",
        3.0 + half.y
    );
}

/// Terrain is placed by moving the body, so the collision has to follow the transform. A field
/// tested in world space would put the ground back at the origin and let the box fall past it.
#[test]
fn terrain_collision_follows_the_bodys_transform() {
    let mut w = scene();
    let e = w.spawn();
    let mut collider = Collider::heightfield(vec![0.0; 25], 5, 5, Vec3::new(2.0, 1.0, 2.0));
    collider.material = grippy();
    let mut t = Transform::new(Vec3::new(0.0, 5.0, 0.0));
    t.rotation = Quat::IDENTITY;
    w.add_component(e, t);
    w.add_component(e, RigidBody::new_static());
    w.add_component(e, Velocity::default());
    w.add_component(e, collider);

    let half = Vec3::splat(0.5);
    let id = drop_box(&mut w, half, Vec3::new(0.0, 9.0, 0.0));
    step(&w, 960);

    let y = y_of(&w, id);
    assert!(
        (y - (5.0 + half.y)).abs() < 0.1,
        "the terrain sits at y = 5, so the box rests at {}, got {y}",
        5.0 + half.y
    );
}

/// A field with no cells collides with nothing — and, crucially, does not panic while being
/// asked to. Malformed terrain in a scene file must not take the physics step down with it.
#[test]
fn a_malformed_field_is_inert_rather_than_fatal() {
    let mut w = scene();
    terrain(&mut w, vec![0.0; 3], 5, 5, Vec3::new(2.0, 1.0, 2.0));
    let id = drop_box(&mut w, Vec3::splat(0.5), Vec3::new(0.0, 4.0, 0.0));

    step(&w, 240); // 1 s of free fall, and no panic

    let y = y_of(&w, id);
    assert!(y < 0.0, "nothing stopped it, so it is still falling: {y}");
}
