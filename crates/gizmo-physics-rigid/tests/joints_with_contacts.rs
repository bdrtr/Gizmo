//! A body held by a joint and touched by contacts is solved against both in one solve.
//!
//! Until 2026-11-23 a substep solved every contact island to the end and then every joint, each
//! blind to the other. A box resting on a hinged seesaw shook at 6 cm/s, and neither it nor the
//! seesaw ever slept: each substep the contacts stopped the box, and the joint, solved after, moved
//! the plank under it. Joints are now edges of the contact islands
//! (`IslandManager::build_islands_with_joints`), and an island's joints are swept before each of its
//! contact sweeps (`pipeline.rs`, `solve_island_contacts`).

use gizmo_core::world::World;
use gizmo_math::{Quat, Vec3};
use gizmo_physics_core::{BodyHandle, Collider, Transform};
use gizmo_physics_rigid::components::{RigidBody, Velocity};
use gizmo_physics_rigid::joints::Joint;
use gizmo_physics_rigid::system::physics_step_system;
use gizmo_physics_rigid::world::PhysicsWorld;

/// sin θ = pivot height / half length: the plank's end on the ground.
const TILT: f32 = 0.3047;
const PLANK: Vec3 = Vec3::new(2.0, 0.05, 0.35);

/// A 4 m plank on a hinge at the top of a 0.6 m block, resting on one end, with a 0.3 m box on its
/// lower half. Returns the world, the plank and the box.
fn seesaw(joint: fn(BodyHandle, BodyHandle, Vec3, Vec3) -> Joint) -> (World, gizmo_core::entity::Entity, gizmo_core::entity::Entity) {
    let mut world = World::new();
    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));
    let rot = Quat::from_rotation_z(TILT);
    let pivot = Vec3::new(0.0, 0.6, 0.0);
    let floor = world.spawn();
    world.add_component(floor, Transform::new(Vec3::new(0.0, -0.1, 0.0)));
    world.add_component(floor, Collider::box_collider(Vec3::new(10.0, 0.1, 10.0)));
    let block = world.spawn();
    world.add_component(block, Transform::new(Vec3::new(0.0, 0.3, 0.0)));
    world.add_component(block, RigidBody::new_static());
    world.add_component(block, Velocity::default());
    world.add_component(block, Collider::box_collider(Vec3::new(0.15, 0.3, 0.3)));
    let plank = world.spawn();
    world.add_component(plank, Transform::new(pivot + rot * Vec3::Y * PLANK.y).with_rotation(rot));
    world.add_component(plank, RigidBody::new(4.0, true));
    world.add_component(plank, Velocity::default());
    world.add_component(plank, Collider::box_collider(PLANK));
    let load = world.spawn();
    world.add_component(load, Transform::new(pivot + rot * Vec3::new(-1.0, 2.0 * PLANK.y + 0.15, 0.0)).with_rotation(rot));
    world.add_component(load, RigidBody::new(0.5, true));
    world.add_component(load, Velocity::default());
    world.add_component(load, Collider::box_collider(Vec3::splat(0.15)));
    let (a, b) = (BodyHandle::from_id(plank.id()), BodyHandle::from_id(block.id()));
    world.get_resource_mut::<PhysicsWorld>().expect("the physics world").joints.push(joint(a, b, Vec3::new(0.0, -PLANK.y, 0.0), Vec3::new(0.0, 0.3, 0.0)));
    (world, plank, load)
}

fn hinge(a: BodyHandle, b: BodyHandle, la: Vec3, lb: Vec3) -> Joint {
    Joint::hinge(a, b, la, lb, Vec3::Z)
}

fn settles(joint: fn(BodyHandle, BodyHandle, Vec3, Vec3) -> Joint, name: &str) {
    let (mut world, plank, load) = seesaw(joint);
    let mut fastest = 0.0f32;
    for i in 0..480 {
        world.tick += 1;
        physics_step_system(&world, 1.0 / 60.0);
        world.apply_commands();
        if i >= 360 {
            let v = world.query::<&Velocity>().get(load).map(|v| v.linear.length()).expect("the box");
            fastest = fastest.max(v);
        }
    }
    let asleep = |e| world.query::<&RigidBody>().get(e).is_some_and(|rb| rb.is_sleeping);
    assert!(fastest < 1e-3, "{name}: the box on the plank still moves at {fastest} m/s after six seconds");
    assert!(asleep(plank) && asleep(load), "{name}: the seesaw and its box sleep (plank {}, box {})", asleep(plank), asleep(load));
}

/// The seesaw at rest with a box on it comes to rest and sleeps, on a hinge and on a ball socket.
#[test]
fn a_box_on_a_hinged_seesaw_comes_to_rest_and_sleeps() {
    settles(hinge, "hinge");
    settles(Joint::ball_socket, "ball socket");
}
