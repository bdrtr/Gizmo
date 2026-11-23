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

/// A broken body lets go of what it held. A leg parented under a breakable lander — part of its
/// compound — becomes a body of its own where the lander held it, ageing as debris; a bare visual
/// under the lander goes with it.
///
/// Red on the code before: the break despawned the lander alone, and despawning a parent unparents
/// its children, so the leg became static geometry at its LOCAL position (0.8, −0.8, 0) read as a
/// world one — beside the origin, half in the ground — and the visual stayed where the lander was.
#[test]
fn a_broken_body_lets_go_of_its_parts_where_it_held_them() {
    use gizmo_core::component::{Children, Parent};
    let mut world = world_with_floor();
    let lander = world.spawn();
    world.add_component(lander, Transform::new(Vec3::new(4.0, 3.0, 0.0)));
    world.add_component(lander, RigidBody::new(10.0, true));
    world.add_component(lander, Velocity::new(Vec3::new(0.0, -15.0, 0.0)));
    world.add_component(lander, Collider::box_collider(Vec3::new(1.0, 0.4, 0.5)));
    world.add_component(lander, Breakable { debris_lifetime: 7.0, ..fragile() });
    let leg = world.spawn();
    world.add_component(leg, Transform::new(Vec3::new(0.8, -0.8, 0.0)));
    world.add_component(leg, Collider::box_collider(Vec3::new(0.1, 0.4, 0.1)));
    world.add_component(leg, Parent(lander));
    let visual = world.spawn();
    world.add_component(visual, Transform::new(Vec3::new(0.0, 0.5, 0.0)));
    world.add_component(visual, Parent(lander));
    world.add_component(lander, Children(vec![leg, visual]));

    let mut held_at = None;
    for _ in 0..60 {
        let lander_at = world.query::<&Transform>().get(lander).map(|t| (t.position, t.rotation));
        if !step(&mut world).is_empty() {
            let (p, r) = lander_at.expect("the lander stood until this step");
            held_at = Some(p + r * Vec3::new(0.8, -0.8, 0.0));
            break;
        }
    }
    let held_at = held_at.expect("the lander broke on the floor");
    assert!(!world.is_alive(lander), "the lander is gone");
    assert!(!world.is_alive(visual), "and its visual with it");
    assert!(world.is_alive(leg), "the leg broke loose");
    assert!(world.query::<&Parent>().get(leg).is_none(), "on its own");
    assert!(world.query::<&RigidBody>().get(leg).is_some_and(|b| b.mass > 0.0), "a body of its own");
    assert_eq!(world.query::<&Debris>().get(leg).map(|d| d.lifetime), Some(7.0), "ageing as the lander's debris");
    let at = world.query::<&Transform>().get(leg).map(|t| t.position).expect("placed");
    assert!(at.distance(held_at) < 0.3, "where the lander held it, {held_at}, not at {at}");
}

/// What breaks in a plane breaks in it: a planar body's debris and the parts it lets go of keep its
/// axis locks, and do not leave the slab the body was (its pieces are cut through its depth, ±0.5
/// here, and each keeps its own). Measured in `inis` on the code before: a lander's legs ended 1.7 m
/// off the plane of a side-on game, and its pieces were spawned free.
#[test]
fn a_planar_bodys_pieces_stay_in_its_plane() {
    use gizmo_core::component::{Children, Parent};
    let mut world = world_with_floor();
    let lander = world.spawn();
    world.add_component(lander, Transform::new(Vec3::new(0.0, 3.0, 0.0)));
    world.add_component(lander, RigidBody::new(10.0, true).planar_xy());
    world.add_component(lander, Velocity::new(Vec3::new(1.0, -15.0, 0.0)));
    world.add_component(lander, Collider::box_collider(Vec3::new(1.0, 0.4, 0.5)));
    world.add_component(lander, fragile());
    let leg = world.spawn();
    world.add_component(leg, Transform::new(Vec3::new(0.8, -0.8, 0.0)));
    world.add_component(leg, Collider::box_collider(Vec3::new(0.1, 0.4, 0.1)));
    world.add_component(leg, Parent(lander));
    world.add_component(lander, Children(vec![leg]));
    let mut broke = false;
    for _ in 0..180 {
        broke |= !step(&mut world).is_empty();
    }
    assert!(broke, "the lander broke");
    let pieces = world.query::<(gizmo_core::entity::Entity, &Debris, &RigidBody, &Transform, &Velocity)>();
    let mut seen = 0;
    for (e, _, body, t, v) in pieces.iter() {
        seen += 1;
        assert!(body.lock_translation_z && body.lock_rotation_x && body.lock_rotation_y, "{e:?} kept the lander's locks");
        // Not `== 0.0`: macOS's arm64 left −2.4·10⁻¹⁵ m/s there (CI, 2026-11-23) where x86 leaves
        // none — rounding, not a body leaving its plane. Same-platform determinism is the contract.
        assert!(v.linear.z.abs() < 1e-9 && t.position.z.abs() <= 0.5 + 1e-3, "{e:?} stayed in the slab: z = {}, vz = {}", t.position.z, v.linear.z);
    }
    let leg_z = world.query::<&Transform>().get(leg).map(|t| t.position.z).expect("the leg");
    assert!(leg_z.abs() < 1e-6, "the leg, cut from nowhere, is on the plane itself: z = {leg_z}");
    assert!(seen > 1, "the debris and the leg were looked at: {seen}");
}

/// A blast's break lets go of the parts too: the explosion path shatters through the same helper.
#[test]
fn a_blast_lets_go_of_a_bodys_parts() {
    use gizmo_core::component::{Children, Parent};
    let mut world = World::new();
    world.insert_resource(PhysicsWorld::new());
    let crate_ = world.spawn();
    world.add_component(crate_, Transform::new(Vec3::new(1.0, 0.0, 0.0)));
    world.add_component(crate_, Collider::box_collider(Vec3::splat(0.3)));
    world.add_component(crate_, Breakable { threshold: 1.0, current_health: 50.0, max_health: 50.0, debris_lifetime: 3.0, ..Default::default() });
    let lid = world.spawn();
    world.add_component(lid, Transform::new(Vec3::new(0.0, 0.4, 0.0)));
    world.add_component(lid, Collider::box_collider(Vec3::new(0.3, 0.05, 0.3)));
    world.add_component(lid, Parent(crate_));
    world.add_component(crate_, Children(vec![lid]));
    let blast = world.spawn();
    world.add_component(
        blast,
        Explosion { force_radius: 10.0, force: 500.0, damage: 1000.0, damage_radius: 2.0, falloff: ExplosionFalloff::Linear, offset: Vec3::ZERO, is_active: true },
    );
    world.add_component(blast, Transform::new(Vec3::ZERO));
    physics_explosion_system(&world, DT);
    world.apply_commands();
    assert!(!world.is_alive(crate_), "the crate broke");
    assert!(world.query::<&RigidBody>().get(lid).is_some(), "its lid is a body of its own");
    let at = world.query::<&Transform>().get(lid).map(|t| t.position);
    assert_eq!(at, Some(Vec3::new(1.0, 0.4, 0.0)), "where the crate held it");
}

/// A body breaks into pieces that weigh what it did: its mass, shared by volume between its debris
/// and the parts it lets go of. Red on the code before, where each piece weighed its volume times
/// the material's relative density: a 600 kg crate's pieces weighed about 1.0 together.
#[test]
fn a_body_breaks_into_pieces_that_weigh_what_it_did() {
    use gizmo_core::component::{Children, Parent};
    let mut world = world_with_floor();
    let crate_ = world.spawn();
    world.add_component(crate_, Transform::new(Vec3::new(0.0, 3.0, 0.0)));
    world.add_component(crate_, RigidBody::new(600.0, true));
    world.add_component(crate_, Velocity::new(Vec3::new(0.0, -15.0, 0.0)));
    world.add_component(crate_, Collider::box_collider(Vec3::splat(0.5)));
    world.add_component(crate_, fragile());
    let handle = world.spawn();
    world.add_component(handle, Transform::new(Vec3::new(0.0, 0.6, 0.0)));
    world.add_component(handle, Collider::box_collider(Vec3::new(0.3, 0.05, 0.05)));
    world.add_component(handle, Parent(crate_));
    world.add_component(crate_, Children(vec![handle]));
    let mut broke = false;
    for _ in 0..60 {
        broke |= !step(&mut world).is_empty();
    }
    assert!(broke, "the crate broke");
    let pieces = world.query::<(&Debris, &RigidBody, &Collider)>();
    let total: f32 = pieces.iter().map(|(_, b, _)| b.mass).sum();
    assert!((total - 600.0).abs() < 0.6, "the pieces and the handle weigh what the crate did: {total} kg");
    // Shared by volume: every piece, the handle among them, at one mass per cubic metre.
    let volume: f32 = pieces.iter().map(|(_, _, c)| c.volume()).sum();
    let share = total / volume;
    for (_, b, c) in pieces.iter() {
        let density = b.mass / c.volume();
        assert!((density - share).abs() < 0.02 * share, "every piece at the crate's {share} kg/m³: {density}");
    }
    assert!(world.query::<&RigidBody>().get(handle).is_some_and(|b| b.mass > 0.0), "the handle broke loose with its share");
}
