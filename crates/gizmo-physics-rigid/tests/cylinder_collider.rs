//! What a cylinder collider has to do that a capsule of the same numbers cannot.
//!
//! The shape exists for wheels, barrels and columns — things with **flat ends**. Every test here
//! is written so that it would fail if the narrowphase were quietly treating the shape as its
//! capsule (rounded ends, so it rests a radius higher and rocks instead of standing) or as its
//! bounding box (square, so it would not roll and its resting height would be wrong on its side).
//!
//! Driven through the real ECS path (`physics_step_system`), like `analytical.rs`, because the
//! question is what the solver ends up doing, not what one function returns.

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

/// A static floor whose top surface is at `y = 0`.
fn floor(world: &mut World) {
    let e = world.spawn();
    world.add_component(e, Transform::new(Vec3::new(0.0, -0.5, 0.0)));
    world.add_component(e, RigidBody::new_static());
    world.add_component(e, Velocity::default());
    world.add_component(e, Collider::box_collider(Vec3::new(50.0, 0.5, 50.0)));
}

/// High friction and no bounce: the shapes here are meant to settle, not to skate or hop.
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

fn drop_body(world: &mut World, collider: Collider, position: Vec3, rotation: Quat) -> u32 {
    let e = world.spawn();
    let mut t = Transform::new(position);
    t.rotation = rotation;
    let mut rb = RigidBody::new(2.0, true); // (mass, use_gravity)
    rb.update_inertia_from_collider(&collider);
    world.add_component(e, t);
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

fn transform(world: &World, id: u32) -> Transform {
    *world.borrow::<Transform>().get(id).unwrap()
}

/// A cylinder stands on its flat end at exactly its half-height. A capsule of the same numbers
/// rests a full radius higher — so this measurement is what tells the two apart at the contact,
/// and it is the reason the shape was added.
#[test]
fn a_cylinder_rests_on_its_flat_end_at_its_half_height() {
    let mut w = scene();
    floor(&mut w);
    let radius = 0.4;
    let half_height = 0.5;
    let mut collider = Collider::cylinder(radius, half_height);
    collider.material = grippy();
    let id = drop_body(&mut w, collider, Vec3::new(0.0, 1.2, 0.0), Quat::IDENTITY);

    step(&w, 720); // 3 s

    let t = transform(&w, id);
    assert!(
        (t.position.y - half_height).abs() < 0.02,
        "rests on its flat end at {half_height} m, got {}",
        t.position.y
    );
    assert!(
        t.position.y < half_height + radius - 0.1,
        "and NOT at the capsule's resting height ({}), which is what a rounded end would give",
        half_height + radius
    );

    // Still upright: its local +Y has not tipped away from world +Y.
    let up = t.rotation * Vec3::Y;
    assert!(
        up.dot(Vec3::Y) > 0.99,
        "a cylinder on its flat end stays standing; up = {up:?}"
    );
}

/// Laid on its side it rests on the wall of the tube, so its centre sits at exactly one radius.
/// A box of the same half-extents would rest at its half-height instead, and a capsule would
/// agree here — which is why the standing test above carries the discrimination and this one
/// carries the orientation.
#[test]
fn a_cylinder_on_its_side_rests_at_its_radius() {
    let mut w = scene();
    floor(&mut w);
    let radius = 0.35;
    let half_height = 0.6;
    let mut collider = Collider::cylinder(radius, half_height);
    collider.material = grippy();
    let lying = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
    let id = drop_body(&mut w, collider, Vec3::new(0.0, 1.0, 0.0), lying);

    step(&w, 720);

    let t = transform(&w, id);
    assert!(
        (t.position.y - radius).abs() < 0.02,
        "rests on the tube wall at {radius} m, got {}",
        t.position.y
    );
}

/// The tensor is the analytic one, and specifically not the capsule's.
///
/// `I_y = ½·m·r²` about the axis and `I_x = I_z = m·(3r² + h²)/12` across it. The capsule adds a
/// hemisphere at each end — the mass farthest from the axis — so its transverse inertia is
/// strictly larger for the same radius and half-height. Sharing one formula between the two
/// would make a wheel resist tipping like something it is not.
#[test]
fn the_cylinder_inertia_tensor_is_the_analytic_one() {
    let (m, r, half_h) = (3.0_f32, 0.4_f32, 0.9_f32);
    let h = half_h * 2.0;

    let mut rb = RigidBody::new(m, true);
    rb.update_inertia_from_collider(&Collider::cylinder(r, half_h));

    let expected_axial = 0.5 * m * r * r;
    let expected_transverse = m * (3.0 * r * r + h * h) / 12.0;
    assert!(
        (rb.local_inertia.y - expected_axial).abs() < 1e-5,
        "axial: expected {expected_axial}, got {}",
        rb.local_inertia.y
    );
    assert!(
        (rb.local_inertia.x - expected_transverse).abs() < 1e-5,
        "transverse: expected {expected_transverse}, got {}",
        rb.local_inertia.x
    );
    assert!(
        (rb.local_inertia.x - rb.local_inertia.z).abs() < 1e-6,
        "X and Z are the same across a body of revolution"
    );

    let mut capsule_rb = RigidBody::new(m, true);
    capsule_rb.update_inertia_from_collider(&Collider::capsule(r, half_h));
    assert!(
        capsule_rb.local_inertia.x > rb.local_inertia.x * 1.05,
        "the capsule's caps put mass further from the axis: {} vs {}",
        capsule_rb.local_inertia.x,
        rb.local_inertia.x
    );
}
