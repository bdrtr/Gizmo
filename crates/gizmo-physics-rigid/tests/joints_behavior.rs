//! Per-type behavioural tests for the joint library — Faz 4.
//!
//! Existing coverage (proptest_joints) only checked the ball-socket point
//! constraint + chain stability. These lock the *feature* behaviour of every
//! joint type — motor, limit, cone, rotation lock, spring — and are discriminating:
//!   * FIXED rotation lock — found+fixed: the bare point constraint let a Fixed
//!     joint spin freely (≈ ball-socket); now relative rotation is locked.
//!   * SLIDER limit — found+fixed: the limit's impulse-clamp bounds were inverted,
//!     so a fast body blew straight through it (1 m limit → 19 m travel).

use gizmo_physics_core::BodyHandle;
use gizmo_math::{Quat, Vec3};
use gizmo_physics_core::{Collider, Transform};
use gizmo_physics_rigid::{D6Drive, D6Motion, Joint, JointData, PhysicsWorld, RigidBody, Velocity};

fn anchor(world: &mut PhysicsWorld, id: u32, pos: Vec3) {
    let mut rb = RigidBody::new_static();
    rb.wake_up();
    world.add_body(BodyHandle::from_id(id), rb, Transform::new(pos), Velocity::default(), Collider::sphere(0.2));
}
fn dyn_box(world: &mut PhysicsWorld, id: u32, pos: Vec3, lin: Vec3, ang: Vec3, gravity: bool) {
    let mut rb = RigidBody::new(1.0, gravity);
    rb.wake_up();
    let col = Collider::box_collider(Vec3::splat(0.3));
    rb.update_inertia_from_collider(&col);
    let v = Velocity { linear: lin, angular: ang, ..Velocity::default() };
    world.add_body(BodyHandle::from_id(id), rb, Transform::new(pos), v, col);
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
    world.joints.push(Joint::fixed(BodyHandle::from_id(1), BodyHandle::from_id(2), Vec3::new(0.5, 0.0, 0.0), Vec3::ZERO));
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
    world.joints.push(Joint::fixed(BodyHandle::from_id(1), BodyHandle::from_id(2), Vec3::ZERO, Vec3::new(-0.3, 0.0, 0.0)));
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
    let mut j = Joint::hinge(BodyHandle::from_id(1), BodyHandle::from_id(2), Vec3::new(0.5, 0.0, 0.0), Vec3::ZERO, Vec3::Z);
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
    let mut j = Joint::hinge(BodyHandle::from_id(1), BodyHandle::from_id(2), Vec3::new(0.5, 0.0, 0.0), Vec3::ZERO, Vec3::Z);
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
    let mut j = Joint::slider(BodyHandle::from_id(1), BodyHandle::from_id(2), Vec3::ZERO, Vec3::ZERO, Vec3::X);
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
    let mut j = Joint::slider(BodyHandle::from_id(1), BodyHandle::from_id(2), Vec3::ZERO, Vec3::ZERO, Vec3::X);
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
    let mut j = Joint::ball_socket(BodyHandle::from_id(1), BodyHandle::from_id(2), Vec3::new(0.5, 0.0, 0.0), Vec3::ZERO);
    if let JointData::BallSocket(ref mut d) = j.data {
        d.use_cone_limit = true; d.cone_limit_angle = 0.5;
    }
    world.joints.push(j);
    let mut max_swing = 0f32;
    for _ in 0..240 { world.step(1.0 / 60.0).ok(); max_swing = max_swing.max(quat_angle(world.transforms[1].rotation)); }
    assert!(max_swing < 0.7, "ball-socket cone limit breached: {max_swing}");
}

#[test]
fn ballsocket_cone_limit_clamps_large_angle() {
    // Regression: the cone limit compared the saturating chord `2·|sin(θ/2)|` (max 2.0)
    // against a radian limit, so every limit ≥ 2 rad — including the constructor default π —
    // was silently inert. Here the limit is 2.618 rad (150°): the body must clamp near it,
    // not swing freely to ~π. (Old buggy code lets it reach ~π ≈ 3.14.)
    let mut world = PhysicsWorld::new().with_gravity(Vec3::ZERO);
    anchor(&mut world, 1, Vec3::ZERO);
    dyn_box(&mut world, 2, Vec3::new(0.5, 0.0, 0.0), Vec3::ZERO, Vec3::new(4.0, 0.0, 0.0), false);
    let mut j = Joint::ball_socket(BodyHandle::from_id(1), BodyHandle::from_id(2), Vec3::new(0.5, 0.0, 0.0), Vec3::ZERO);
    if let JointData::BallSocket(ref mut d) = j.data {
        d.use_cone_limit = true;
        d.cone_limit_angle = 2.618;
    }
    world.joints.push(j);
    let mut max_swing = 0f32;
    for _ in 0..240 { world.step(1.0 / 60.0).ok(); max_swing = max_swing.max(quat_angle(world.transforms[1].rotation)); }
    assert!(
        max_swing > 1.5 && max_swing < 2.95,
        "cone limit at 2.618 rad must clamp the swing (got {max_swing}); the old bug let it reach ~π"
    );
}

#[test]
fn spring_settles_at_rest_length() {
    let mut world = PhysicsWorld::new().with_gravity(Vec3::ZERO);
    dyn_box(&mut world, 1, Vec3::new(-2.0, 0.0, 0.0), Vec3::ZERO, Vec3::ZERO, false);
    dyn_box(&mut world, 2, Vec3::new(2.0, 0.0, 0.0), Vec3::ZERO, Vec3::ZERO, false);
    // separation 4 m, rest 1 m, stiff spring with damping → settles near rest length.
    world.joints.push(Joint::spring(BodyHandle::from_id(1), BodyHandle::from_id(2), Vec3::ZERO, Vec3::ZERO, 1.0, 30.0, 3.0));
    for _ in 0..600 { world.step(1.0 / 60.0).ok(); }
    let sep = (world.transforms[0].position - world.transforms[1].position).length();
    assert!((sep - 1.0).abs() < 0.2, "spring did not settle at rest length: {sep}");
}

fn dyn_ball(world: &mut PhysicsWorld, id: u32, pos: Vec3) {
    let mut rb = RigidBody::new(1.0, true);
    rb.wake_up();
    let col = Collider::sphere(0.1);
    rb.update_inertia_from_collider(&col);
    world.add_body(BodyHandle::from_id(id), rb, Transform::new(pos), Velocity::default(), col);
}

#[test]
fn rope_is_slack_when_short_and_catches_when_taut() {
    // Rope length 2; ball starts only 1 m below the anchor → SLACK (dist 1 < 2). A rope
    // exerts nothing while slack, so the ball must FREE-FALL until it reaches dist 2,
    // where the rope catches it. (A rigid rod would snap it out to 2 immediately — the
    // whole point of a rope is that it does not.)
    let mut world = PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0));
    anchor(&mut world, 1, Vec3::new(0.0, 10.0, 0.0)); // transforms[0]
    dyn_ball(&mut world, 2, Vec3::new(0.0, 9.0, 0.0)); // transforms[1]
    world.joints.push(Joint::rope(
        BodyHandle::from_id(1),
        BodyHandle::from_id(2),
        Vec3::ZERO,
        Vec3::ZERO,
        2.0,
    ));

    // Early: still slack ⇒ falling freely below its start, not yet at the rope's reach.
    for _ in 0..30 {
        world.step(1.0 / 240.0).ok();
    }
    let early = world.transforms[1].position;
    assert!(early.y < 9.0, "slack rope must let the ball fall, y={}", early.y);
    assert!(
        (early - Vec3::new(0.0, 10.0, 0.0)).length() < 2.0,
        "should still be slack early"
    );

    // Settle: the rope catches at length 2 and holds it there.
    for _ in 0..400 {
        world.step(1.0 / 240.0).ok();
    }
    let p = world.transforms[1].position;
    let dist = (p - Vec3::new(0.0, 10.0, 0.0)).length();
    assert!((1.9..=2.05).contains(&dist), "rope must catch/hold at length 2, dist={dist}");
    assert!(p.y < 8.3, "ball must have fallen to ~y=8 (2 below anchor), y={}", p.y);
}

#[test]
fn distance_rigid_rod_holds_exact_length() {
    // min == max == 2 ⇒ rigid rod: a ball starting 1 m below (dist 1 < min) is PUSHED
    // out to length 2 — unlike a rope, which would leave it slack. Direct contrast.
    let mut world = PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0));
    anchor(&mut world, 1, Vec3::new(0.0, 10.0, 0.0));
    dyn_ball(&mut world, 2, Vec3::new(0.0, 9.0, 0.0));
    world.joints.push(Joint::distance(
        BodyHandle::from_id(1),
        BodyHandle::from_id(2),
        Vec3::ZERO,
        Vec3::ZERO,
        2.0,
        2.0,
    ));
    for _ in 0..600 {
        world.step(1.0 / 240.0).ok();
    }
    let dist = (world.transforms[1].position - Vec3::new(0.0, 10.0, 0.0)).length();
    assert!((dist - 2.0).abs() < 0.15, "rigid rod holds length 2 (min=max), dist={dist}");
}

#[test]
fn spring_breaks_past_break_force() {
    // A heavily-stretched stiff spring with a tiny break_force must snap. (Regression:
    // solve_spring_joint used to never set is_broken, so break_force was a no-op on Spring.)
    let mut world = PhysicsWorld::new().with_gravity(Vec3::ZERO);
    dyn_box(&mut world, 1, Vec3::new(-2.0, 0.0, 0.0), Vec3::ZERO, Vec3::ZERO, false);
    dyn_box(&mut world, 2, Vec3::new(2.0, 0.0, 0.0), Vec3::ZERO, Vec3::ZERO, false);
    // sep 4, rest 0.5, stiffness 100 → force ≈ 350 ≫ break_force 5.
    let j = Joint::spring(
        BodyHandle::from_id(1),
        BodyHandle::from_id(2),
        Vec3::ZERO,
        Vec3::ZERO,
        0.5,
        100.0,
        1.0,
    )
    .with_break_force(5.0, 5.0);
    world.joints.push(j);
    world.step(1.0 / 240.0).ok();
    assert!(world.joints[0].is_broken, "spring must break when its force exceeds break_force");
}

#[test]
fn slider_servo_reaches_target_position() {
    // Position-servo motor: drive-and-hold a target offset along the slider axis.
    let mut world = PhysicsWorld::new().with_gravity(Vec3::ZERO);
    anchor(&mut world, 1, Vec3::ZERO); // static, transforms[0]
    dyn_box(&mut world, 2, Vec3::ZERO, Vec3::ZERO, Vec3::ZERO, false); // transforms[1]
    let mut j = Joint::slider(
        BodyHandle::from_id(1),
        BodyHandle::from_id(2),
        Vec3::ZERO,
        Vec3::ZERO,
        Vec3::X,
    );
    if let JointData::Slider(ref mut d) = j.data {
        d.use_motor = true;
        d.motor_is_servo = true;
        d.motor_target_position = 3.0;
        d.motor_max_force = 500.0;
    }
    world.joints.push(j);
    for _ in 0..800 {
        world.step(1.0 / 240.0).ok();
    }
    let x = world.transforms[1].position.x;
    assert!((x - 3.0).abs() < 0.3, "slider servo should reach & hold target 3.0, got x={x}");
}

#[test]
fn hinge_servo_reaches_target_angle() {
    // Position-servo motor on a hinge: rotate to and hold a target angle.
    let mut world = PhysicsWorld::new().with_gravity(Vec3::ZERO);
    anchor(&mut world, 1, Vec3::ZERO);
    dyn_box(&mut world, 2, Vec3::new(1.0, 0.0, 0.0), Vec3::ZERO, Vec3::ZERO, false);
    let mut j = Joint::hinge(
        BodyHandle::from_id(1),
        BodyHandle::from_id(2),
        Vec3::ZERO,
        Vec3::new(-1.0, 0.0, 0.0),
        Vec3::Z,
    );
    if let JointData::Hinge(ref mut d) = j.data {
        d.use_motor = true;
        d.motor_is_servo = true;
        d.motor_target_position = 1.0; // 1 rad
        d.motor_max_force = 500.0;
    }
    world.joints.push(j);
    for _ in 0..1000 {
        world.step(1.0 / 240.0).ok();
    }
    let angle = if let JointData::Hinge(d) = world.joints[0].data {
        d.current_angle
    } else {
        0.0
    };
    assert!((angle - 1.0).abs() < 0.2, "hinge servo should reach target angle 1.0 rad, got {angle}");
}

#[test]
fn ball_socket_twist_limit_clamps_roll() {
    // Cone-twist: a body spun about the twist axis must be stopped at the twist limit
    // (before this, BallSocket had only a swing/cone limit and rolled freely about its axis).
    let mut world = PhysicsWorld::new().with_gravity(Vec3::ZERO);
    anchor(&mut world, 1, Vec3::ZERO); // static, transforms[0]
    dyn_box(&mut world, 2, Vec3::ZERO, Vec3::ZERO, Vec3::new(0.0, 2.0, 0.0), false); // spin about Y
    let mut j = Joint::ball_socket(BodyHandle::from_id(1), BodyHandle::from_id(2), Vec3::ZERO, Vec3::ZERO);
    if let JointData::BallSocket(ref mut d) = j.data {
        d.use_twist_limit = true;
        d.twist_axis = Vec3::Y;
        d.twist_lower = -0.3;
        d.twist_upper = 0.3;
    }
    world.joints.push(j);
    // Track the peak twist: without the limit a +2 rad/s spin would free-run to several
    // radians; the limit must clamp |twist| at the 0.3 bound (never breach it).
    let mut max_twist = 0f32;
    for _ in 0..400 {
        world.step(1.0 / 240.0).ok();
        let q = world.transforms[1].rotation; // A static ⇒ B's world rotation is the twist
        max_twist = max_twist.max((2.0 * q.y.atan2(q.w)).abs());
    }
    assert!(max_twist > 0.25, "the twist limit must engage (B rolled to ~0.3), got max {max_twist}");
    assert!(max_twist < 0.4, "twist must never breach the 0.3 limit (no free-spin), got max {max_twist}");
}

#[test]
fn slider_spring_suspends_under_gravity() {
    // Suspension spring: a body on a vertical spring-slider must settle at the spring/
    // gravity equilibrium (K·|sag| = mg ⇒ sag ≈ -9.81/200 ≈ -0.049), NOT slide away as it
    // would with only hard limits + a velocity motor.
    let mut world = PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0));
    anchor(&mut world, 1, Vec3::ZERO);
    dyn_box(&mut world, 2, Vec3::ZERO, Vec3::ZERO, Vec3::ZERO, true); // gravity on
    let mut j = Joint::slider(
        BodyHandle::from_id(1),
        BodyHandle::from_id(2),
        Vec3::ZERO,
        Vec3::ZERO,
        Vec3::Y,
    );
    if let JointData::Slider(ref mut d) = j.data {
        d.use_spring = true;
        d.spring_stiffness = 200.0;
        d.spring_damping = 60.0; // over-damped → settles firmly
        d.spring_rest_position = 0.0;
    }
    world.joints.push(j);
    for _ in 0..800 {
        world.step(1.0 / 240.0).ok();
    }
    let y = world.transforms[1].position.y;
    assert!((y - (-0.049)).abs() < 0.05, "spring should suspend at ~-0.049, got y={y}");
    assert!(world.velocities[1].linear.y.abs() < 0.2, "should have settled, vy={}", world.velocities[1].linear.y);
}

#[test]
fn hinge_torsional_spring_returns_to_rest_angle() {
    // Return-to-center: a torsional spring must drive the hinge to its rest_angle and hold
    // (self-closing door / spring flap). B starts at angle 0; the spring pulls it to 0.8.
    let mut world = PhysicsWorld::new().with_gravity(Vec3::ZERO);
    anchor(&mut world, 1, Vec3::ZERO);
    dyn_box(&mut world, 2, Vec3::new(0.5, 0.0, 0.0), Vec3::ZERO, Vec3::ZERO, false);
    let mut j = Joint::hinge(
        BodyHandle::from_id(1),
        BodyHandle::from_id(2),
        Vec3::new(0.5, 0.0, 0.0),
        Vec3::ZERO,
        Vec3::Z,
    );
    if let JointData::Hinge(ref mut d) = j.data {
        d.use_torsional_spring = true;
        d.torsional_stiffness = 30.0;
        d.torsional_damping = 8.0; // over-damped → settles at rest without oscillating
        d.rest_angle = 0.8;
    }
    world.joints.push(j);
    for _ in 0..800 {
        world.step(1.0 / 240.0).ok();
    }
    let angle = if let JointData::Hinge(d) = world.joints[0].data {
        d.current_angle
    } else {
        0.0
    };
    assert!((angle - 0.8).abs() < 0.15, "torsional spring should settle at rest_angle 0.8, got {angle}");
    assert!(world.velocities[1].angular.z.abs() < 0.3, "should have settled, wz={}", world.velocities[1].angular.z);
}

#[test]
fn ball_socket_asymmetric_swing_limits() {
    // Elliptical/asymmetric cone: the swing about one perpendicular of the twist axis is
    // tightly limited (0.3) while the other is loose (1.0). Spinning about the tight axis
    // must clamp near 0.3 — a shoulder/hip range that the single circular cone can't express.
    let mut world = PhysicsWorld::new().with_gravity(Vec3::ZERO);
    anchor(&mut world, 1, Vec3::ZERO);
    dyn_box(&mut world, 2, Vec3::ZERO, Vec3::ZERO, Vec3::new(0.0, 0.0, 2.0), false); // spin about Z
    let mut j = Joint::ball_socket(BodyHandle::from_id(1), BodyHandle::from_id(2), Vec3::ZERO, Vec3::ZERO);
    if let JointData::BallSocket(ref mut d) = j.data {
        d.use_swing_limits = true;
        d.twist_axis = Vec3::Y;
        d.swing_limit_1 = 0.3; // about one perpendicular of Y (Z-ish) — tight
        d.swing_limit_2 = 1.0; // about the other (X-ish) — loose
    }
    world.joints.push(j);
    let mut max_z = 0f32;
    for _ in 0..400 {
        world.step(1.0 / 240.0).ok();
        let q = world.transforms[1].rotation;
        max_z = max_z.max((2.0 * q.z.atan2(q.w)).abs());
    }
    assert!(max_z > 0.2, "should swing up to the tight ~0.3 limit, got {max_z}");
    assert!(max_z < 0.45, "the 0.3 swing limit must clamp (not free-swing), got {max_z}");
}

#[test]
fn distance_compliance_makes_rope_stretch() {
    // Per-joint compliance: a rigid rope (compliance 0) holds its length under a hanging
    // load; a compliant one (>0) is elastic and stretches — the same primitive, softened.
    let rope_max = |compliance: f32| -> f32 {
        let mut world = PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0));
        anchor(&mut world, 1, Vec3::new(0.0, 10.0, 0.0));
        dyn_ball(&mut world, 2, Vec3::new(0.0, 8.0, 0.0)); // starts taut at length 2
        let mut j = Joint::rope(
            BodyHandle::from_id(1),
            BodyHandle::from_id(2),
            Vec3::ZERO,
            Vec3::ZERO,
            2.0,
        );
        if let JointData::Distance(ref mut d) = j.data {
            d.compliance = compliance;
        }
        world.joints.push(j);
        let mut max_d = 0f32;
        for _ in 0..600 {
            world.step(1.0 / 240.0).ok();
            max_d = max_d.max((world.transforms[1].position - Vec3::new(0.0, 10.0, 0.0)).length());
        }
        max_d
    };
    let rigid = rope_max(0.0);
    let elastic = rope_max(0.03);
    assert!(rigid < 2.15, "rigid rope must hold ~length 2, got {rigid}");
    assert!(elastic > rigid + 0.15, "compliant rope must stretch under load, rigid={rigid} elastic={elastic}");
}

#[test]
fn d6_as_slider_frees_one_linear_axis() {
    // Configure a generic D6 as a slider: X translation Free, everything else Locked.
    // A body pushed diagonally must slide along X but be held on Y and Z (and not rotate).
    let mut world = PhysicsWorld::new().with_gravity(Vec3::ZERO);
    anchor(&mut world, 1, Vec3::ZERO);
    dyn_box(&mut world, 2, Vec3::ZERO, Vec3::new(3.0, 3.0, 0.0), Vec3::new(0.0, 4.0, 0.0), false);
    let mut j = Joint::d6(BodyHandle::from_id(1), BodyHandle::from_id(2), Vec3::ZERO, Vec3::ZERO);
    if let JointData::D6(ref mut d) = j.data {
        d.linear[0] = D6Motion::Free; // X free ⇒ slider; Y,Z + all angular locked
    }
    world.joints.push(j);
    for _ in 0..240 {
        world.step(1.0 / 240.0).ok();
    }
    let p = world.transforms[1].position;
    assert!(p.x > 0.5, "D6 must allow sliding along the free X axis, x={}", p.x);
    assert!(p.y.abs() < 0.1, "D6 must lock the Y axis, y={}", p.y);
    assert!(p.z.abs() < 0.1, "D6 must lock the Z axis, z={}", p.z);
    assert!(quat_angle(world.transforms[1].rotation) < 0.15, "D6 all-angular-locked must not rotate");
}

#[test]
fn d6_angular_drive_reaches_target_angle() {
    // A D6 angular drive (spring-damper toward a target) on the free Z axis drives the
    // body to the target angle and holds it — a per-axis motor/spring on the D6.
    let mut world = PhysicsWorld::new().with_gravity(Vec3::ZERO);
    anchor(&mut world, 1, Vec3::ZERO);
    dyn_box(&mut world, 2, Vec3::ZERO, Vec3::ZERO, Vec3::ZERO, false);
    let mut j = Joint::d6(BodyHandle::from_id(1), BodyHandle::from_id(2), Vec3::ZERO, Vec3::ZERO);
    if let JointData::D6(ref mut d) = j.data {
        d.angular[2] = D6Motion::Free; // Z rotation free so the drive can turn it
        d.angular_drives[2] = D6Drive {
            enabled: true,
            stiffness: 40.0,
            damping: 8.0,
            target_position: 0.8,
            target_velocity: 0.0,
            max_force: 500.0,
        };
    }
    world.joints.push(j);
    for _ in 0..1000 {
        world.step(1.0 / 240.0).ok();
    }
    let q = world.transforms[1].rotation;
    let angle = 2.0 * q.z.atan2(q.w);
    assert!((angle - 0.8).abs() < 0.15, "D6 angular drive should reach target 0.8, got {angle}");
}

// ─────────────────────────────────────────────────────────────────────────────────────────
// Degenerate joint axes, and what the ball-socket limits actually measure.
// ─────────────────────────────────────────────────────────────────────────────────────────

/// **A slider with no axis must not latch NaN into its public state.**
///
/// `SliderJointData::axis` is a public field and the derived `Default` leaves it at
/// `Vec3::ZERO` — only `Joint::slider` substitutes `Vec3::Y`, and `..Default::default()` or a
/// direct field assignment walks straight past that. The solver normalised it bare, so
/// `axis_w` was a NaN vector and `current_position` — a public field, read by the servo —
/// was NaN on every pass.
///
/// Nothing worse happened, and that is the point: every downstream consumer is a `>`
/// comparison (`err.abs() > 1e-4`, `along < lower_limit`, `k > 1e-10`), and NaN loses all of
/// them. The containment was an accident of IEEE semantics, not a design. `current_position`
/// is the one place it escaped, so it is the one place a test can see it.
#[test]
fn slider_with_no_axis_reports_a_finite_position() {
    let mut world = PhysicsWorld::new().with_gravity(Vec3::ZERO);
    anchor(&mut world, 1, Vec3::ZERO);
    dyn_box(&mut world, 2, Vec3::new(0.5, 0.0, 0.0), Vec3::ZERO, Vec3::ZERO, false);
    let mut j = Joint::slider(
        BodyHandle::from_id(1),
        BodyHandle::from_id(2),
        Vec3::ZERO,
        Vec3::ZERO,
        Vec3::Y,
    );
    if let JointData::Slider(ref mut d) = j.data {
        // Exactly what `SliderJointData { .. }` + `Default` hands the solver.
        d.axis = Vec3::ZERO;
        d.use_limits = true; // and the limit rows must stay inert rather than compare to NaN
        d.lower_limit = -1.0;
        d.upper_limit = 1.0;
    }
    world.joints.push(j);
    for _ in 0..60 {
        world.step(1.0 / 60.0).ok();
    }

    let pos = match world.joints[0].data {
        JointData::Slider(d) => d.current_position,
        _ => unreachable!(),
    };
    assert!(
        pos.is_finite(),
        "a zero slider axis must not put NaN in `current_position` (got {pos}) — \
         `(rot * ZERO).normalize()` is a NaN vector"
    );

    // The bodies staying finite passes with or without the fix (NaN never reached them),
    // so this is a contract guard, NOT the discriminator. Keeping it honest: it is here to
    // catch a future gate rewritten as `!(x <= eps)`, which WOULD release the NaN.
    assert!(
        world.transforms[1].position.is_finite() && world.velocities[1].linear.is_finite(),
        "a degenerate axis must never reach a body"
    );
}

/// **A cone limit must not stop a pure twist — that is the twist limit's job.**
///
/// `cone_limit_angle` measured the angle of the *whole* deviation quaternion, so roll about
/// `twist_axis` spent the cone's budget even though the axis had not tipped at all: a ragdoll
/// limb rolling 30° about its own bone was 30° into a 45° cone it had never entered. The cone
/// now measures the SWING half of the swing–twist decomposition about `twist_axis`.
///
/// B is spun about `twist_axis` itself, so its swing is identically zero and the cone must
/// never engage. The old code clamped it at ~0.5 rad; free, it reaches ~2 rad in 1 s.
#[test]
fn cone_limit_ignores_pure_twist() {
    let spin = 2.0_f32;
    let steps = 240; // 1 s at 240 Hz ⇒ ~2 rad, comfortably under π (no wrap in the measure)

    let peak_twist = |use_cone: bool, use_twist: bool| -> f32 {
        let mut world = PhysicsWorld::new().with_gravity(Vec3::ZERO);
        anchor(&mut world, 1, Vec3::ZERO);
        // Anchors at both origins ⇒ the point constraint exerts no torque, so B's spin is
        // governed by the limit rows alone.
        dyn_box(&mut world, 2, Vec3::ZERO, Vec3::ZERO, Vec3::new(0.0, spin, 0.0), false);
        let mut j = Joint::ball_socket(
            BodyHandle::from_id(1),
            BodyHandle::from_id(2),
            Vec3::ZERO,
            Vec3::ZERO,
        );
        if let JointData::BallSocket(ref mut d) = j.data {
            d.twist_axis = Vec3::Y; // == the spin axis: pure twist, zero swing
            d.use_cone_limit = use_cone;
            d.cone_limit_angle = 0.5;
            d.use_twist_limit = use_twist;
            d.twist_lower = -0.5;
            d.twist_upper = 0.5;
        }
        world.joints.push(j);
        let mut peak = 0f32;
        for _ in 0..steps {
            world.step(1.0 / 240.0).ok();
            let q = world.transforms[1].rotation; // A is static ⇒ this IS the deviation
            peak = peak.max((2.0 * q.y.atan2(q.w)).abs());
        }
        peak
    };

    let cone_only = peak_twist(true, false);
    assert!(
        cone_only > 1.5,
        "a cone limit must not clamp a roll about its own axis (reached {cone_only} rad, \
         expected ~{spin}) — the cone was measuring the full deviation, twist included"
    );

    // Not vacuous in either direction: the twist limit DOES stop the same motion…
    let twist_only = peak_twist(false, true);
    assert!(
        twist_only < 0.7,
        "the twist limit must clamp this roll at 0.5 rad, got {twist_only}"
    );

    // …and the cone still stops a genuine SWING (a spin perpendicular to `twist_axis`), so
    // the first assertion is "the cone ignores twist", not "the cone is dead".
    let mut world = PhysicsWorld::new().with_gravity(Vec3::ZERO);
    anchor(&mut world, 1, Vec3::ZERO);
    dyn_box(&mut world, 2, Vec3::ZERO, Vec3::ZERO, Vec3::new(0.0, 0.0, spin), false);
    let mut j = Joint::ball_socket(
        BodyHandle::from_id(1),
        BodyHandle::from_id(2),
        Vec3::ZERO,
        Vec3::ZERO,
    );
    if let JointData::BallSocket(ref mut d) = j.data {
        d.twist_axis = Vec3::Y;
        d.use_cone_limit = true;
        d.cone_limit_angle = 0.5;
    }
    world.joints.push(j);
    let mut peak_swing = 0f32;
    for _ in 0..steps {
        world.step(1.0 / 240.0).ok();
        peak_swing = peak_swing.max(quat_angle(world.transforms[1].rotation));
    }
    assert!(
        peak_swing < 0.7,
        "the cone must still clamp a real swing at 0.5 rad, got {peak_swing}"
    );
}

/// **A swing limit written in radians must engage at that angle.**
///
/// The per-axis swing rows resolved `2·q.xyz` onto each perpendicular — the quaternion vector
/// part, i.e. `2·sin(θ/2)`, a CHORD — and compared it against a bound in radians. The chord
/// is always ≤ θ and saturates at 2.0, so every bound was too loose by a margin that grows
/// with it: a limit written as π/2 first engaged at `2·asin(π/4)` ≈ 1.80 rad ≈ 103°, and any
/// bound ≥ 2 rad could never engage at all.
///
/// The gap only opens up at large bounds, which is why the 0.3 rad limits elsewhere in this
/// file looked correct (0.30 vs 0.301). At π/2 it is 15°, and 1.70 rad sits between the two
/// answers: this test fails on the old code by measuring ~1.81 rad.
#[test]
fn swing_limit_engages_at_the_angle_it_is_written_in() {
    let limit = std::f32::consts::FRAC_PI_2;
    let mut world = PhysicsWorld::new().with_gravity(Vec3::ZERO);
    anchor(&mut world, 1, Vec3::ZERO);
    // Spin about Z. `perpendiculars(Y)` yields (−Z, −X), so this is a pure swing about the
    // FIRST perpendicular and `swing_limit_1` is the bound under test.
    dyn_box(&mut world, 2, Vec3::ZERO, Vec3::ZERO, Vec3::new(0.0, 0.0, 1.0), false);
    let mut j = Joint::ball_socket(
        BodyHandle::from_id(1),
        BodyHandle::from_id(2),
        Vec3::ZERO,
        Vec3::ZERO,
    );
    if let JointData::BallSocket(ref mut d) = j.data {
        d.use_swing_limits = true;
        d.twist_axis = Vec3::Y;
        d.swing_limit_1 = limit;
        d.swing_limit_2 = std::f32::consts::PI; // the other perpendicular: unconstrained
    }
    world.joints.push(j);
    let mut peak = 0f32;
    for _ in 0..600 {
        world.step(1.0 / 240.0).ok();
        let q = world.transforms[1].rotation;
        peak = peak.max((2.0 * q.z.atan2(q.w)).abs());
    }

    assert!(
        peak < 1.70,
        "a swing limit of π/2 = {limit:.4} rad must engage near π/2, got {peak} — the old \
         code compared the chord 2·sin(θ/2) against it and only engaged at ~1.80 rad"
    );
    assert!(
        peak > 1.40,
        "…and it must still let the body swing UP to the limit (got {peak}); a limit that \
         engages far too early would pass the assertion above for the wrong reason"
    );
}

/// The same chord-vs-radian defect on the D6's angular limits, which `D6Motion::Limited`
/// documents in radians. `Locked` axes are deliberately NOT converted — a lock is driven to
/// zero, where the quaternion vector part IS the standard error term and the two measures
/// agree to third order.
///
/// Fails on the old code at ~1.81 rad, the angle at which `2·sin(θ/2)` first exceeds π/2.
#[test]
fn d6_angular_limit_engages_at_the_angle_it_is_written_in() {
    let limit = std::f32::consts::FRAC_PI_2;
    let mut world = PhysicsWorld::new().with_gravity(Vec3::ZERO);
    anchor(&mut world, 1, Vec3::ZERO);
    dyn_box(&mut world, 2, Vec3::ZERO, Vec3::ZERO, Vec3::new(0.0, 0.0, 1.0), false);
    let mut j = Joint::d6(BodyHandle::from_id(1), BodyHandle::from_id(2), Vec3::ZERO, Vec3::ZERO);
    if let JointData::D6(ref mut d) = j.data {
        // Z limited, X and Y left Locked (so the spin stays a pure Z rotation).
        d.angular[2] = D6Motion::Limited { lower: -limit, upper: limit };
    }
    world.joints.push(j);
    let mut peak = 0f32;
    for _ in 0..600 {
        world.step(1.0 / 240.0).ok();
        let q = world.transforms[1].rotation;
        peak = peak.max((2.0 * q.z.atan2(q.w)).abs());
    }
    assert!(
        peak < 1.70,
        "a D6 angular limit of π/2 = {limit:.4} rad must engage near π/2, got {peak}"
    );
    assert!(
        peak > 1.40,
        "…and must still allow rotation up to it, got {peak}"
    );
}
