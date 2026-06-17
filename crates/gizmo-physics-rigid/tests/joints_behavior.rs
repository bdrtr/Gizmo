//! Per-type behavioural tests for the joint library — Faz 4.
//!
//! Existing coverage (proptest_joints) only checked the ball-socket point
//! constraint + chain stability. These lock the *feature* behaviour of every
//! joint type — motor, limit, cone, rotation lock, spring — and are discriminating:
//!   * FIXED rotation lock — found+fixed: the bare point constraint let a Fixed
//!     joint spin freely (≈ ball-socket); now relative rotation is locked.
//!   * SLIDER limit — found+fixed: the limit's impulse-clamp bounds were inverted,
//!     so a fast body blew straight through it (1 m limit → 19 m travel).

use gizmo_core::entity::Entity;
use gizmo_math::{Quat, Vec3};
use gizmo_physics_core::{Collider, Transform};
use gizmo_physics_rigid::{Joint, JointData, PhysicsWorld, RigidBody, Velocity};

fn anchor(world: &mut PhysicsWorld, id: u32, pos: Vec3) {
    let mut rb = RigidBody::new_static();
    rb.wake_up();
    world.add_body(Entity::new(id, 0), rb, Transform::new(pos), Velocity::default(), Collider::sphere(0.2));
}
fn dyn_box(world: &mut PhysicsWorld, id: u32, pos: Vec3, lin: Vec3, ang: Vec3, gravity: bool) {
    let mut rb = RigidBody::new(1.0, 0.0, 0.0, gravity);
    rb.wake_up();
    let col = Collider::box_collider(Vec3::splat(0.3));
    rb.update_inertia_from_collider(&col);
    let v = Velocity { linear: lin, angular: ang, ..Velocity::default() };
    world.add_body(Entity::new(id, 0), rb, Transform::new(pos), v, col);
}
fn quat_angle(q: Quat) -> f32 {
    2.0 * q.normalize().w.clamp(-1.0, 1.0).acos()
}

#[test]
fn fixed_joint_locks_rotation() {
    // Spin B; a Fixed joint must lock it to A. (Old point-only Fixed left it spinning
    // at ≈3 rad rotation / ≈4.5 rad·s⁻¹.)
    let mut world = PhysicsWorld::new().with_gravity(Vec3::ZERO);
    anchor(&mut world, 1, Vec3::ZERO);
    dyn_box(&mut world, 2, Vec3::new(0.5, 0.0, 0.0), Vec3::ZERO, Vec3::new(0.0, 5.0, 0.0), false);
    world.joints.push(Joint::fixed(Entity::new(1, 0), Entity::new(2, 0), Vec3::new(0.5, 0.0, 0.0), Vec3::ZERO));
    for _ in 0..120 { world.step(1.0 / 60.0).ok(); }
    assert!(quat_angle(world.transforms[1].rotation) < 0.1, "Fixed joint did not lock rotation");
    assert!(world.velocities[1].angular.length() < 0.5, "Fixed joint left residual spin");
}

#[test]
fn fixed_joint_welds_offset_load_under_gravity() {
    // Anchor OFFSET from B's centre of mass; gravity then exerts a torque about the
    // anchor. A genuine weld must hold B horizontal (no swing-down rotation) over 5 s.
    let mut world = PhysicsWorld::new();
    world.integrator.gravity = Vec3::new(0.0, -10.0, 0.0);
    anchor(&mut world, 1, Vec3::new(0.7, 0.0, 0.0));
    dyn_box(&mut world, 2, Vec3::new(1.0, 0.0, 0.0), Vec3::ZERO, Vec3::ZERO, true);
    // anchor_a = (0.7,0,0); anchor_b = B.pos + (-0.3,0,0) = (0.7,0,0) → coincide, 0.3 m
    // lever from B's COM.
    world.joints.push(Joint::fixed(Entity::new(1, 0), Entity::new(2, 0), Vec3::ZERO, Vec3::new(-0.3, 0.0, 0.0)));
    for _ in 0..300 { world.step(1.0 / 60.0).ok(); }
    let p = world.transforms[1].position;
    assert!(quat_angle(world.transforms[1].rotation) < 0.15, "weld rotated under offset gravity load");
    assert!((p - Vec3::new(1.0, 0.0, 0.0)).length() < 0.15, "weld drifted: {p:?}");
}

#[test]
fn hinge_limit_stops_rotation() {
    let mut world = PhysicsWorld::new().with_gravity(Vec3::ZERO);
    anchor(&mut world, 1, Vec3::ZERO);
    dyn_box(&mut world, 2, Vec3::new(0.5, 0.0, 0.0), Vec3::ZERO, Vec3::new(0.0, 0.0, 3.0), false);
    let mut j = Joint::hinge(Entity::new(1, 0), Entity::new(2, 0), Vec3::new(0.5, 0.0, 0.0), Vec3::ZERO, Vec3::Z);
    if let JointData::Hinge(ref mut d) = j.data {
        d.use_limits = true; d.lower_limit = -0.5; d.upper_limit = 0.5;
    }
    world.joints.push(j);
    let mut max_angle = 0f32;
    for _ in 0..240 {
        world.step(1.0 / 60.0).ok();
        if let JointData::Hinge(d) = world.joints[0].data { max_angle = max_angle.max(d.current_angle.abs()); }
    }
    assert!(max_angle < 0.7, "hinge limit breached: {max_angle}");
}

#[test]
fn hinge_motor_reaches_target_velocity() {
    let mut world = PhysicsWorld::new().with_gravity(Vec3::ZERO);
    anchor(&mut world, 1, Vec3::ZERO);
    dyn_box(&mut world, 2, Vec3::new(0.5, 0.0, 0.0), Vec3::ZERO, Vec3::ZERO, false);
    let mut j = Joint::hinge(Entity::new(1, 0), Entity::new(2, 0), Vec3::new(0.5, 0.0, 0.0), Vec3::ZERO, Vec3::Z);
    if let JointData::Hinge(ref mut d) = j.data {
        d.use_motor = true; d.motor_target_velocity = 2.0; d.motor_max_force = 50.0;
    }
    world.joints.push(j);
    for _ in 0..240 { world.step(1.0 / 60.0).ok(); }
    assert!((world.velocities[1].angular.z - 2.0).abs() < 0.3, "hinge motor missed target: {}", world.velocities[1].angular.z);
}

#[test]
fn slider_limit_stops_translation() {
    // B launched at 5 m/s along +X; limit at 1 m must stop it. (Inverted clamp bug
    // let it reach ≈19.6 m.)
    let mut world = PhysicsWorld::new().with_gravity(Vec3::ZERO);
    anchor(&mut world, 1, Vec3::ZERO);
    dyn_box(&mut world, 2, Vec3::ZERO, Vec3::new(5.0, 0.0, 0.0), Vec3::ZERO, false);
    let mut j = Joint::slider(Entity::new(1, 0), Entity::new(2, 0), Vec3::ZERO, Vec3::ZERO, Vec3::X);
    if let JointData::Slider(ref mut d) = j.data {
        d.use_limits = true; d.lower_limit = -1.0; d.upper_limit = 1.0;
    }
    world.joints.push(j);
    let mut max_x = 0f32;
    for _ in 0..240 { world.step(1.0 / 60.0).ok(); max_x = max_x.max(world.transforms[1].position.x.abs()); }
    assert!(max_x < 1.2, "slider limit breached: max |x| = {max_x}");
}

#[test]
fn slider_motor_reaches_target_velocity() {
    let mut world = PhysicsWorld::new().with_gravity(Vec3::ZERO);
    anchor(&mut world, 1, Vec3::ZERO);
    dyn_box(&mut world, 2, Vec3::ZERO, Vec3::ZERO, Vec3::ZERO, false);
    let mut j = Joint::slider(Entity::new(1, 0), Entity::new(2, 0), Vec3::ZERO, Vec3::ZERO, Vec3::X);
    if let JointData::Slider(ref mut d) = j.data {
        d.use_motor = true; d.motor_target_velocity = 2.0; d.motor_max_force = 50.0;
    }
    world.joints.push(j);
    for _ in 0..240 { world.step(1.0 / 60.0).ok(); }
    assert!((world.velocities[1].linear.x - 2.0).abs() < 0.3, "slider motor missed target: {}", world.velocities[1].linear.x);
}

#[test]
fn ballsocket_cone_limit_clamps_swing() {
    let mut world = PhysicsWorld::new().with_gravity(Vec3::ZERO);
    anchor(&mut world, 1, Vec3::ZERO);
    dyn_box(&mut world, 2, Vec3::new(0.5, 0.0, 0.0), Vec3::ZERO, Vec3::new(3.0, 0.0, 0.0), false);
    let mut j = Joint::ball_socket(Entity::new(1, 0), Entity::new(2, 0), Vec3::new(0.5, 0.0, 0.0), Vec3::ZERO);
    if let JointData::BallSocket(ref mut d) = j.data {
        d.use_cone_limit = true; d.cone_limit_angle = 0.5;
    }
    world.joints.push(j);
    let mut max_swing = 0f32;
    for _ in 0..240 { world.step(1.0 / 60.0).ok(); max_swing = max_swing.max(quat_angle(world.transforms[1].rotation)); }
    assert!(max_swing < 0.7, "ball-socket cone limit breached: {max_swing}");
}

#[test]
fn spring_settles_at_rest_length() {
    let mut world = PhysicsWorld::new().with_gravity(Vec3::ZERO);
    dyn_box(&mut world, 1, Vec3::new(-2.0, 0.0, 0.0), Vec3::ZERO, Vec3::ZERO, false);
    dyn_box(&mut world, 2, Vec3::new(2.0, 0.0, 0.0), Vec3::ZERO, Vec3::ZERO, false);
    // separation 4 m, rest 1 m, stiff spring with damping → settles near rest length.
    world.joints.push(Joint::spring(Entity::new(1, 0), Entity::new(2, 0), Vec3::ZERO, Vec3::ZERO, 1.0, 30.0, 3.0));
    for _ in 0..600 { world.step(1.0 / 60.0).ok(); }
    let sep = (world.transforms[0].position - world.transforms[1].position).length();
    assert!((sep - 1.0).abs() < 0.2, "spring did not settle at rest length: {sep}");
}
