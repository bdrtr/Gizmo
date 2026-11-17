//! Terrain has to be concave, and that is the whole test.
//!
//! Any convex treatment of a heightfield — GJK against its support function, a convex hull of its
//! samples, its own AABB — produces the same wrong answer: a lid across every valley. A body
//! dropped into a dip lands on that lid, metres above the ground, and the symptom in a game is
//! "the car floats over the ditch". So the first test here drops a box into a valley and measures
//! where it comes to rest, against a floor that only exists if the shape is dispatched per cell.
//!
//! Driven through the real ECS path (`physics_step_system`), like `analytical.rs`.

use gizmo_core::Entity;
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

fn drop_box(world: &mut World, half: Vec3, position: Vec3) -> Entity {
    let e = world.spawn();
    let mut collider = Collider::box_collider(half);
    collider.material = grippy();
    let mut rb = RigidBody::new(1.0, true); // (mass, use_gravity)
    rb.update_inertia_from_collider(&collider);
    world.add_component(e, Transform::new(position));
    world.add_component(e, rb);
    world.add_component(e, Velocity::default());
    world.add_component(e, collider);
    e
}

fn step(world: &World, n: usize) {
    for _ in 0..n {
        physics_step_system(world, DT);
    }
}

fn y_of(world: &World, id: Entity) -> f32 {
    world.query::<&Transform>().get(id).unwrap().position.y
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

/// **A CCD body falling onto terrain lands on it.** A CCD body a step above the ground and not yet
/// touching it gets a speculative contact, and that path handed the whole heightfield to GJK's
/// `support_point` — which a heightfield does not have: an error logged and a zero support used
/// on every such step (a golf ball rolling on its course: about once a physics step), and in a
/// debug build a panic on the first one. It now tests the field's triangles within reach, as the
/// touching path does. Measured where it was found: `golf`, 2026-11-10.
#[test]
fn a_ccd_ball_falling_onto_terrain_lands_on_it() {
    let mut w = scene();
    terrain(&mut w, vec![0.0; 49], 7, 7, Vec3::ONE);
    let e = w.spawn();
    let mut collider = Collider::sphere(0.3);
    collider.material = grippy();
    let mut rb = RigidBody::new(1.0, true);
    rb.calculate_sphere_inertia(0.3);
    rb.ccd_enabled = true;
    w.add_component(e, Transform::new(Vec3::new(0.4, 6.0, -0.3)));
    w.add_component(e, rb);
    w.add_component(e, Velocity::new(Vec3::new(0.0, -20.0, 0.0)));
    w.add_component(e, collider);
    let mut lowest = f32::MAX;
    for _ in 0..480 {
        step(&w, 1);
        lowest = lowest.min(y_of(&w, e));
    }
    assert!(lowest > 0.2, "the ball went {:.3} m deep into the ground (its centre reached {lowest:.3})", 0.3 - lowest);
    let y = y_of(&w, e);
    assert!((y - 0.3).abs() < 0.03, "the ball is not resting on the ground: centre at {y:.3}");
}

/// Friction 0.1 on both sides of a pair.
fn slippery() -> PhysicsMaterial {
    PhysicsMaterial { static_friction: 0.1, dynamic_friction: 0.1, friction_combine: CombineMode::Min, ..grippy() }
}

/// A flat, slippery static field of 11×11 samples, 1 m cells, at `at` (it spans ±5 m around it).
fn flat_field_at(world: &mut World, at: Vec3) {
    let e = world.spawn();
    let mut collider = Collider::heightfield(vec![0.0; 121], 11, 11, Vec3::ONE);
    collider.material = slippery();
    world.add_component(e, Transform::new(at));
    world.add_component(e, RigidBody::new_static());
    world.add_component(e, Velocity::default());
    world.add_component(e, collider);
}

/// **A body crossing from one terrain field to the next is neither stopped nor thrown.** A world
/// is tiled out of fields, and the border triangles of each pushed a body at the seam sideways
/// and along its own axis: a box sliding across stopped with its nose exactly on the seam, as at
/// a wall, and in `dunya` a parked car resting across one took impulses of 8 000–21 000 N·s and,
/// in half the runs, fell through the ground.
#[test]
fn a_box_sliding_across_the_seam_of_two_fields_is_neither_stopped_nor_thrown() {
    let mut world = scene();
    flat_field_at(&mut world, Vec3::new(0.0, 0.0, -5.0));
    flat_field_at(&mut world, Vec3::new(0.0, 0.0, 5.0));
    // A car-sized box, settled on the near field, then sent sliding across the seam at z = 0 on
    // slippery ground (it would slide 8 m), so that only the seam could stop it.
    let b = drop_box(&mut world, Vec3::new(0.9, 0.3, 2.0), Vec3::new(0.0, 0.31, -3.0));
    world.query_mut::<&mut Collider>().get_mut(b).unwrap().material = slippery();
    step(&world, 240);
    {
        let mut q = world.query_mut::<(&mut RigidBody, &mut Velocity)>();
        let (mut rb, mut v) = q.get_mut(b).unwrap();
        rb.wake_up();
        v.linear = Vec3::new(0.0, 0.0, 4.0);
    }
    let (mut fastest_up, mut lowest, mut farthest) = (0.0f32, f32::MAX, f32::MIN);
    for _ in 0..480 {
        step(&world, 1);
        let v = world.query::<&Velocity>().get(b).unwrap().linear;
        fastest_up = fastest_up.max(v.y.abs());
        lowest = lowest.min(y_of(&world, b));
        farthest = farthest.max(world.query::<&Transform>().get(b).unwrap().position.z);
    }
    assert!(farthest > 2.5, "stopped at the seam: its centre got to z = {farthest}, its nose to {}", farthest + 2.0);
    assert!(fastest_up < 1.0, "thrown at the seam: |vy| reached {fastest_up} m/s");
    assert!(lowest > 0.1, "sank into the ground: its centre went down to {lowest} m");
}

/// **A body sunk into the ground comes up out of it.** A tilted box placed 0.3 m into a flat
/// field — a spawn a little too low, a body let go where its ground has just loaded — is pushed
/// up, along the ground's normal, not through to the other side.
#[test]
fn a_tilted_box_sunk_into_the_ground_comes_up_out_of_it() {
    let mut world = scene();
    flat_field_at(&mut world, Vec3::ZERO);
    let b = drop_box(&mut world, Vec3::new(0.9, 0.3, 2.0), Vec3::new(0.0, 0.5, 0.0));
    world.query_mut::<&mut Transform>().get_mut(b).unwrap().rotation = Quat::from_rotation_x(0.35);
    step(&world, 480);
    let y = y_of(&world, b);
    assert!(y > 0.25 && y < 1.0, "it should rest on the ground, centre ~0.3 m up: {y}");
}

/// **A fast body crossing the seam of two fields is not stopped by the one it is not over.** A CCD
/// body near the ground is held by speculative contacts, and the field beside it offered one
/// against its border edge — sideways, a wall at the seam.
#[test]
fn a_ccd_ball_rolling_across_the_seam_of_two_fields_is_not_stopped() {
    let mut world = scene();
    flat_field_at(&mut world, Vec3::new(0.0, 0.0, -5.0));
    flat_field_at(&mut world, Vec3::new(0.0, 0.0, 5.0));
    let e = world.spawn();
    let mut collider = Collider::sphere(0.3);
    collider.material = slippery();
    let mut rb = RigidBody::new(1.0, true);
    rb.calculate_sphere_inertia(0.3);
    rb.ccd_enabled = true;
    world.add_component(e, Transform::new(Vec3::new(0.0, 0.31, -3.0)));
    world.add_component(e, rb);
    world.add_component(e, Velocity::default());
    world.add_component(e, collider);
    step(&world, 240);
    {
        let mut q = world.query_mut::<(&mut RigidBody, &mut Velocity)>();
        let (mut rb, mut v) = q.get_mut(e).unwrap();
        rb.wake_up();
        v.linear = Vec3::new(0.0, 0.0, 8.0);
    }
    let mut farthest = f32::MIN;
    let mut highest = f32::MIN;
    for _ in 0..240 {
        step(&world, 1);
        let p = world.query::<&Transform>().get(e).unwrap().position;
        farthest = farthest.max(p.z);
        highest = highest.max(p.y);
    }
    assert!(farthest > 3.0, "stopped at the seam: got to z = {farthest}");
    assert!(highest < 0.5, "thrown up at the seam: centre rose to {highest}");
}
