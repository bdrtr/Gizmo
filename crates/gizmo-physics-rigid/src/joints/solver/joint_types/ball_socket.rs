//! Ball-socket-joint solver — extracted verbatim from the former 1236-line joint_types.rs.
//! One `impl JointSolver` block per joint kind; `pub(crate)` methods stay callable
//! from `solve_joints` regardless of file (inherent impls compose across modules).

use super::super::*;

impl JointSolver {

    pub(crate) fn solve_ball_socket_joint(
        &self,
        joint: &mut Joint,
        rigid_bodies: &[RigidBody],
        transforms: &[Transform],
        velocities: &mut [Velocity],
        idx_a: usize,
        idx_b: usize,
        dt: f32,
    ) {
        // 1. Position constraint
        self.solve_fixed_joint(
            joint,
            rigid_bodies,
            transforms,
            velocities,
            idx_a,
            idx_b,
            dt,
        );

        let JointData::BallSocket(ref mut data) = joint.data else {
            return;
        };
        if !data.use_cone_limit && !data.use_twist_limit && !data.use_swing_limits {
            return;
        }

        // 2. Initialise reference rotation on first solve
        let relative_rot = transforms[idx_a].rotation.inverse() * transforms[idx_b].rotation;
        let initial_rot = match data.initial_relative_rotation {
            None => {
                data.initial_relative_rotation = Some(relative_rot);
                return;
            }
            Some(rot) => rot,
        };

        // Rotation of B away from its initial orientation, in A's frame.
        let swing_quat = initial_rot.inverse() * relative_rot;
        let mut total_ang_impulse = 0.0;

        // ── Cone (swing) limit — clamps how far B rotates from its initial pose ──
        if data.use_cone_limit {
            // Angular-error DIRECTION from the small-angle approximation (2·quat.xyz).
            let swing_err_local = if swing_quat.w >= 0.0 {
                Vec3::new(swing_quat.x, swing_quat.y, swing_quat.z) * 2.0
            } else {
                -Vec3::new(swing_quat.x, swing_quat.y, swing_quat.z) * 2.0
            };
            // TRUE swing angle θ = 2·acos(|w|) (the chord length saturates at 2.0, so it
            // cannot be compared to a radian limit directly).
            let swing_mag = swing_err_local.length();
            let swing_angle = 2.0 * swing_quat.w.abs().clamp(0.0, 1.0).acos();
            if swing_angle > data.cone_limit_angle && swing_mag >= 1e-6 {
                let excess = swing_angle - data.cone_limit_angle;
                let swing_dir_world = transforms[idx_a].rotation * (swing_err_local / swing_mag);
                total_ang_impulse += self
                    .apply_angular_constraint_soft(
                        rigid_bodies,
                        transforms,
                        velocities,
                        idx_a,
                        idx_b,
                        swing_dir_world,
                        -excess,
                        dt,
                        f32::NEG_INFINITY,
                        0.0,
                        data.compliance,
                    )
                    .abs();
            }
        }

        // ── Twist (roll) limit — swing-twist decomposition about twist_axis ──
        // Isolate the roll about `twist_axis`: project the quaternion's vector part onto
        // the axis; the twist angle is 2·atan2(proj, w). Two-sided clamp like a hinge limit.
        if data.use_twist_limit && data.twist_axis.length_squared() > 1e-6 {
            let axis_local = data.twist_axis.normalize();
            // Canonicalise to w ≥ 0 (a quaternion and its negation are the same rotation).
            let q = if swing_quat.w < 0.0 { -swing_quat } else { swing_quat };
            let proj = Vec3::new(q.x, q.y, q.z).dot(axis_local);
            let twist_angle = 2.0 * proj.atan2(q.w);
            let axis_world = transforms[idx_a].rotation * axis_local;
            if twist_angle > data.twist_upper {
                total_ang_impulse += self
                    .apply_angular_constraint_soft(
                        rigid_bodies,
                        transforms,
                        velocities,
                        idx_a,
                        idx_b,
                        axis_world,
                        data.twist_upper - twist_angle, // < 0
                        dt,
                        f32::NEG_INFINITY,
                        0.0,
                        data.compliance,
                    )
                    .abs();
            } else if twist_angle < data.twist_lower {
                total_ang_impulse += self
                    .apply_angular_constraint_soft(
                        rigid_bodies,
                        transforms,
                        velocities,
                        idx_a,
                        idx_b,
                        axis_world,
                        data.twist_lower - twist_angle, // > 0
                        dt,
                        0.0,
                        f32::INFINITY,
                        data.compliance,
                    )
                    .abs();
            }
        }

        // ── Asymmetric per-axis swing limits (about the two perpendiculars of twist_axis) ──
        // Clamp the swing about each perp independently, so a shoulder/hip can have a
        // different range in each direction (an elliptical/box cone vs the circular one).
        if data.use_swing_limits && data.twist_axis.length_squared() > 1e-6 {
            let axis_local = data.twist_axis.normalize();
            let (perp1, perp2) = Self::perpendiculars(axis_local);
            // Swing rotation vector (small-angle: 2·xyz), canonicalised to w ≥ 0.
            let q = if swing_quat.w < 0.0 { -swing_quat } else { swing_quat };
            let rvec = 2.0 * Vec3::new(q.x, q.y, q.z);
            for (perp, limit) in [(perp1, data.swing_limit_1), (perp2, data.swing_limit_2)] {
                let a = rvec.dot(perp); // swing angle about this perpendicular
                let perp_world = transforms[idx_a].rotation * perp;
                if a > limit {
                    total_ang_impulse += self
                        .apply_angular_constraint_soft(
                            rigid_bodies,
                            transforms,
                            velocities,
                            idx_a,
                            idx_b,
                            perp_world,
                            limit - a, // < 0
                            dt,
                            f32::NEG_INFINITY,
                            0.0,
                            data.compliance,
                        )
                        .abs();
                } else if a < -limit {
                    total_ang_impulse += self
                        .apply_angular_constraint_soft(
                            rigid_bodies,
                            transforms,
                            velocities,
                            idx_a,
                            idx_b,
                            perp_world,
                            -limit - a, // > 0
                            dt,
                            0.0,
                            f32::INFINITY,
                            data.compliance,
                        )
                        .abs();
                }
            }
        }

        if total_ang_impulse / dt > joint.break_torque {
            joint.is_broken = true;
            tracing::debug!(
                entity_a = ?joint.entity_a,
                entity_b = ?joint.entity_b,
                applied_torque = total_ang_impulse / dt,
                break_torque = joint.break_torque,
                "Ball-socket joint broke (torque exceeded break threshold)"
            );
        }
    }
}
