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
        let deviation = initial_rot.inverse() * relative_rot;
        // Canonicalise to w ≥ 0 once — a quaternion and its negation are the same rotation,
        // and all three blocks below assumed it separately before.
        let q = if deviation.w < 0.0 { -deviation } else { deviation };

        // ── Swing–twist decomposition about `twist_axis` ──────────────────────────
        // The cone and the per-axis swing limits both need the SWING alone. Measuring the
        // whole deviation made twist eat the cone's budget: a limb rolling 30° about its own
        // bone, with its axis not tipped at all, spent 30° of the cone. `twist_axis` is the
        // axis the cone is *around*, so it is the axis to decompose about; when it is unset
        // (the derived `Default`) `swing_about` returns the whole deviation, which is the
        // total-angle measure the cone had before and the only thing an axis-less cone can
        // mean. The twist row below is untouched: `2·atan2(v·a, w)` is already exactly the
        // angle of the decomposition's twist factor, so the two halves agree by construction.
        let cone_axis = if data.twist_axis.length_squared() > 1e-6 {
            data.twist_axis.normalize()
        } else {
            Vec3::ZERO
        };
        let swing_quat = Self::swing_about(q, cone_axis);
        // Direction of the swing: the quaternion vector part, which points along the swing
        // axis. Its LENGTH is the chord 2·|sin(θ/2)|, kept only as the "is there a direction
        // at all" gate it always was — never as an angle (see below).
        let swing_err_local = Vec3::new(swing_quat.x, swing_quat.y, swing_quat.z) * 2.0;
        let swing_mag = swing_err_local.length();
        let swing_axis = if swing_mag >= 1e-6 {
            swing_err_local / swing_mag
        } else {
            Vec3::ZERO
        };
        // TRUE swing angle θ = 2·acos(|w|), in RADIANS. The chord saturates at 2.0, so it
        // cannot be compared to a radian limit at all — and where it does not saturate it is
        // still 2·sin(θ/2) < θ, i.e. systematically permissive.
        let needs_swing = data.use_cone_limit || data.use_swing_limits;
        let swing_angle = if needs_swing {
            2.0 * swing_quat.w.abs().clamp(0.0, 1.0).acos()
        } else {
            0.0
        };
        // The swing as a rotation VECTOR (axis · angle, radians) — the quantity the per-axis
        // swing rows resolve onto the two perpendiculars.
        let swing_rot_vec = swing_axis * swing_angle;

        // ── Cone (swing) limit — clamps how far B's axis tips away from its initial pose ──
        if data.use_cone_limit && swing_angle > data.cone_limit_angle && swing_mag >= 1e-6 {
            let excess = swing_angle - data.cone_limit_angle;
            let swing_dir_world = transforms[idx_a].rotation * swing_axis;
            self.apply_angular_constraint_soft(
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
                &mut joint.scratch, row::ANG,
            );
        }

        // ── Twist (roll) limit — the twist half of the same decomposition ──
        // Isolate the roll about `twist_axis`: project the quaternion's vector part onto
        // the axis; the twist angle is 2·atan2(proj, w). Two-sided clamp like a hinge limit.
        // This reads the FULL deviation `q`, never `swing_quat` — the swing is by
        // construction the part with no roll about this axis, so measuring the twist on it
        // would always report ≈0 and the row would never engage.
        if data.use_twist_limit && data.twist_axis.length_squared() > 1e-6 {
            let axis_local = data.twist_axis.normalize();
            let proj = Vec3::new(q.x, q.y, q.z).dot(axis_local);
            let twist_angle = 2.0 * proj.atan2(q.w);
            let axis_world = transforms[idx_a].rotation * axis_local;
            if twist_angle > data.twist_upper {
                self.apply_angular_constraint_soft(
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
                        &mut joint.scratch, row::ANG + 1,
                    );
            } else if twist_angle < data.twist_lower {
                self.apply_angular_constraint_soft(
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
                        &mut joint.scratch, row::ANG + 1,
                    );
            }
        }

        // ── Asymmetric per-axis swing limits (about the two perpendiculars of twist_axis) ──
        // Clamp the swing about each perp independently, so a shoulder/hip can have a
        // different range in each direction (an elliptical/box cone vs the circular one).
        if data.use_swing_limits && data.twist_axis.length_squared() > 1e-6 {
            let axis_local = data.twist_axis.normalize();
            let (perp1, perp2) = Self::perpendiculars(axis_local);
            for (i, (perp, limit)) in [(perp1, data.swing_limit_1), (perp2, data.swing_limit_2)]
                .into_iter()
                .enumerate()
            {
                // Swing angle about this perpendicular, in RADIANS, resolved from the swing
                // ROTATION VECTOR. It used to resolve `2·q.xyz` — the quaternion vector part
                // of the whole deviation, which is `2·sin(θ/2)` along the axis, not θ. Being
                // a chord it is always ≤ θ and saturates at 2.0, so every bound was too
                // permissive by a margin that grows with the bound: a limit written as π/2
                // (90°) first engaged at 2·asin(π/4) ≈ 1.80 rad ≈ 103°, and any bound ≥ 2 rad
                // could never be reached at all. Below ~0.3 rad the two agree to ~0.4%, which
                // is why small ragdoll limits looked right.
                let a = swing_rot_vec.dot(perp);
                let perp_world = transforms[idx_a].rotation * perp;
                if a > limit {
                    self.apply_angular_constraint_soft(
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
                            &mut joint.scratch, row::SWING + i,
                        );
                } else if a < -limit {
                    self.apply_angular_constraint_soft(
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
                            &mut joint.scratch, row::SWING + i,
                        );
                }
            }
        }

    }
}
