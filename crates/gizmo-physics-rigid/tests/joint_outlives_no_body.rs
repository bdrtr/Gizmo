//! **A joint does not outlive its body.** A joint names its bodies by raw id, and the next body
//! to recycle a despawned body's id used to be tied to the dead body's partner.

use gizmo_core::world::World;
use gizmo_math::Vec3;
use gizmo_physics_core::{BodyHandle, Collider, Transform};
use gizmo_physics_rigid::components::{RigidBody, Velocity};
use gizmo_physics_rigid::joints::Joint;
use gizmo_physics_rigid::world::PhysicsWorld;

fn step(world: &mut World, n: usize) {
    for _ in 0..n {
        gizmo_physics_rigid::system::physics_step_system(world, 1.0 / 60.0);
        world.apply_commands();
    }
}

fn body(world: &mut World, x: f32) -> gizmo_core::entity::Entity {
    let e = world.spawn();
    world.add_component(e, Transform::new(Vec3::new(x, 0.0, 0.0)));
    world.add_component(e, RigidBody::new(1.0, true));
    world.add_component(e, Velocity::default());
    world.add_component(e, Collider::sphere(0.5));
    e
}

/// A and B held 5 m apart by a rod; A despawned; C spawned 15 m from B takes A's id. C must stay
/// where it was put, and the joint must be gone.
#[test]
fn a_body_that_recycles_a_dead_bodys_id_is_not_tied_to_its_partner() {
    let mut world = World::new();
    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::ZERO));
    let a = body(&mut world, 0.0);
    let b = body(&mut world, 5.0);
    step(&mut world, 1);
    world.get_resource_mut::<PhysicsWorld>().unwrap().joints.push(Joint::distance(
        BodyHandle::from_id(a.id()),
        BodyHandle::from_id(b.id()),
        Vec3::ZERO,
        Vec3::ZERO,
        5.0,
        5.0,
    ));
    step(&mut world, 10);
    world.despawn(a);
    step(&mut world, 2);
    assert!(world.get_resource::<PhysicsWorld>().unwrap().joints.is_empty(), "the joint went with its body");

    let c = body(&mut world, 20.0);
    assert_eq!(c.id(), a.id(), "the test needs the id recycled");
    step(&mut world, 60);
    let x = world.query::<&Transform>().get(c).unwrap().position.x;
    assert!((x - 20.0).abs() < 1e-3, "the new body was pulled by the dead one's joint: x = {x}");
}
