//! A sleeping body wakes when what holds it at rest changes: something moving pushes it, or the
//! gravity it settled under is no longer the gravity.
//!
//! Both found on 2026-11-23 by driving a settled box the way a game would, through the ECS step.

use gizmo_core::entity::Entity;
use gizmo_core::world::World;
use gizmo_math::{Quat, Vec3};
use gizmo_physics_core::{Collider, Transform};
use gizmo_physics_rigid::components::{KinematicTarget, RigidBody, Velocity};
use gizmo_physics_rigid::system::physics_step_system;
use gizmo_physics_rigid::world::PhysicsWorld;

fn step(world: &mut World) {
    world.tick += 1;
    physics_step_system(world, 1.0 / 60.0);
    world.apply_commands();
}

/// A unit box asleep on a floor.
fn settled_box() -> (World, Entity) {
    let mut world = World::new();
    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));
    let floor = world.spawn();
    world.add_component(floor, Transform::new(Vec3::new(0.0, -0.5, 0.0)));
    world.add_component(floor, Collider::box_collider(Vec3::new(5.0, 0.5, 5.0)));
    let b = world.spawn();
    world.add_component(b, Transform::new(Vec3::new(0.0, 0.5, 0.0)));
    world.add_component(b, RigidBody::new(1.0, true));
    world.add_component(b, Velocity::default());
    world.add_component(b, Collider::box_collider(Vec3::splat(0.5)));
    for _ in 0..600 {
        step(&mut world);
    }
    assert!(world.query::<&RigidBody>().get(b).is_some_and(|rb| rb.is_sleeping), "the box settled and slept");
    (world, b)
}

/// A kinematic paddle sliding into a sleeping box pushes it, and the box the game sees is the box
/// the physics has, every step.
///
/// Until 2026-11-23 the island of the box and the paddle read as all-eligible to sleep — the
/// paddle has no sleep counter — while the paddle woke the box, so the box was woken (its counter
/// to 0, a velocity written) and put straight back to sleep in the same substep. Nothing told the
/// ECS mirror: the step's debug check panicked ("neither brought over nor written back"), and a
/// release build showed a box a step behind its physics.
#[test]
fn a_kinematic_push_keeps_the_box_it_pushes_awake_and_in_step() {
    let (mut world, b) = settled_box();
    let paddle = world.spawn();
    world.add_component(paddle, Transform::new(Vec3::new(-2.0, 0.5, 0.0)));
    world.add_component(paddle, RigidBody::new_kinematic());
    world.add_component(paddle, Velocity::default());
    world.add_component(paddle, Collider::box_collider(Vec3::splat(0.5)));
    world.add_component(paddle, KinematicTarget::new(Vec3::new(-2.0, 0.5, 0.0), Quat::IDENTITY));
    for i in 0..120 {
        let x = -2.0 + i as f32 * (3.0 / 120.0);
        if let Some(mut t) = world.query_mut::<&mut KinematicTarget>().get_mut(paddle) {
            *t = KinematicTarget::new(Vec3::new(x, 0.5, 0.0), Quat::IDENTITY);
        }
        step(&mut world);
        let ecs = world.query::<&RigidBody>().get(b).copied().expect("the box");
        let pw = world.get_resource::<PhysicsWorld>().expect("the physics world");
        let idx = *pw.entity_index_map.get(&b.id()).expect("the box's row");
        let (body, vel) = (pw.rigid_bodies[idx], pw.velocities[idx]);
        assert!(
            !body.is_sleeping || body.sleep_eligible(pw.sleep_steps()),
            "step {i}: the box is asleep without having been still (counter {})",
            body.sleep_counter
        );
        let ecs_vel = world.query::<&Velocity>().get(b).copied().expect("the box's velocity");
        assert!(ecs == body && ecs_vel == vel, "step {i}: the ECS box is not the physics box");
    }
    let x = world.query::<&Transform>().get(b).map(|t| t.position.x).expect("the box");
    assert!(x > 1.5, "the paddle pushed the box along: x {x}");
}

/// Gravity turned over lifts a sleeping box off the floor.
///
/// Until 2026-11-23 the box stayed asleep where it lay: nothing touched it, and a sleeping body is
/// not integrated, so a game that inverted gravity left every settled body on the floor.
#[test]
fn a_change_of_gravity_wakes_what_slept_under_the_old_one() {
    let (mut world, b) = settled_box();
    world.get_resource_mut::<PhysicsWorld>().expect("the physics world").integrator.gravity = Vec3::new(0.0, 9.81, 0.0);
    for _ in 0..60 {
        step(&mut world);
    }
    let y = world.query::<&Transform>().get(b).map(|t| t.position.y).expect("the box");
    assert!(y > 2.0, "the box fell up: y {y}");
}
