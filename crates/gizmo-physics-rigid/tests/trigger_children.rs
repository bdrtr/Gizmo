//! A trigger collider under a body is a trigger, carried by the body.
//!
//! The compound walk merged every child collider's shape into its body's and kept the body's own
//! flags, so a trigger zone in a tray became part of the tray's solid shape: a ball dropped into it
//! came to rest on top of the zone, and no trigger event was ever raised (measured: the ball at
//! y 1.75 on a zone whose top is 1.5, zero events). Now a trigger under a body stays out of the solid
//! compound and is simulated as a trigger of its own where the body holds it, every step — and its
//! own body overlapping it is not an event.
use gizmo_core::component::{Children, Parent};
use gizmo_core::entity::Entity;
use gizmo_core::world::World;
use gizmo_math::{Quat, Vec3};
use gizmo_physics_core::{Collider, Transform};
use gizmo_physics_rigid::components::{KinematicTarget, RigidBody, Velocity};
use gizmo_physics_rigid::system::physics_step_system;
use gizmo_physics_rigid::world::PhysicsWorld;

fn world() -> World {
    let mut world = World::new();
    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));
    world
}

/// A kinematic tray (a 4 × 0.2 × 4 floor), driven to `at`, with a trigger zone parented at `zone_at`.
fn tray_with_zone(world: &mut World, at: Vec3, zone_at: Vec3) -> (Entity, Entity) {
    let tray = world.spawn();
    world.add_component(tray, Transform::new(Vec3::ZERO));
    world.add_component(tray, RigidBody::new_kinematic());
    world.add_component(tray, Velocity::default());
    world.add_component(tray, Collider::box_collider(Vec3::new(2.0, 0.1, 2.0)));
    world.add_component(tray, KinematicTarget::new(at, Quat::IDENTITY));
    let zone = world.spawn();
    world.add_component(zone, Transform::new(zone_at));
    world.add_component(zone, Collider::box_collider(Vec3::new(1.0, 0.5, 1.0)).with_trigger(true));
    world.add_component(zone, Parent(tray));
    world.add_component(tray, Children(vec![zone]));
    (tray, zone)
}

fn ball(world: &mut World, at: Vec3) -> Entity {
    let b = world.spawn();
    world.add_component(b, Transform::new(at));
    world.add_component(b, RigidBody::new(1.0, true));
    world.add_component(b, Velocity::default());
    world.add_component(b, Collider::sphere(0.25));
    b
}

/// Steps `n` times, collecting every trigger event as (trigger, other) ids.
fn run(world: &mut World, n: usize, events: &mut Vec<(u32, u32)>) {
    for _ in 0..n {
        world.tick += 1;
        physics_step_system(world, 1.0 / 60.0);
        let physics = world.get_resource::<PhysicsWorld>().unwrap();
        events.extend(physics.trigger_events.iter().map(|e| (e.trigger_entity.id(), e.other_entity.id())));
    }
}

fn y(world: &World, e: Entity) -> f32 {
    world.query::<&Transform>().get(e).unwrap().position.y
}

#[test]
fn a_trigger_under_a_body_is_a_trigger() {
    let mut world = world();
    let (tray, zone) = tray_with_zone(&mut world, Vec3::ZERO, Vec3::new(0.0, 1.0, 0.0));
    let b = ball(&mut world, Vec3::new(0.0, 4.0, 0.0));
    let mut events = Vec::new();
    run(&mut world, 180, &mut events);
    assert!((y(&world, b) - 0.35).abs() < 0.03, "the ball came to rest at y {} — on the zone, not through it to the tray", y(&world, b));
    assert!(events.iter().any(|&(t, o)| t == zone.id() && o == b.id()), "the ball passed through the zone and no trigger event named it: {events:?}");
    let _ = tray;
}

/// The trigger goes where its body goes: the tray is driven ten metres along, and a ball dropped
/// over it there enters the zone.
#[test]
fn a_carried_trigger_moves_with_its_body() {
    let mut world = world();
    let (_, zone) = tray_with_zone(&mut world, Vec3::new(10.0, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0));
    let mut events = Vec::new();
    run(&mut world, 10, &mut events);
    let b = ball(&mut world, Vec3::new(10.0, 4.0, 0.0));
    run(&mut world, 120, &mut events);
    assert!(events.iter().any(|&(t, o)| t == zone.id() && o == b.id()), "the zone stayed behind its tray: {events:?}");
}

/// A zone sunk into its own moving tray does not report the tray; it does report the ball.
#[test]
fn a_carried_trigger_does_not_report_its_own_body() {
    let mut world = world();
    let (tray, zone) = tray_with_zone(&mut world, Vec3::ZERO, Vec3::new(0.0, 0.3, 0.0));
    let b = ball(&mut world, Vec3::new(0.0, 3.0, 0.0));
    let mut events = Vec::new();
    for f in 0..120 {
        // Keep the tray moving, so the narrowphase tests it against the zone every step.
        let at = Vec3::new((f as f32 * 0.05).sin() * 0.5, 0.0, 0.0);
        *world.query_mut::<&mut KinematicTarget>().get_mut(tray).unwrap() = KinematicTarget::new(at, Quat::IDENTITY);
        run(&mut world, 1, &mut events);
    }
    assert!(
        !events.iter().any(|&(t, o)| (t == zone.id() && o == tray.id()) || (t == tray.id() && o == zone.id())),
        "the zone reported its own tray: {events:?}"
    );
    assert!(events.iter().any(|&(t, o)| t == zone.id() && o == b.id()), "and the ball in it went unreported");
}
