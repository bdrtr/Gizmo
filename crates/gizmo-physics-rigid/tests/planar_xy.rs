//! `RigidBody::planar_xy` — the 2-D setup — keeps a body on its plane through contacts that push
//! it off: no drift along Z, no rotation except about Z.
//!
//! The probe is a box dropped onto a sphere it does not sit squarely over, offset along **Z**:
//! the contact normal then has a large Z component, so an unlocked box is shoved sideways off the
//! plane and tumbles about X. That is exactly the push a 2-D game must not see.
//!
//! **It did not hold until 2026-11-01.** Locks are velocity-level, and two paths moved the
//! transform past them: Heun's position step averages `pre_linear` with the current velocity, and
//! `pre_linear` is copied from the solver's output before locks run; and the contact solver's
//! positional correction is applied to the transform directly. Measured on this probe: 4·10⁻⁴ off
//! the plane with both leaks, 8·10⁻⁴ with only the correction masked, exactly 0 with both — so
//! the assertion is `== 0.0`, not a tolerance.

use gizmo_core::world::World;
use gizmo_math::Vec3;
use gizmo_physics_core::{Collider, Transform};
use gizmo_physics_rigid::components::{RigidBody, Velocity};
use gizmo_physics_rigid::world::PhysicsWorld;

/// Drops the box, returns its final `(position, rotation)`.
fn drop_box(body: RigidBody) -> (Vec3, gizmo_math::Quat) {
    let mut world = World::new();
    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));

    let ball = world.spawn();
    world.add_component(ball, Transform::new(Vec3::new(0.3, 0.0, 0.4)));
    world.add_component(ball, RigidBody::new_static());
    world.add_component(ball, Velocity::default());
    world.add_component(ball, Collider::sphere(0.5));

    let b = world.spawn();
    world.add_component(b, Transform::new(Vec3::new(0.0, 1.2, 0.0)));
    world.add_component(b, body);
    world.add_component(b, Velocity::default());
    world.add_component(b, Collider::box_collider(Vec3::splat(0.3)));

    for _ in 0..120 {
        gizmo_physics_rigid::system::physics_step_system(&world, 1.0 / 60.0);
    }
    let t = *world.borrow::<Transform>().get(b.id()).unwrap();
    (t.position, t.rotation)
}

#[test]
fn a_planar_body_stays_on_its_plane_and_turns_only_about_z() {
    // The probe has to push: unlocked, the box leaves the plane.
    let (free, _) = drop_box(RigidBody::new(1.0, true));
    assert!(free.z.abs() > 0.05, "the probe does not push off the plane (z = {}) — the test proves nothing", free.z);

    let (pos, rot) = drop_box(RigidBody::new(1.0, true).planar_xy());
    assert_eq!(pos.z, 0.0, "a planar body drifted off z = 0 to {}", pos.z);
    // Rotation only about Z: the body's own Z axis still points along world Z.
    let z_axis = rot * Vec3::Z;
    assert!((z_axis - Vec3::Z).length() < 1e-4, "a planar body tipped out of the plane: local Z is {z_axis:?}");
    assert!(pos.y < 1.2, "it did fall");
}

#[test]
fn planar_xy_sets_exactly_the_three_locks() {
    let b = RigidBody::new(1.0, true).planar_xy();
    assert!(b.lock_translation_z && b.lock_rotation_x && b.lock_rotation_y);
    assert!(!b.lock_translation_x && !b.lock_translation_y && !b.lock_rotation_z);
}
