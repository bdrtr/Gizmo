//! A joint dies with its body, even when the body's id is handed straight to a new one.
//!
//! Physics knows a body by its raw ECS id. Despawn a jointed body and spawn another before the
//! next step, and the new one gets the old id. It used to get the old body's row too, and with it
//! the old body's joints: a ball spawned 15 m away was pulled 5 m toward the dead ball's anchor in
//! a second.

use gizmo_core::World;
use gizmo_math::Vec3;
use gizmo_physics_core::{BodyHandle, Collider, Transform};
use gizmo_physics_rigid::components::{RigidBody, Velocity};
use gizmo_physics_rigid::joints::Joint;
use gizmo_physics_rigid::system::physics_step_system;
use gizmo_physics_rigid::world::PhysicsWorld;

const STEP: f32 = 1.0 / 60.0;

fn ball(world: &mut World, at: Vec3) -> u32 {
    let col = Collider::sphere(0.3);
    let mut rb = RigidBody::new(10.0, false);
    rb.update_inertia_from_collider(&col);
    world.spawn_bundle((Transform::new(at), rb, Velocity::default(), col)).id()
}

#[test]
fn a_new_body_on_a_recycled_id_is_not_tied_to_the_old_ones_joint() {
    let mut world = World::new();
    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::ZERO));
    let anchor = world
        .spawn_bundle((Transform::new(Vec3::ZERO), RigidBody::new_static(), Velocity::default(), Collider::sphere(0.1)))
        .id();
    let old = ball(&mut world, Vec3::new(0.0, -2.0, 0.0));
    world
        .get_resource_mut::<PhysicsWorld>()
        .unwrap()
        .joints
        .push(Joint::rope(BodyHandle::from_id(anchor), BodyHandle::from_id(old), Vec3::ZERO, Vec3::ZERO, 2.0));
    for _ in 0..10 {
        physics_step_system(&world, STEP);
    }

    let e = world.get_entity(old).unwrap();
    world.despawn(e);
    world.apply_commands();
    let start = Vec3::new(15.0, 0.0, 0.0);
    let new = ball(&mut world, start);
    assert_eq!(old, new, "the scenario needs the id to be recycled");
    for _ in 0..60 {
        physics_step_system(&world, STEP);
    }
    let at = world.borrow::<Transform>().get(new).unwrap().position;
    let joints = world.get_resource::<PhysicsWorld>().unwrap().joints.len();
    assert!(
        at.distance(start) < 1e-3 && joints == 0,
        "the new body moved {:.3} m and {joints} joint(s) are left (ids: old {old}, new {new})",
        at.distance(start)
    );
}
