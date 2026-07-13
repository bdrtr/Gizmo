//! Fixed-joint solver — extracted verbatim from the former 1236-line joint_types.rs.
//! One `impl JointSolver` block per joint kind; `pub(crate)` methods stay callable
//! from `solve_joints` regardless of file (inherent impls compose across modules).

use super::super::*;

impl JointSolver {
    pub(crate) fn solve_fixed_joint(
        &self,
        joint: &mut Joint,
        rigid_bodies: &[RigidBody],
        transforms: &[Transform],
        velocities: &mut [Velocity],
        idx_a: usize,
        idx_b: usize,
        dt: f32,
    ) {
        let anchor_a =
            transforms[idx_a].position + transforms[idx_a].rotation * joint.local_anchor_a;
        let anchor_b =
            transforms[idx_b].position + transforms[idx_b].rotation * joint.local_anchor_b;
        let error = anchor_a - anchor_b; // target = a, current = b, so error = a - b
        let err_len = error.length();

        // Position (point) constraint — shared with the hinge / ball-socket position
        // stage. Skip only the LINEAR part when the anchor is already coincident; the
        // Fixed angular lock below must still run (earlier an early `return` here let a
        // perfectly-pinned Fixed joint spin freely).
        if err_len >= 0.0001 {
            let r_a = anchor_a - transforms[idx_a].position;
            let r_b = anchor_b - transforms[idx_b].position;

            let max_impulse = f32::MAX;
            let min_impulse = f32::MIN;

            let mut impulse_sum = 0.0;
            impulse_sum += self
                .apply_linear_constraint(
                    rigid_bodies,
                    transforms,
                    velocities,
                    idx_a,
                    idx_b,
                    Vec3::new(1.0, 0.0, 0.0),
                    r_a,
                    r_b,
                    error.x,
                    dt,
                    min_impulse,
                    max_impulse,
                )
                .abs();
            impulse_sum += self
                .apply_linear_constraint(
                    rigid_bodies,
                    transforms,
                    velocities,
                    idx_a,
                    idx_b,
                    Vec3::new(0.0, 1.0, 0.0),
                    r_a,
                    r_b,
                    error.y,
                    dt,
                    min_impulse,
                    max_impulse,
                )
                .abs();
            impulse_sum += self
                .apply_linear_constraint(
                    rigid_bodies,
                    transforms,
                    velocities,
                    idx_a,
                    idx_b,
                    Vec3::new(0.0, 0.0, 1.0),
                    r_a,
                    r_b,
                    error.z,
                    dt,
                    min_impulse,
                    max_impulse,
                )
                .abs();

            if impulse_sum / dt > joint.break_force {
                joint.is_broken = true;
            }
        }

        // Angular lock — a genuine Fixed joint must ALSO prevent relative rotation.
        // The point constraint above only pins an anchor, leaving the bodies free to
        // spin around it (so "Fixed" behaved like a ball-socket). Drive the relative
        // angular velocity to zero on all three axes. `solve_fixed_joint` is reused by
        // the hinge/ball-socket position stage, so this gate keeps the lock exclusive
        // to real Fixed joints (which allow no relative DOF). Velocity-level lock: the
        // solver runs every sub-step before integration, so no relative rotation
        // accumulates; the joint stays welded.
        if matches!(joint.data, JointData::Fixed) {
            let mut total_ang_impulse = 0.0;
            for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
                total_ang_impulse += self
                    .apply_angular_constraint(
                        rigid_bodies,
                        transforms,
                        velocities,
                        idx_a,
                        idx_b,
                        axis,
                        0.0,
                        dt,
                        f32::NEG_INFINITY,
                        f32::INFINITY,
                    )
                    .abs();
            }
            // Break the weld under excessive torsional load. The hinge/ball-socket/slider
            // solvers all honor break_torque, but the Fixed angular lock previously
            // discarded every lambda, so a Fixed joint could never break no matter how small
            // break_torque was (the with_break_force(force, torque) API silently no-op'd its
            // torque argument). Checked outside the linear-error gate so a perfectly-pinned
            // weld — which carries its whole reaction through this angular lock — still breaks.
            if total_ang_impulse / dt > joint.break_torque {
                joint.is_broken = true;
            }
        }
    }
}
