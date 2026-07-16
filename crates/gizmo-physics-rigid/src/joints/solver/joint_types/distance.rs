//! Distance/rope-joint solver — extracted verbatim from the former 1236-line joint_types.rs.
//! One `impl JointSolver` block per joint kind; `pub(crate)` methods stay callable
//! from `solve_joints` regardless of file (inherent impls compose across modules).

use super::super::*;

impl JointSolver {

    /// Distance/rope joint: a 1-DOF LINEAR limit along the (dynamic) anchor-to-anchor
    /// direction. It is an INEQUALITY — force is applied only when a bound is violated.
    /// `length > max_length` (rope taut) pulls together only (`lambda ≤ 0`); `length <
    /// min_length` (rod floor) pushes apart only (`lambda ≥ 0`); between the bounds the
    /// bodies are free (a slack rope exerts nothing). Mirrors the cone-limit pattern:
    /// negative error + a one-sided lambda clamp on `apply_linear_constraint`.
    pub(crate) fn solve_distance_joint(
        &self,
        joint: &mut Joint,
        rigid_bodies: &[RigidBody],
        transforms: &[Transform],
        velocities: &mut [Velocity],
        idx_a: usize,
        idx_b: usize,
        dt: f32,
    ) {
        let JointData::Distance(data) = joint.data else {
            return;
        };

        let anchor_a =
            transforms[idx_a].position + transforms[idx_a].rotation * joint.local_anchor_a;
        let anchor_b =
            transforms[idx_b].position + transforms[idx_b].rotation * joint.local_anchor_b;
        let diff = anchor_b - anchor_a;
        let length = diff.length();
        if length < 1e-6 {
            return; // degenerate: direction undefined
        }
        let n = diff / length; // A→B
        let r_a = anchor_a - transforms[idx_a].position;
        let r_b = anchor_b - transforms[idx_b].position;

        // error = target - current (so a violated UPPER bound gives a negative error,
        // driving a negative — i.e. pulling-together — lambda, exactly like the cone limit).
        let lin_impulse = if length > data.max_length {
            self.apply_linear_constraint_soft(
                rigid_bodies,
                transforms,
                velocities,
                idx_a,
                idx_b,
                n,
                r_a,
                r_b,
                data.max_length - length, // < 0
                dt,
                f32::NEG_INFINITY,
                0.0, // pull only
                data.compliance, // 0 = rigid rope; >0 = elastic/stretchy
            )
        } else if length < data.min_length {
            self.apply_linear_constraint_soft(
                rigid_bodies,
                transforms,
                velocities,
                idx_a,
                idx_b,
                n,
                r_a,
                r_b,
                data.min_length - length, // > 0
                dt,
                0.0, // push only
                f32::INFINITY,
                data.compliance,
            )
        } else {
            0.0 // within bounds → free
        };

        if lin_impulse.abs() / dt > joint.break_force {
            joint.is_broken = true;
            tracing::debug!(
                entity_a = ?joint.entity_a,
                entity_b = ?joint.entity_b,
                applied_force = lin_impulse.abs() / dt,
                break_force = joint.break_force,
                "Distance joint broke (force exceeded break threshold)"
            );
        }
    }
}
