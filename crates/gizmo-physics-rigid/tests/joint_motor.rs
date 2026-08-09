//! `iterations` is a solver-quality knob for motors too.
//!
//! The hinge and slider motors are the only joint rows that never went through the shared
//! constraint helpers. They hand-rolled their own budget as
//! `motor_max_force * dt / self.iterations`, with a comment saying so: without the divisor
//! each iteration would clamp its own share and the motor would apply roughly `iterations`
//! times too much force. The divisor was a hand-written stand-in for the accumulator the
//! solver did not have.
//!
//! It bounds the total correctly, but it makes the motor's effect a FUNCTION of the
//! iteration count rather than something the iteration count refines: the motor can only
//! ramp to its budget over `N` iterations, and cannot give any of it back if it overshoots.
//! Now that a real accumulated total exists, the budget applies to the total and the motor
//! reaches — and returns — force in one iteration.
//!
//! The observable consequence, and what this file asserts: the answer **converges**.

use gizmo_math::Vec3;
use gizmo_physics_core::BodyHandle;
use gizmo_physics_core::{Collider, Transform};
use gizmo_physics_rigid::{Joint, JointData, PhysicsWorld, RigidBody, Velocity};

/// A 4 kg arm on a hinge, held up only by its motor against gravity — so the motor is
/// genuinely load-limited rather than coasting to its target with force to spare. Returns
/// (peak angle, final angle).
fn loaded_arm(motor_max_force: f32, servo: bool, iterations: usize) -> (f32, f32) {
    let mut world = PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0));
    world.joint_solver.iterations = iterations;

    let mut a = RigidBody::new_static();
    a.wake_up();
    world.add_body(
        BodyHandle::from_id(1),
        a,
        Transform::new(Vec3::ZERO),
        Velocity::default(),
        Collider::sphere(0.1),
    );

    let mut rb = RigidBody::new(4.0, true);
    rb.wake_up();
    let col = Collider::box_collider(Vec3::new(0.8, 0.1, 0.1));
    rb.update_inertia_from_collider(&col);
    world.add_body(
        BodyHandle::from_id(2),
        rb,
        Transform::new(Vec3::new(0.8, 0.0, 0.0)),
        Velocity::default(),
        col,
    );

    let mut j = Joint::hinge(
        BodyHandle::from_id(1),
        BodyHandle::from_id(2),
        Vec3::ZERO,
        Vec3::new(-0.8, 0.0, 0.0),
        Vec3::Z,
    );
    if let JointData::Hinge(ref mut d) = j.data {
        d.use_motor = true;
        d.motor_max_force = motor_max_force;
        d.motor_is_servo = servo;
        d.motor_target_position = 1.2;
        d.motor_target_velocity = 6.0;
    }
    world.joints.push(j);

    let mut peak = 0f32;
    let mut tail = 0f32;
    for _ in 0..300 {
        world.step(1.0 / 60.0).ok();
        if let JointData::Hinge(d) = world.joints[0].data {
            peak = peak.max(d.current_angle);
            tail = d.current_angle;
        }
    }
    (peak, tail)
}

/// Doubling the iteration count on an already-converged solve must not move the answer.
///
/// This is the discriminating property. Under the old `/ iterations` budget the motor's
/// contribution scaled with the loop it ran in, so the result never settled — it drifted,
/// and not even monotonically:
///
/// ```text
///                       5 iters      10          20          40
///   loaded servo peak   1.7588705   1.7593216   1.7595481   1.7595280
///   stalled servo tail -0.6354679  -0.6352561  -0.6352435  -0.6356549
///   velocity motor tail -0.3462713  -0.3463851  -0.3464802  -0.3465131
/// ```
///
/// With the budget on the accumulated total, all three are settled by 20 iterations and 40
/// reproduces them exactly. A user raising `iterations` for stability is asking for a better
/// answer to the same problem, not a different problem.
///
/// # The tolerance was 1e-6 until 2026-08-09
///
/// Rigid joint rows became soft constraints at `JointSolver::rigid_hertz`, and the hinge's
/// point rows are rigid ones. Two of the three regimes still settle to the last bit; the
/// saturated velocity motor does not quite —
///
/// ```text
///   tail angle, hz = 200:  20 -0.34664080   40 -0.34664220   80 -0.34664255   160 -0.34664220
///   step-to-step delta:            1.40e-6          3.58e-7          3.58e-7
/// ```
///
/// The residual stops shrinking at 3.6e-7 (≈12 f32 ulps at 0.35 rad) and stays there through
/// 160 iterations, so it is the noise floor and not an unconverged answer. 5e-6 is set above
/// that floor and still catches the bug this file was written for by 48× — the old
/// `/ iterations` budget drifted the same number by 2.4e-4 between 5 and 40 iterations.
#[test]
fn a_force_limited_motor_converges_as_iterations_rise() {
    // Three regimes: reaching its target with force to spare, stalled under load, and a
    // plain velocity motor that cannot hold the arm up.
    for (name, force, servo) in [
        ("servo with headroom", 60.0_f32, true),
        ("stalled servo", 25.0, true),
        ("saturated velocity motor", 8.0, false),
    ] {
        let (peak_20, tail_20) = loaded_arm(force, servo, 20);
        let (peak_40, tail_40) = loaded_arm(force, servo, 40);

        assert!(
            (peak_20 - peak_40).abs() < 5e-6,
            "{name}: peak angle moved when iterations doubled 20 -> 40 \
             ({peak_20:.8} vs {peak_40:.8}) — `iterations` must refine the answer, not change it"
        );
        assert!(
            (tail_20 - tail_40).abs() < 5e-6,
            "{name}: final angle moved when iterations doubled 20 -> 40 \
             ({tail_20:.8} vs {tail_40:.8})"
        );
    }
}

/// …and the convergence is toward something, not just a fixed point of a broken map: a
/// motor with more force must get further against the same load.
#[test]
fn a_stronger_motor_lifts_the_arm_further() {
    let (_, weak) = loaded_arm(25.0, true, 10);
    let (_, strong) = loaded_arm(60.0, true, 10);
    assert!(
        strong > weak + 1.0,
        "a 60 N·m motor must lift the arm well past a 25 N·m one (weak={weak:.4}, \
         strong={strong:.4})"
    );
    assert!(
        (strong - 1.2).abs() < 0.05,
        "the stronger servo must actually reach its 1.2 rad target, got {strong:.4}"
    );
}
