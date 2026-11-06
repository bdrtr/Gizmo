//! A joint dies with its body, even when the body's id is handed straight to a new one.
//!
//! Physics knows a body by its raw ECS id. Despawn a jointed body and spawn another before the
//! next step, and the new one gets the old id. It used to get the old body's row too, and with it
//! the old body's joints: a ball spawned 15 m away was pulled 5 m toward the dead ball's anchor in
//! a second.

use gizmo_core::Entity;
use gizmo_core::World;
use gizmo_math::Vec3;
use gizmo_physics_core::{BodyHandle, Collider, Transform};
use gizmo_physics_rigid::components::{RigidBody, Velocity};
use gizmo_physics_rigid::joints::Joint;
use gizmo_physics_rigid::system::physics_step_system;
use gizmo_physics_rigid::world::PhysicsWorld;

const STEP: f32 = 1.0 / 60.0;

fn ball(world: &mut World, at: Vec3) -> Entity {
    let col = Collider::sphere(0.3);
    let mut rb = RigidBody::new(10.0, false);
    rb.update_inertia_from_collider(&col);
    world.spawn_bundle((Transform::new(at), rb, Velocity::default(), col))
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
        .push(Joint::rope(BodyHandle::from_id(anchor), BodyHandle::from_id(old.id()), Vec3::ZERO, Vec3::ZERO, 2.0));
    for _ in 0..10 {
        physics_step_system(&world, STEP);
    }

    let e = world.live(old).unwrap();
    world.despawn(e);
    world.apply_commands();
    let start = Vec3::new(15.0, 0.0, 0.0);
    let new = ball(&mut world, start);
    assert_eq!(old.id(), new.id(), "the scenario needs the id to be recycled");
    for _ in 0..60 {
        physics_step_system(&world, STEP);
    }
    let at = world.query::<&Transform>().get(new).unwrap().position;
    let joints = world.get_resource::<PhysicsWorld>().unwrap().joints.len();
    assert!(
        at.distance(start) < 1e-3 && joints == 0,
        "the new body moved {:.3} m and {joints} joint(s) are left (ids: old {old}, new {new})",
        at.distance(start)
    );
}

/// A body that comes into a trigger on a dead body's id is entering it, not staying in it.
///
/// The contact cache is keyed by the pair's raw ids, and it decides `Started` against `Persisting`
/// (and carries warm-start impulses for solid contacts). A body despawned inside a trigger and a
/// new one spawned there on the same id used to be told it had been there all along — the game's
/// "on enter" never ran for it.
#[test]
fn a_new_body_on_a_recycled_id_enters_a_trigger_it_is_born_in() {
    use gizmo_physics_core::collision::CollisionEventType;
    let mut world = World::new();
    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::ZERO));
    let zone = world
        .spawn_bundle((Transform::new(Vec3::ZERO), RigidBody::new_static(), Velocity::default(), Collider::sphere(3.0).with_trigger(true)))
        .id();
    let old = ball(&mut world, Vec3::ZERO);
    // Moving, so it stays awake: a sleeping pair drops out of the cache on its own.
    world.query_mut::<&mut Velocity>().get_mut(old).unwrap().linear = Vec3::new(0.3, 0.0, 0.0);
    for _ in 0..5 {
        physics_step_system(&world, STEP);
    }
    let e = world.live(old).unwrap();
    world.despawn(e);
    world.apply_commands();
    let new = ball(&mut world, Vec3::new(-0.5, 0.0, 0.0));
    assert_eq!(old.id(), new.id(), "the scenario needs the id to be recycled");
    physics_step_system(&world, STEP);
    let events: Vec<CollisionEventType> = world
        .get_resource::<PhysicsWorld>()
        .unwrap()
        .trigger_events()
        .iter()
        .filter(|t| [t.trigger_entity.id(), t.other_entity.id()].contains(&zone) && [t.trigger_entity.id(), t.other_entity.id()].contains(&new.id()))
        .map(|t| t.event_type)
        .collect();
    // The dead body leaves, then the new one comes in — in that order, on the shared id.
    let (ended, started) = (
        events.iter().position(|e| *e == CollisionEventType::Ended),
        events.iter().position(|e| *e == CollisionEventType::Started),
    );
    assert!(
        ended.is_some() && started.is_some() && ended < started,
        "the dead body's Ended, then the new body's Started: {events:?}"
    );
}

/// A body despawned inside a trigger is reported leaving it.
#[test]
fn a_body_despawned_in_a_trigger_ends_its_overlap() {
    use gizmo_physics_core::collision::CollisionEventType;
    let mut world = World::new();
    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::ZERO));
    let zone = world
        .spawn_bundle((Transform::new(Vec3::ZERO), RigidBody::new_static(), Velocity::default(), Collider::sphere(3.0).with_trigger(true)))
        .id();
    let visitor = ball(&mut world, Vec3::ZERO);
    world.query_mut::<&mut Velocity>().get_mut(visitor).unwrap().linear = Vec3::new(0.3, 0.0, 0.0);
    for _ in 0..5 {
        physics_step_system(&world, STEP);
    }
    let e = world.live(visitor).unwrap();
    world.despawn(e);
    world.apply_commands();
    physics_step_system(&world, STEP);
    let ended = world
        .get_resource::<PhysicsWorld>()
        .unwrap()
        .trigger_events()
        .iter()
        .filter(|t| t.event_type == CollisionEventType::Ended)
        .filter(|t| [t.trigger_entity.id(), t.other_entity.id()].contains(&zone) && [t.trigger_entity.id(), t.other_entity.id()].contains(&visitor.id()))
        .count();
    assert_eq!(ended, 1, "the zone was not told its visitor left");
}
