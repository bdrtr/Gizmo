//! **What a joint carries is readable.** `Joint::last_force` / `last_torque` / `load` report the
//! reaction the solver measures against the break thresholds — it was `pub(crate)`, so a game
//! could not show how close a beam was to giving way.

use gizmo_core::world::World;
use gizmo_math::Vec3;
use gizmo_physics_core::{BodyHandle, Collider, Transform};
use gizmo_physics_rigid::components::{RigidBody, Velocity};
use gizmo_physics_rigid::joints::Joint;
use gizmo_physics_rigid::world::PhysicsWorld;

/// A 10 kg weight hung 2 m below a fixed point on a ball socket; returns the world after
/// `steps` (long enough for it to settle and fall asleep).
fn hanging(break_force: f32, steps: usize) -> World {
    let mut world = World::new();
    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));
    let anchor = world.spawn();
    world.add_component(anchor, Transform::new(Vec3::new(0.0, 10.0, 0.0)));
    world.add_component(anchor, RigidBody::new_static());
    world.add_component(anchor, Velocity::default());
    world.add_component(anchor, Collider::sphere(0.1));
    let weight = world.spawn();
    world.add_component(weight, Transform::new(Vec3::new(0.0, 8.0, 0.0)));
    world.add_component(weight, RigidBody::new(10.0, true));
    world.add_component(weight, Velocity::default());
    world.add_component(weight, Collider::sphere(0.2));
    let mut joint = Joint::ball_socket(BodyHandle::from_id(anchor.id()), BodyHandle::from_id(weight.id()), Vec3::new(0.0, -2.0, 0.0), Vec3::ZERO);
    joint.break_force = break_force;
    world.get_resource_mut::<PhysicsWorld>().unwrap().joints.push(joint);
    for _ in 0..steps {
        gizmo_physics_rigid::system::physics_step_system(&world, 1.0 / 120.0);
        world.apply_commands();
    }
    world
}

#[test]
fn a_hanging_weight_loads_its_joint_with_its_weight_even_asleep() {
    let world = hanging(200.0, 240);
    let physics = world.get_resource::<PhysicsWorld>().unwrap();
    let rope = &physics.joints[0];
    let force = rope.last_force();
    eprintln!("force {force} load {} broken {}", rope.load(), rope.is_broken);
    assert!(!rope.is_broken, "98 N on a 200 N joint — it used to read 196 N and break at 100");
    assert!((force - 98.1).abs() < 0.05 * 98.1, "10 kg hangs with ~98 N: {force}");
    assert!((rope.load() - force / 200.0).abs() < 1e-6, "load is force over the threshold");
    assert!(!rope.is_broken);
}

#[test]
fn a_joint_too_weak_for_the_weight_breaks_at_a_load_above_one() {
    let world = hanging(50.0, 240);
    let physics = world.get_resource::<PhysicsWorld>().unwrap();
    let rope = &physics.joints[0];
    eprintln!("broken {} force {} load {}", rope.is_broken, rope.last_force(), rope.load());
    assert!(rope.is_broken, "98 N on a 50 N joint");
    assert!(rope.load() > 1.0, "the step that broke it is the last one it reports: {}", rope.load());
}
