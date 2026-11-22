//! What a game sees of a break: a body that is not a body breaks too, its pieces come to rest on
//! their own shapes, a blast hurts only as far as it says, and the break is told.
//!
//! Each was found writing `cam`, a game of breaking glass, and each test is red on the code before:
//!
//! * the fracture pass asked for a `Velocity`, so a breakable standing as bare collider — a pane,
//!   a wall: static geometry — was immune to every hit;
//! * a piece's collider was a sphere of `(0.1 · volume)^(1/3)`, 42 % of its cell, which the drawn
//!   cell sank into and which rolled on forever: a crate's pieces were still rolling at 1.7 m/s
//!   after eight seconds;
//! * `Explosion::damage_radius` was read by nothing;
//! * nothing said what broke — the body was despawned by the same step.

use gizmo_core::event::Events;
use gizmo_core::world::World;
use gizmo_math::Vec3;
use gizmo_physics_core::{Collider, Transform};
use gizmo_physics_rigid::components::{BreakEvent, Breakable, Debris, Explosion, ExplosionFalloff, RigidBody, Velocity};
use gizmo_physics_rigid::system::{fracture_step, physics_explosion_system, physics_step_system};
use gizmo_physics_rigid::world::PhysicsWorld;

const DT: f32 = 1.0 / 60.0;

fn world_with_floor() -> World {
    let mut world = World::new();
    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));
    world.init_events::<BreakEvent>();
    let floor = world.spawn();
    world.add_component(floor, Transform::new(Vec3::new(0.0, -0.5, 0.0)));
    world.add_component(floor, Collider::box_collider(Vec3::new(20.0, 0.5, 20.0)));
    world
}

fn fragile() -> Breakable {
    Breakable { threshold: 1.0, current_health: 1.0, max_health: 1.0, debris_lifetime: 0.0, ..Default::default() }
}

fn throw_ball(world: &mut World, from: Vec3, velocity: Vec3, mass: f32) {
    let ball = world.spawn();
    world.add_component(ball, Transform::new(from));
    world.add_component(ball, RigidBody::new(mass, false));
    world.add_component(ball, Velocity::new(velocity));
    world.add_component(ball, Collider::sphere(0.2));
}

/// One step, with fracture, the commands applied, and the breaks it reported.
fn step(world: &mut World) -> Vec<BreakEvent> {
    physics_step_system(world, DT);
    let breaks = fracture_step(world, DT);
    world.apply_commands();
    breaks
}

/// A pane standing as bare collider — no `RigidBody`, no `Velocity` — breaks when a ball hits
/// it, and the break is told once, both as the step's answer and into `Events<BreakEvent>`.
#[test]
fn a_breakable_without_a_body_breaks_and_says_so() {
    let mut world = world_with_floor();
    let pane = world.spawn();
    world.add_component(pane, Transform::new(Vec3::new(0.0, 1.2, 0.0)));
    world.add_component(pane, Collider::box_collider(Vec3::new(1.0, 1.0, 0.05)));
    world.add_component(pane, Breakable { break_sound: Some("glass".into()), ..fragile() });
    throw_ball(&mut world, Vec3::new(0.0, 1.2, -3.0), Vec3::new(0.0, 0.0, 15.0), 1.0);
    let mut told = Vec::new();
    for _ in 0..60 {
        told.extend(step(&mut world));
    }
    assert!(!world.is_alive(pane), "the pane broke");
    let pieces = world.query::<&Debris>().iter().count();
    assert!(pieces > 1, "into pieces: {pieces}");
    assert_eq!(told.len(), 1, "one break, told once: {told:?}");
    let event = &told[0];
    assert_eq!((event.body, event.pieces as usize, event.sound.as_deref()), (pane, pieces, Some("glass")));
    assert!(event.position.distance(Vec3::new(0.0, 1.2, 0.0)) < 1e-4, "where the pane stood");
    assert!(event.impact_point.z.abs() < 0.2, "struck on its face: {:?}", event.impact_point);
    // A frame's events are read on the next frame: rotate once, as the frame loop does.
    world.update_events();
    let queued: Vec<BreakEvent> = world.get_resource_mut::<Events<BreakEvent>>().expect("registered").drain().collect();
    assert_eq!(queued, told, "the same break in the event queue");
}

/// A crate's pieces come to rest on their own shapes: still within seconds, near where it broke,
/// and their drawn cells on the floor, not in it.
#[test]
fn debris_comes_to_rest_on_its_own_shape() {
    let mut world = world_with_floor();
    let crate_ = world.spawn();
    world.add_component(crate_, Transform::new(Vec3::new(0.0, 0.5, 0.0)));
    world.add_component(crate_, Collider::box_collider(Vec3::new(1.0, 0.5, 0.1)));
    world.add_component(crate_, RigidBody::new(50.0, true));
    world.add_component(crate_, Velocity::default());
    world.add_component(crate_, fragile());
    throw_ball(&mut world, Vec3::new(0.0, 0.5, -3.0), Vec3::new(0.0, 0.0, 15.0), 1.0);
    for _ in 0..60 * 5 {
        step(&mut world);
    }
    assert!(!world.is_alive(crate_), "the crate broke");
    let pieces: Vec<(f32, f32, f32)> = world
        .query::<(&Transform, &Velocity, &Debris)>()
        .iter()
        .map(|(t, v, d)| {
            let lowest = d.vertices.iter().map(|p| (t.position + t.rotation * *p).y).fold(f32::MAX, f32::min);
            (v.linear.length(), lowest, Vec3::new(t.position.x, 0.0, t.position.z).length())
        })
        .collect();
    assert!(pieces.len() > 1);
    let fastest = pieces.iter().map(|p| p.0).fold(0.0, f32::max);
    let deepest = pieces.iter().map(|p| p.1).fold(f32::MAX, f32::min);
    let farthest = pieces.iter().map(|p| p.2).fold(0.0, f32::max);
    assert!(fastest < 0.1, "at rest after five seconds: the fastest piece moves at {fastest:.3} m/s");
    assert!(deepest > -0.025, "a drawn cell is on the floor, not in it: {deepest:.3} m");
    assert!(farthest < 5.0, "near where it broke: {farthest:.1} m");
}

/// A blast hurts as far as its `damage_radius` and pushes as far as its `force_radius`.
#[test]
fn a_blast_hurts_only_as_far_as_its_damage_radius() {
    let mut world = World::new();
    world.insert_resource(PhysicsWorld::new());
    let near = world.spawn();
    world.add_component(near, Transform::new(Vec3::new(1.0, 0.0, 0.0)));
    world.add_component(near, Collider::box_collider(Vec3::splat(0.3)));
    world.add_component(near, Breakable { threshold: 1.0, current_health: 50.0, max_health: 50.0, ..Default::default() });
    let far = world.spawn();
    world.add_component(far, Transform::new(Vec3::new(5.0, 0.0, 0.0)));
    world.add_component(far, Collider::box_collider(Vec3::splat(0.3)));
    world.add_component(far, Breakable { threshold: 1.0, current_health: 50.0, max_health: 50.0, ..Default::default() });
    let blast = world.spawn();
    world.add_component(
        blast,
        Explosion { force_radius: 10.0, force: 500.0, damage: 1000.0, damage_radius: 2.0, falloff: ExplosionFalloff::Linear, offset: Vec3::ZERO, is_active: true },
    );
    world.add_component(blast, Transform::new(Vec3::ZERO));
    physics_explosion_system(&world, DT);
    world.apply_commands();
    assert!(!world.is_alive(near), "inside the damage radius: broken");
    let health = world.query::<&Breakable>().get(far).map(|b| b.current_health);
    assert_eq!(health, Some(50.0), "pushed, but outside the damage radius: unhurt");
}
