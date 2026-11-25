//! `PhysicsWorld::settings_hash`: what decides how a world steps that is not its state, for two
//! rollback peers to compare before they play (`gizmo-net`'s session opening). The same set-up
//! gives the same hash wherever its bodies have got to; any setting that changes how the world
//! steps changes it, the order its bodies were added in with the rest.

use gizmo_math::{Quat, Vec3};
use gizmo_physics_core::{BodyHandle, Collider, Transform};
use gizmo_physics_rigid::joints::{Joint, JointData};
use gizmo_physics_rigid::world::{FluidZone, Weather};
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

/// A joint of each kind that keeps something from its last solve: an angle, a travel, a reference
/// pose.
fn joint_kinds() -> Vec<(&'static str, Joint)> {
    let (a, b) = (BodyHandle::from_id(2), BodyHandle::from_id(3));
    let (at_a, at_b) = (Vec3::new(0.75, 0.0, 0.0), Vec3::new(-0.75, 0.0, 0.0));
    let mut cone = Joint::ball_socket(a, b, at_a, at_b);
    if let JointData::BallSocket(d) = &mut cone.data {
        (d.use_cone_limit, d.cone_limit_angle) = (true, 0.5);
    }
    vec![
        ("hinge", Joint::hinge(a, b, at_a, at_b, Vec3::Z)),
        ("ball-socket", cone),
        ("slider", Joint::slider(a, b, at_a, at_b, Vec3::X)),
        ("D6", Joint::d6(a, b, at_a, at_b)),
    ]
}

/// What a joint's solve leaves in it is state, not a setting: `state_hash` sees it, `settings_hash`
/// does not. It was the other way round until 2026-11-25, when only a joint that keeps nothing was
/// tested: a scene with a hinge changed its settings hash by stepping, and a reference pose, which
/// nothing else of the state gives back, was in no state hash.
#[test]
fn what_a_joint_keeps_from_its_solve_is_state_and_not_a_setting() {
    for (kind, joint) in joint_kinds() {
        let mut w = scene(&[1, 2, 3]);
        w.joints.push(joint);
        // Turned and away, so an angle, a travel and a pose are all something.
        w.transforms[3].rotation = Quat::from_rotation_z(0.2);
        w.transforms[3].position.x += 0.1;
        let settings = w.settings_hash();
        for _ in 0..30 {
            w.step(1.0 / 60.0).unwrap();
        }
        assert_eq!(w.settings_hash(), settings, "{kind}: what its solve left counted as a setting");

        // The same world but for what the joint kept.
        let before = format!("{:?}", w.joints[0].data);
        let state = w.state_hash();
        match &mut w.joints[0].data {
            JointData::Hinge(d) => d.current_angle += 0.1,
            JointData::Slider(d) => d.current_position += 0.1,
            JointData::BallSocket(d) => d.initial_relative_rotation = None,
            JointData::D6(d) => d.initial_relative_rotation = None,
            _ => unreachable!(),
        }
        assert_ne!(before, format!("{:?}", w.joints[0].data), "premise: {kind} kept nothing");
        assert_ne!(w.state_hash(), state, "{kind}: what its solve left is in no state hash");
        assert_eq!(w.settings_hash(), settings, "{kind}: what it keeps counted as a setting");
        // A slider keeps a pose as well as its travel.
        if matches!(w.joints[0].data, JointData::Slider(_)) {
            let state = w.state_hash();
            let JointData::Slider(d) = &mut w.joints[0].data else { unreachable!() };
            assert!(d.initial_relative_rotation.take().is_some(), "premise: no pose kept");
            assert_ne!(w.state_hash(), state, "slider: its reference pose is in no state hash");
            assert_eq!(w.settings_hash(), settings, "slider: its reference pose counted as a setting");
        }
    }
}

/// Water's colour under the surface is the picture's: two peers that draw it differently step alike.
#[test]
fn a_fluid_zones_fog_is_not_a_setting() {
    let with = |zone: FluidZone| {
        let mut w = scene(&[1]);
        w.fluid_zones.push(zone);
        w.settings_hash()
    };
    let mut foggy = FluidZone::default();
    (foggy.fog_color, foggy.fog_density) = ([0.9, 0.1, 0.1], 3.0);
    assert_eq!(with(foggy), with(FluidZone::default()));
    let mut dense = FluidZone::default();
    dense.density *= 2.0;
    assert_ne!(with(dense), with(FluidZone::default()), "a zone's density did not change the hash");
}

/// What a snapshot carries and a step reads, and no transform or velocity shows: two worlds that
/// differ in one of these are not in the same state.
#[test]
fn what_a_snapshot_carries_unseen_is_in_the_state_hash() {
    let base = scene(&[1, 2, 3]).state_hash();
    let changed: Vec<(&str, PhysicsWorld)> = vec![
        // Less than a substep: nothing has moved but the remainder.
        ("time not yet stepped", { let mut w = scene(&[1, 2, 3]); w.step(1.0 / 1000.0).unwrap(); w }),
        ("weather", { let mut w = scene(&[1, 2, 3]); w.weather = Weather::Rain; w }),
        ("a sleep counter", { let mut w = scene(&[1, 2, 3]); w.rigid_bodies[1].sleep_counter += 1; w }),
        ("a gathered force", { let mut w = scene(&[1, 2, 3]); w.rigid_bodies[1].force_accumulator = Vec3::X; w }),
        ("a gathered torque", { let mut w = scene(&[1, 2, 3]); w.rigid_bodies[1].torque_accumulator = Vec3::X; w }),
    ];
    for (what, w) in changed {
        assert_eq!(w.transforms[1].position, scene(&[1, 2, 3]).transforms[1].position, "premise: {what} moved a body");
        assert_ne!(w.state_hash(), base, "{what} did not change the state hash");
    }
}
