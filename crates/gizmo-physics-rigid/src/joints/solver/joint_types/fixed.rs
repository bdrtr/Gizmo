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
            let r_a = Self::lever_arm(rigid_bodies, transforms, idx_a, anchor_a);
            let r_b = Self::lever_arm(rigid_bodies, transforms, idx_b, anchor_b);

            let max_impulse = f32::MAX;
            let min_impulse = f32::MIN;

            self.apply_linear_constraint(
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
                    &mut joint.scratch, row::LIN,
                );
            self.apply_linear_constraint(
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
                    &mut joint.scratch, row::LIN + 1,
                );
            self.apply_linear_constraint(
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
                    &mut joint.scratch, row::LIN + 2,
                );

        }

        // Angular lock — a genuine Fixed joint must ALSO prevent relative rotation.
        // The point constraint above only pins an anchor, leaving the bodies free to
        // spin around it (so "Fixed" behaved like a ball-socket). Drive the relative
        // angular velocity to zero on all three axes. `solve_fixed_joint` is reused by
        // the hinge/ball-socket position stage, so this gate keeps the lock exclusive
        // to real Fixed joints (which allow no relative DOF). Velocity-level lock: the
        // solver runs every sub-step before integration, so no relative rotation
        // accumulates; the joint stays welded.
        //
        // The literal `0.0` below is LOAD-BEARING, and `apply_angular_constraint_soft` keys
        // off it: a row with no position term must stay a HARD velocity constraint even when
        // `rigid_hertz > 0`, because a soft row leaves `impulse_scale · v` behind and there
        // is no restoring term here to take it back — the weld angle would then integrate
        // linearly and without bound under any sustained torque (measured at 7.3° in 40 s
        // under 10 rad/s² before that branch existed). `JointData::Fixed` carries no
        // reference pose to servo toward, which is why this is a velocity lock in the first
        // place; giving it one is a breaking change to a public enum and is tracked in
        // docs/FIXPLAN.md rather than done here.
        if matches!(joint.data, JointData::Fixed) {
            for (i, axis) in [Vec3::X, Vec3::Y, Vec3::Z].into_iter().enumerate() {
                self.apply_angular_constraint(
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
                        &mut joint.scratch, row::ANG + i,
                    );
            }
        }
    }
}
