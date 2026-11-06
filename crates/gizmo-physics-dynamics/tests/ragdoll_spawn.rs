//! A humanoid ragdoll is born in one piece and at rest.
//!
//! Until 2026-11-04 nine of the stock humanoid's ten joints were apart at spawn — each bone's
//! `local_pos` disagreed with the anchors of the joint that holds it (the shoulders by 18 cm) —
//! so every ragdoll snapped itself together on its first step, and its lower arms were born
//! inside the torso they do not hang from, which pushed them out. Measured in zero gravity: the
//! fastest bone at 6.97 rad/s a second after spawning, from nothing.

use gizmo_core::world::World;
use gizmo_math::Vec3;
use gizmo_physics_core::Transform;
use gizmo_physics_dynamics::ragdoll::RagdollBuilder;
use gizmo_physics_rigid::components::Velocity;
use gizmo_physics_rigid::world::PhysicsWorld;

fn humanoid(world: &mut World) -> gizmo_physics_dynamics::ragdoll::RagdollInstance {
    let mut b = RagdollBuilder::new(Vec3::new(0.0, 2.0, 0.0));
    b.create_humanoid();
    b.spawn(world)
}

#[test]
fn every_joint_of_a_new_humanoid_is_closed() {
    let mut world = World::new();
    world.insert_resource(PhysicsWorld::new());
    let ragdoll = humanoid(&mut world);
    let pw = world.get_resource::<PhysicsWorld>().unwrap();
    let ts = world.borrow::<Transform>();
    for &j in &ragdoll.joint_indices {
        let joint = &pw.joints[j];
        let (a, b) = (ts.get(joint.entity_a.id()).unwrap(), ts.get(joint.entity_b.id()).unwrap());
        let gap = ((a.position + a.rotation * joint.local_anchor_a) - (b.position + b.rotation * joint.local_anchor_b)).length();
        assert!(gap < 1e-4, "joint {j} is {gap:.3} m apart at spawn");
    }
}

#[test]
fn a_new_humanoid_in_zero_gravity_stays_still() {
    let mut world = World::new();
    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::ZERO));
    let ragdoll = humanoid(&mut world);
    for _ in 0..60 {
        gizmo_physics_rigid::system::physics_step_system(&world, 1.0 / 60.0);
    }
    let vs = world.borrow::<Velocity>();
    let (v, w) = ragdoll.bones.iter().fold((0.0f32, 0.0f32), |(v, w), (_, e)| {
        let vel = vs.get(*e).unwrap();
        (v.max(vel.linear.length()), w.max(vel.angular.length()))
    });
    assert!(v < 0.05 && w < 0.05, "a ragdoll born at rest moves: {v:.2} m/s, {w:.2} rad/s after a second");
}
