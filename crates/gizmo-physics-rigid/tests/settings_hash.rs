//! `PhysicsWorld::settings_hash`: what decides how a world steps that is not its state, for two
//! rollback peers to compare before they play (`gizmo-net`'s session opening). The same set-up
//! gives the same hash wherever its bodies have got to; any setting that changes how the world
//! steps changes it, the order its bodies were added in with the rest.

use gizmo_math::Vec3;
use gizmo_physics_core::{BodyHandle, Collider, Transform};
use gizmo_physics_rigid::joints::Joint;
use gizmo_physics_rigid::world::FluidZone;
use gizmo_physics_rigid::{PhysicsWorld, RigidBody, Velocity};

fn body(w: &mut PhysicsWorld, id: u32, x: f32) {
    let col = Collider::box_collider(Vec3::splat(0.5));
    let mut rb = RigidBody::new(1.0 + id as f32, true);
    rb.update_inertia_from_collider(&col);
    w.add_body(BodyHandle::from_id(id), rb, Transform::new(Vec3::new(x, 1.0, 0.0)), Velocity::default(), col);
}

fn scene(order: &[u32]) -> PhysicsWorld {
    let mut w = PhysicsWorld::new();
    w.add_body(BodyHandle::from_id(0), RigidBody::new_static(), Transform::new(Vec3::new(0.0, -1.0, 0.0)), Velocity::default(), Collider::box_collider(Vec3::new(20.0, 1.0, 20.0)));
    for &id in order {
        body(&mut w, id, id as f32 * 1.5);
    }
    w
}

#[test]
fn the_same_set_up_hashes_the_same_wherever_its_bodies_have_got_to() {
    let base = scene(&[1, 2, 3]).settings_hash();
    assert_eq!(scene(&[1, 2, 3]).settings_hash(), base, "the same set-up, built twice");
    // Stepped until it has fallen, landed and slept: every kind of state a step changes.
    let mut stepped = scene(&[1, 2, 3]);
    let (_, joint) = jointed();
    stepped.joints.push(joint);
    let with_joint = stepped.settings_hash();
    stepped.rigid_bodies[1].force_accumulator = Vec3::X;
    for _ in 0..240 {
        stepped.step(1.0 / 60.0).unwrap();
    }
    assert!(stepped.rigid_bodies.iter().any(|b| b.is_dynamic() && b.is_sleeping), "premise: nothing slept");
    assert_eq!(stepped.settings_hash(), with_joint, "what a step changed counted as a setting");
    // What a step writes only on some bodies, written here by hand.
    let body = &mut stepped.rigid_bodies[1];
    (body.is_sleeping, body.sleep_counter, body.pose_driven) = (!body.is_sleeping, body.sleep_counter + 1, !body.pose_driven);
    (body.force_accumulator, body.torque_accumulator, body.contact_response) = (Vec3::X, Vec3::Y, Vec3::Z);
    stepped.joints[0].is_broken = true;
    assert_eq!(stepped.settings_hash(), with_joint, "what a step writes counted as a setting");
}

/// Two bodies and a joint between them.
fn jointed() -> (PhysicsWorld, Joint) {
    let joint = Joint::ball_socket(BodyHandle::from_id(2), BodyHandle::from_id(3), Vec3::new(0.75, 0.0, 0.0), Vec3::new(-0.75, 0.0, 0.0));
    let mut w = scene(&[1, 2, 3]);
    w.joints.push(joint.clone());
    (w, joint)
}

#[test]
fn every_setting_that_changes_a_step_changes_the_hash() {
    let base = scene(&[1, 2, 3]).settings_hash();
    let changed: Vec<(&str, PhysicsWorld)> = vec![
        ("substep rate", scene(&[1, 2, 3]).with_substep_rate(120).unwrap()),
        ("gravity", scene(&[1, 2, 3]).with_gravity(Vec3::new(0.0, -9.0, 0.0))),
        ("solver iterations", { let mut w = scene(&[1, 2, 3]); w.solver.iterations += 1; w }),
        ("contact stiffness", { let mut w = scene(&[1, 2, 3]); w.solver.contact_hertz *= 1.5; w }),
        ("joint solver", { let mut w = scene(&[1, 2, 3]); w.joint_solver.iterations += 1; w }),
        ("air density", { let mut w = scene(&[1, 2, 3]); w.integrator.air_density += 0.1; w }),
        ("a body's mass", { let mut w = scene(&[1, 2, 3]); w.rigid_bodies[2].mass = 9.0; w }),
        ("a collider's friction", { let mut w = scene(&[1, 2, 3]); w.colliders[1].material.dynamic_friction = 0.9; w }),
        ("a collider's shape", { let mut w = scene(&[1, 2, 3]); w.colliders[3] = Collider::sphere(0.5); w }),
        ("a fluid zone", { let mut w = scene(&[1, 2, 3]); w.fluid_zones.push(FluidZone::default()); w }),
        ("one more body", scene(&[1, 2, 3, 4])),
        ("the order the bodies were added in", scene(&[3, 1, 2])),
        ("a joint", jointed().0),
    ];
    for (what, w) in changed {
        assert_ne!(w.settings_hash(), base, "{what} did not change the hash");
    }
    let (with_joint, _) = jointed();
    let mut other = jointed().0;
    other.joints[0].local_anchor_a.y += 0.1;
    assert_ne!(other.settings_hash(), with_joint.settings_hash(), "a joint's anchor did not change the hash");
    other = jointed().0;
    other.joints[0].break_force = 10.0;
    assert_ne!(other.settings_hash(), with_joint.settings_hash(), "a joint's breaking force did not change the hash");
}
