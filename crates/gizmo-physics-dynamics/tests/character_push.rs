//! A character pushes the dynamic bodies it walks into (`CharacterController::push_force`).
//!
//! Before it, the controller treated every collider as level geometry: a character walking into
//! a 10 kg crate stopped at it as at a wall (measured: player x 1.20, crate x 2.00 after 3 s), so
//! a box-pushing puzzle could not be built.

#![cfg(feature = "ecs")]

use gizmo_core::entity::Entity;
use gizmo_core::world::World;
use gizmo_math::Vec3;
use gizmo_physics_core::components::CharacterController;
use gizmo_physics_core::{Collider, Transform};
use gizmo_physics_rigid::components::{RigidBody, Velocity};
use gizmo_physics_rigid::world::PhysicsWorld;

const WALK: f32 = 3.0;

struct Run {
    player_x: f32,
    crate_x: f32,
    /// The crate's fastest speed along +x over the whole run.
    crate_peak_vx: f32,
}

/// A character walking +x at [`WALK`] m/s into a crate of `mass` kg at x = 2, for 3 s.
fn walk_into_crate(mass: f32, push_force: Option<f32>) -> Run {
    let mut world = World::new();
    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));
    let ground = world.spawn();
    world.add_component(ground, Transform::new(Vec3::new(0.0, -0.5, 0.0)));
    world.add_component(ground, RigidBody::new_static());
    world.add_component(ground, Velocity::default());
    world.add_component(ground, Collider::box_collider(Vec3::new(20.0, 0.5, 20.0)));
    let crate_ = world.spawn();
    world.add_component(crate_, Transform::new(Vec3::new(2.0, 0.5, 0.0)));
    world.add_component(crate_, RigidBody::new(mass, true));
    world.add_component(crate_, Velocity::default());
    world.add_component(crate_, Collider::box_collider(Vec3::splat(0.5)));
    let player = world.spawn();
    world.add_component(player, Transform::new(Vec3::new(0.0, 0.9, 0.0)));
    let mut kcc = CharacterController {
        speed: WALK,
        target_velocity: Vec3::new(WALK, 0.0, 0.0),
        ..Default::default()
    };
    if let Some(f) = push_force {
        kcc.push_force = f;
    }
    world.add_component(player, kcc);
    world.add_component(player, Velocity::default());
    world.add_component(player, Collider::capsule(0.3, 0.4));

    let x = |world: &World, e: Entity| world.query::<&Transform>().get(e).unwrap().position.x;
    let dt = 1.0 / 60.0;
    let mut crate_peak_vx = 0.0f32;
    for _ in 0..180 {
        gizmo_physics_dynamics::character_controller_system(&world, dt);
        gizmo_physics_rigid::system::physics_step_system(&world, dt);
        world.apply_commands();
        let vx = world.query::<&Velocity>().get(crate_).unwrap().linear.x;
        crate_peak_vx = crate_peak_vx.max(vx);
    }
    Run { player_x: x(&world, player), crate_x: x(&world, crate_), crate_peak_vx }
}

#[test]
fn a_character_pushes_a_light_crate_at_walking_pace() {
    let r = walk_into_crate(10.0, None);
    // Contact at t ≈ 0.4 s (player x 1.2), then 2.6 s at up to 3 m/s: ≈ 7.8 m more at most.
    assert!(r.crate_x > 8.0, "the crate was pushed only to x = {:.2}", r.crate_x);
    assert!(r.crate_x < 2.0 + 7.8 + 0.1, "the crate went further than walking pace allows: x = {:.2}", r.crate_x);
    // The player keeps walking behind it, at the crate's face (half-width 0.5 + radius 0.3).
    let gap = r.crate_x - r.player_x;
    assert!((0.75..1.0).contains(&gap), "player x {:.2}, crate x {:.2}: gap {gap:.2}", r.player_x, r.crate_x);
    // Never flung: the push stops adding speed once the crate moves as fast as the character.
    assert!(r.crate_peak_vx <= WALK + 0.05, "the crate reached {:.2} m/s, faster than the walk", r.crate_peak_vx);
}

#[test]
fn a_crate_heavier_than_the_push_can_slide_does_not_move() {
    // 500 kg: ground friction (~μ·m·g, thousands of newtons) is far beyond the default 400 N.
    let r = walk_into_crate(500.0, None);
    assert!((r.crate_x - 2.0).abs() < 0.05, "a 500 kg crate moved to x = {:.2}", r.crate_x);
    assert!(r.player_x < 1.25, "the player walked into the crate: x = {:.2}", r.player_x);
}

#[test]
fn push_force_zero_turns_pushing_off() {
    let r = walk_into_crate(10.0, Some(0.0));
    assert!((r.crate_x - 2.0).abs() < 0.01, "push_force 0 still moved the crate to x = {:.2}", r.crate_x);
    assert!(r.player_x < 1.25, "the player walked into the crate: x = {:.2}", r.player_x);
}

#[test]
fn push_force_survives_a_scene_file_written_before_it() {
    // A controller serialized before the field existed must still load, with the default.
    let mut json = serde_json::to_value(CharacterController::default()).unwrap();
    json.as_object_mut().unwrap().remove("push_force");
    let back: CharacterController = serde_json::from_value(json).unwrap();
    assert_eq!(back.push_force, CharacterController::default().push_force);
}
