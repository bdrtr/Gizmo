use super::*;

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

    pub(crate) fn solve_hinge_joint(
        &self,
        joint: &mut Joint,
        rigid_bodies: &[RigidBody],
        transforms: &[Transform],
        velocities: &mut [Velocity],
        idx_a: usize,
        idx_b: usize,
        dt: f32,
    ) {
        // 1. Position constraint — keep anchor points together
        self.solve_fixed_joint(
            joint,
            rigid_bodies,
            transforms,
            velocities,
            idx_a,
            idx_b,
            dt,
        );

        let JointData::Hinge(ref mut data) = joint.data else {
            return;
        };

        let rot_a = transforms[idx_a].rotation;
        let rot_b = transforms[idx_b].rotation;
        let axis_a = rot_a * data.axis;
        let axis_b = rot_b * data.axis;

        // 2. Angular constraint — keep hinge axes aligned (2 DOF)
        let ang_err = axis_a.cross(axis_b);
        let err_mag = ang_err.length();
        let mut total_ang_impulse = 0.0;
        if err_mag > 1e-6 {
            let err_dir = ang_err / err_mag;
            total_ang_impulse += self
                .apply_angular_constraint(
                    rigid_bodies,
                    transforms,
                    velocities,
                    idx_a,
                    idx_b,
                    err_dir,
                    -err_mag,
                    dt,
                    f32::NEG_INFINITY,
                    f32::INFINITY,
                )
                .abs();
        }

        // 3. Track current angle
        let ref_local = if data.axis.cross(Vec3::X).length() > 0.1 {
            data.axis.cross(Vec3::X).normalize()
        } else {
            data.axis.cross(Vec3::Y).normalize()
        };

        let rot_a = transforms[idx_a].rotation;
        let rot_b = transforms[idx_b].rotation;
        let axis_w = rot_a * data.axis;
        let ref_a_w = rot_a * ref_local;
        let ref_b_w = rot_b * ref_local;

        let proj_a = (ref_a_w - axis_w * ref_a_w.dot(axis_w)).normalize_or_zero();
        let proj_b = (ref_b_w - axis_w * ref_b_w.dot(axis_w)).normalize_or_zero();

        if proj_a.length_squared() > 0.01 && proj_b.length_squared() > 0.01 {
            let cos_a = proj_a.dot(proj_b).clamp(-1.0, 1.0);
            let sign = if proj_a.cross(proj_b).dot(axis_w) >= 0.0 {
                1.0_f32
            } else {
                -1.0
            };
            data.current_angle = sign * cos_a.acos();

            // 4. Angle limits
            if data.use_limits {
                if data.current_angle < data.lower_limit {
                    let err = data.lower_limit - data.current_angle;
                    // axis_w points from A to B; positive lambda increases angle
                    total_ang_impulse += self
                        .apply_angular_constraint(
                            rigid_bodies,
                            transforms,
                            velocities,
                            idx_a,
                            idx_b,
                            axis_w,
                            err,
                            dt,
                            0.0,
                            f32::INFINITY,
                        )
                        .abs();
                } else if data.current_angle > data.upper_limit {
                    let err = data.upper_limit - data.current_angle; // negative
                    total_ang_impulse += self
                        .apply_angular_constraint(
                            rigid_bodies,
                            transforms,
                            velocities,
                            idx_a,
                            idx_b,
                            axis_w,
                            err,
                            dt,
                            f32::NEG_INFINITY,
                            0.0,
                        )
                        .abs();
                }
            }
        }

        if total_ang_impulse / dt > joint.break_torque {
            joint.is_broken = true;
            return;
        }

        // 5. Motor — velocity constraint along hinge axis
        if data.use_motor {
            let axis_w = transforms[idx_a].rotation * data.axis;
            let inv_i_a = rigid_bodies[idx_a].inv_world_inertia_tensor(transforms[idx_a].rotation);
            let inv_i_b = rigid_bodies[idx_b].inv_world_inertia_tensor(transforms[idx_b].rotation);
            let w_a = velocities[idx_a].angular;
            let w_b = velocities[idx_b].angular;
            let dyn_a = rigid_bodies[idx_a].is_dynamic();
            let dyn_b = rigid_bodies[idx_b].is_dynamic();

            let k = axis_w.dot(inv_i_a.mul_vec3(axis_w)) + axis_w.dot(inv_i_b.mul_vec3(axis_w));
            if k > 1e-10 {
                let rel_vel = (w_b - w_a).dot(axis_w);
                // Servo: turn the angle error into a target velocity (P-control via the
                // solver's position_bias); force is still capped by motor_max_force below.
                let target_vel = if data.motor_is_servo {
                    self.position_bias * (data.motor_target_position - data.current_angle) / dt
                } else {
                    data.motor_target_velocity
                };
                let vel_err = target_vel - rel_vel;
                // Step başına toplam motor impulse bütçesini iterasyonlara böl; aksi
                // halde her iterasyon ayrı sınırlandığından motor ~iterations kat fazla
                // kuvvet uygulardı.
                let max_impulse = data.motor_max_force * dt / self.iterations.max(1) as f32;
                let lambda = (vel_err / k).clamp(-max_impulse, max_impulse);

                let delta_a = inv_i_a.mul_vec3(axis_w) * lambda;
                let delta_b = inv_i_b.mul_vec3(axis_w) * lambda;

                if idx_a < idx_b {
                    let (l, r) = velocities.split_at_mut(idx_b);
                    if dyn_a {
                        l[idx_a].angular -= delta_a;
                    }
                    if dyn_b {
                        r[0].angular += delta_b;
                    }
                } else {
                    let (l, r) = velocities.split_at_mut(idx_a);
                    if dyn_b {
                        l[idx_b].angular += delta_b;
                    }
                    if dyn_a {
                        r[0].angular -= delta_a;
                    }
                }
            }
        }
    }

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
        if !data.use_cone_limit && !data.use_twist_limit {
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
                    .apply_angular_constraint(
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
                    .apply_angular_constraint(
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
                    )
                    .abs();
            } else if twist_angle < data.twist_lower {
                total_ang_impulse += self
                    .apply_angular_constraint(
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
                    )
                    .abs();
            }
        }

        if total_ang_impulse / dt > joint.break_torque {
            joint.is_broken = true;
        }
    }

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
            self.apply_linear_constraint(
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
            )
        } else if length < data.min_length {
            self.apply_linear_constraint(
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
            )
        } else {
            0.0 // within bounds → free
        };

        if lin_impulse.abs() / dt > joint.break_force {
            joint.is_broken = true;
        }
    }

    pub(crate) fn solve_slider_joint(
        &self,
        joint: &mut Joint,
        rigid_bodies: &[RigidBody],
        transforms: &[Transform],
        velocities: &mut [Velocity],
        idx_a: usize,
        idx_b: usize,
        dt: f32,
    ) {
        let JointData::Slider(ref mut data) = joint.data else {
            return;
        };

        let anchor_a =
            transforms[idx_a].position + transforms[idx_a].rotation * joint.local_anchor_a;
        let anchor_b =
            transforms[idx_b].position + transforms[idx_b].rotation * joint.local_anchor_b;
        let axis_w = (transforms[idx_a].rotation * data.axis).normalize();

        let delta = anchor_b - anchor_a;
        let along = delta.dot(axis_w);
        let off_axis = anchor_a - (anchor_b - axis_w * along); // error = target - current

        data.current_position = along;

        let r_a = anchor_a - transforms[idx_a].position;
        let r_b = anchor_b - transforms[idx_b].position;

        let mut total_lin_impulse = 0.0;
        let mut total_ang_impulse = 0.0;

        // 1. Off-axis constraint: project onto two perpendicular directions
        let (perp1, perp2) = Self::perpendiculars(axis_w);

        let err1 = off_axis.dot(perp1);
        if err1.abs() > 1e-4 {
            total_lin_impulse += self
                .apply_linear_constraint(
                    rigid_bodies,
                    transforms,
                    velocities,
                    idx_a,
                    idx_b,
                    perp1,
                    r_a,
                    r_b,
                    err1,
                    dt,
                    f32::NEG_INFINITY,
                    f32::INFINITY,
                )
                .abs();
        }

        let err2 = off_axis.dot(perp2);
        if err2.abs() > 1e-4 {
            total_lin_impulse += self
                .apply_linear_constraint(
                    rigid_bodies,
                    transforms,
                    velocities,
                    idx_a,
                    idx_b,
                    perp2,
                    r_a,
                    r_b,
                    err2,
                    dt,
                    f32::NEG_INFINITY,
                    f32::INFINITY,
                )
                .abs();
        }

        // 2. Angular lock — full 3-DOF rotation constraint using quaternion error
        let relative_rot = transforms[idx_a].rotation.inverse() * transforms[idx_b].rotation;
        if let Some(initial_rot) = data.initial_relative_rotation {
            let err_quat = initial_rot.inverse() * relative_rot;
            let ang_err_local = if err_quat.w >= 0.0 {
                Vec3::new(err_quat.x, err_quat.y, err_quat.z) * 2.0
            } else {
                -Vec3::new(err_quat.x, err_quat.y, err_quat.z) * 2.0
            };

            let err_world = transforms[idx_a].rotation * ang_err_local;
            let err_mag = err_world.length();
            if err_mag > 1e-6 {
                total_ang_impulse += self
                    .apply_angular_constraint(
                        rigid_bodies,
                        transforms,
                        velocities,
                        idx_a,
                        idx_b,
                        err_world / err_mag,
                        -err_mag,
                        dt,
                        f32::NEG_INFINITY,
                        f32::INFINITY,
                    )
                    .abs();
            }
        } else {
            data.initial_relative_rotation = Some(relative_rot);
        }

        // 3. Along-axis limits.
        // Impulse-clamp yönü `apply_linear_constraint` konvansiyonuyla uyumlu olmalı
        // (bkz. çalışan hinge limiti): alt-limit ihlali (err > 0) cismi +eksene İTER →
        // pozitif lambda → clamp (0, +∞); üst-limit ihlali (err < 0) −eksene iter →
        // negatif lambda → clamp (−∞, 0). (Eskiden ikisi de TERSTİ → limit hiç tutmuyordu;
        // 5 m/s'lik cisim 1 m'lik üst limiti delip 19 m'ye gidiyordu.)
        if data.use_limits {
            if along < data.lower_limit {
                let err = data.lower_limit - along; // positive
                total_lin_impulse += self
                    .apply_linear_constraint(
                        rigid_bodies,
                        transforms,
                        velocities,
                        idx_a,
                        idx_b,
                        axis_w,
                        r_a,
                        r_b,
                        err,
                        dt,
                        0.0,
                        f32::INFINITY,
                    )
                    .abs();
            } else if along > data.upper_limit {
                let err = data.upper_limit - along; // negative
                total_lin_impulse += self
                    .apply_linear_constraint(
                        rigid_bodies,
                        transforms,
                        velocities,
                        idx_a,
                        idx_b,
                        axis_w,
                        r_a,
                        r_b,
                        err,
                        dt,
                        f32::NEG_INFINITY,
                        0.0,
                    )
                    .abs();
            }
        }

        if total_lin_impulse / dt > joint.break_force || total_ang_impulse / dt > joint.break_torque
        {
            joint.is_broken = true;
            return;
        }

        // 4. Motor — velocity along axis
        if data.use_motor {
            // Step başına toplam motor impulse bütçesini iterasyonlara böl (bkz. hinge motor).
            let max_impulse = data.motor_max_force * dt / self.iterations.max(1) as f32;

            let v_a = velocities[idx_a].linear + velocities[idx_a].angular.cross(r_a);
            let v_b = velocities[idx_b].linear + velocities[idx_b].angular.cross(r_b);
            let rel_vel = (v_b - v_a).dot(axis_w);
            // Servo: drive toward the target position along the axis (P-control via
            // position_bias), force still capped by motor_max_force.
            let target_vel = if data.motor_is_servo {
                self.position_bias * (data.motor_target_position - data.current_position) / dt
            } else {
                data.motor_target_velocity
            };
            let vel_err = target_vel - rel_vel;

            let inv_m_a = rigid_bodies[idx_a].inv_mass();
            let inv_m_b = rigid_bodies[idx_b].inv_mass();
            let inv_i_a = rigid_bodies[idx_a].inv_world_inertia_tensor(transforms[idx_a].rotation);
            let inv_i_b = rigid_bodies[idx_b].inv_world_inertia_tensor(transforms[idx_b].rotation);
            let dyn_a = rigid_bodies[idx_a].is_dynamic();
            let dyn_b = rigid_bodies[idx_b].is_dynamic();

            // k_ang = (r×axis)·I⁻¹·(r×axis) — bkz. apply_linear_constraint düzeltmesi.
            let rxa_a = r_a.cross(axis_w);
            let rxa_b = r_b.cross(axis_w);
            let k = inv_m_a
                + inv_m_b
                + inv_i_a.mul_vec3(rxa_a).dot(rxa_a)
                + inv_i_b.mul_vec3(rxa_b).dot(rxa_b);
            if k > 1e-10 {
                let lambda = (vel_err / k).clamp(-max_impulse, max_impulse);
                let impulse = axis_w * lambda;

                if idx_a < idx_b {
                    let (l, r) = velocities.split_at_mut(idx_b);
                    if dyn_a {
                        l[idx_a].linear -= impulse * inv_m_a;
                        l[idx_a].angular -= inv_i_a.mul_vec3(r_a.cross(impulse));
                    }
                    if dyn_b {
                        r[0].linear += impulse * inv_m_b;
                        r[0].angular += inv_i_b.mul_vec3(r_b.cross(impulse));
                    }
                } else {
                    let (l, r) = velocities.split_at_mut(idx_a);
                    if dyn_b {
                        l[idx_b].linear += impulse * inv_m_b;
                        l[idx_b].angular += inv_i_b.mul_vec3(r_b.cross(impulse));
                    }
                    if dyn_a {
                        r[0].linear -= impulse * inv_m_a;
                        r[0].angular -= inv_i_a.mul_vec3(r_a.cross(impulse));
                    }
                }
            }
        }
    }

    pub(crate) fn solve_spring_joint(
        &self,
        joint: &mut Joint,
        rigid_bodies: &[RigidBody],
        transforms: &[Transform],
        velocities: &mut [Velocity],
        idx_a: usize,
        idx_b: usize,
        dt: f32,
    ) {
        let JointData::Spring(data) = joint.data else {
            return;
        };

        let anchor_a =
            transforms[idx_a].position + transforms[idx_a].rotation * joint.local_anchor_a;
        let anchor_b =
            transforms[idx_b].position + transforms[idx_b].rotation * joint.local_anchor_b;

        let diff = anchor_b - anchor_a;
        let length = diff.length();
        if length < 1e-6 {
            return;
        }

        let direction = diff / length;
        let r_a = anchor_a - transforms[idx_a].position;
        let r_b = anchor_b - transforms[idx_b].position;

        let v_a = velocities[idx_a].linear + velocities[idx_a].angular.cross(r_a);
        let v_b = velocities[idx_b].linear + velocities[idx_b].angular.cross(r_b);

        // Force calculation
        // direction points from A to B
        let spring_force = data.stiffness * (length - data.rest_length); // Positive if stretched (pulls together)
        let relative_vel = (v_b - v_a).dot(direction); // Positive if B is moving away from A
        let damping_force = data.damping * relative_vel;

        // Total force pulling them together
        let pull_force = spring_force + damping_force;
        let pull_impulse = pull_force * dt;

        // Hard limits (optional max_length)
        let clamped_impulse = if length <= data.min_length && pull_impulse > 0.0 {
            0.0 // already at min length, stop pulling
        } else if let Some(max_len) = data.max_length {
            if length >= max_len && pull_impulse < 0.0 {
                0.0 // already at max length, stop pushing apart
            } else {
                pull_impulse
            }
        } else {
            pull_impulse
        };

        // Breakable: the spring's linear force is |impulse|/dt. Previously missing — a
        // Spring could never break despite the advertised break_force (an API footgun,
        // like the old Fixed-torque no-op). Now matches Distance's break handling.
        if clamped_impulse.abs() / dt > joint.break_force {
            joint.is_broken = true;
            return;
        }

        if clamped_impulse.abs() < 1e-10 {
            return;
        }

        // Apply impulse along direction (A to B)
        // If clamped_impulse > 0, they are pulled together: A moves to B (+), B moves to A (-)
        let impulse = direction * clamped_impulse;
        let inv_m_a = rigid_bodies[idx_a].inv_mass();
        let inv_m_b = rigid_bodies[idx_b].inv_mass();
        let inv_i_a = rigid_bodies[idx_a].inv_world_inertia_tensor(transforms[idx_a].rotation);
        let inv_i_b = rigid_bodies[idx_b].inv_world_inertia_tensor(transforms[idx_b].rotation);
        let dyn_a = rigid_bodies[idx_a].is_dynamic();
        let dyn_b = rigid_bodies[idx_b].is_dynamic();

        if idx_a < idx_b {
            let (l, r) = velocities.split_at_mut(idx_b);
            if dyn_a {
                l[idx_a].linear += impulse * inv_m_a;
                l[idx_a].angular += inv_i_a.mul_vec3(r_a.cross(impulse));
            }
            if dyn_b {
                r[0].linear -= impulse * inv_m_b;
                r[0].angular -= inv_i_b.mul_vec3(r_b.cross(impulse));
            }
        } else {
            let (l, r) = velocities.split_at_mut(idx_a);
            if dyn_b {
                l[idx_b].linear -= impulse * inv_m_b;
                l[idx_b].angular -= inv_i_b.mul_vec3(r_b.cross(impulse));
            }
            if dyn_a {
                r[0].linear += impulse * inv_m_a;
                r[0].angular += inv_i_a.mul_vec3(r_a.cross(impulse));
            }
        }
    }

    /// Torsional spring on a Hinge: a soft restoring torque about the hinge axis toward
    /// `rest_angle` (stiffness + damping) — self-closing doors, spring flaps, soft ragdoll
    /// stiffness. Force-based (once per step); reads `current_angle` (updated this step by
    /// solve_hinge_joint). No-op unless `use_torsional_spring`.
    pub(crate) fn solve_hinge_spring(
        &self,
        joint: &Joint,
        rigid_bodies: &[RigidBody],
        transforms: &[Transform],
        velocities: &mut [Velocity],
        idx_a: usize,
        idx_b: usize,
        dt: f32,
    ) {
        let JointData::Hinge(data) = joint.data else {
            return;
        };
        if !data.use_torsional_spring {
            return;
        }
        let axis_w = (transforms[idx_a].rotation * data.axis).normalize_or_zero();
        if axis_w.length_squared() < 1e-6 {
            return;
        }
        let inv_i_a = rigid_bodies[idx_a].inv_world_inertia_tensor(transforms[idx_a].rotation);
        let inv_i_b = rigid_bodies[idx_b].inv_world_inertia_tensor(transforms[idx_b].rotation);
        let rel_ang_vel = (velocities[idx_b].angular - velocities[idx_a].angular).dot(axis_w);

        // Restoring torque toward rest_angle (+ damping). Negative sign so that angle > rest
        // yields a torque that DECREASES the angle. Sign convention matches the hinge motor:
        // a positive impulse about axis_w increases the relative angle.
        let torque_impulse = -(data.torsional_stiffness * (data.current_angle - data.rest_angle)
            + data.torsional_damping * rel_ang_vel)
            * dt;
        if torque_impulse.abs() < 1e-12 {
            return;
        }
        let delta_a = inv_i_a.mul_vec3(axis_w) * torque_impulse;
        let delta_b = inv_i_b.mul_vec3(axis_w) * torque_impulse;
        let dyn_a = rigid_bodies[idx_a].is_dynamic();
        let dyn_b = rigid_bodies[idx_b].is_dynamic();
        if idx_a < idx_b {
            let (l, r) = velocities.split_at_mut(idx_b);
            if dyn_a {
                l[idx_a].angular -= delta_a;
            }
            if dyn_b {
                r[0].angular += delta_b;
            }
        } else {
            let (l, r) = velocities.split_at_mut(idx_a);
            if dyn_b {
                l[idx_b].angular += delta_b;
            }
            if dyn_a {
                r[0].angular -= delta_a;
            }
        }
    }

    /// Suspension spring on a Slider: a soft PD force along the free axis pulling the
    /// along-axis offset toward `spring_rest_position`. Force-based (runs once per step,
    /// like Spring); same sign convention — positive impulse reduces the along-axis offset.
    /// No-op unless `use_spring`. This is the springy-prismatic a shock/suspension needs,
    /// which the hard-limit + velocity-motor Slider could not express.
    pub(crate) fn solve_slider_spring(
        &self,
        joint: &Joint,
        rigid_bodies: &[RigidBody],
        transforms: &[Transform],
        velocities: &mut [Velocity],
        idx_a: usize,
        idx_b: usize,
        dt: f32,
    ) {
        let JointData::Slider(data) = joint.data else {
            return;
        };
        if !data.use_spring {
            return;
        }

        let anchor_a =
            transforms[idx_a].position + transforms[idx_a].rotation * joint.local_anchor_a;
        let anchor_b =
            transforms[idx_b].position + transforms[idx_b].rotation * joint.local_anchor_b;
        let axis_w = (transforms[idx_a].rotation * data.axis).normalize_or_zero();
        if axis_w.length_squared() < 1e-6 {
            return;
        }
        let along = (anchor_b - anchor_a).dot(axis_w);
        let r_a = anchor_a - transforms[idx_a].position;
        let r_b = anchor_b - transforms[idx_b].position;
        let v_a = velocities[idx_a].linear + velocities[idx_a].angular.cross(r_a);
        let v_b = velocities[idx_b].linear + velocities[idx_b].angular.cross(r_b);
        let rel_vel = (v_b - v_a).dot(axis_w);

        let impulse_mag = (data.spring_stiffness * (along - data.spring_rest_position)
            + data.spring_damping * rel_vel)
            * dt;
        if impulse_mag.abs() < 1e-10 {
            return;
        }
        let impulse = axis_w * impulse_mag;
        let inv_m_a = rigid_bodies[idx_a].inv_mass();
        let inv_m_b = rigid_bodies[idx_b].inv_mass();
        let inv_i_a = rigid_bodies[idx_a].inv_world_inertia_tensor(transforms[idx_a].rotation);
        let inv_i_b = rigid_bodies[idx_b].inv_world_inertia_tensor(transforms[idx_b].rotation);
        let dyn_a = rigid_bodies[idx_a].is_dynamic();
        let dyn_b = rigid_bodies[idx_b].is_dynamic();
        if idx_a < idx_b {
            let (l, r) = velocities.split_at_mut(idx_b);
            if dyn_a {
                l[idx_a].linear += impulse * inv_m_a;
                l[idx_a].angular += inv_i_a.mul_vec3(r_a.cross(impulse));
            }
            if dyn_b {
                r[0].linear -= impulse * inv_m_b;
                r[0].angular -= inv_i_b.mul_vec3(r_b.cross(impulse));
            }
        } else {
            let (l, r) = velocities.split_at_mut(idx_a);
            if dyn_b {
                l[idx_b].linear -= impulse * inv_m_b;
                l[idx_b].angular -= inv_i_b.mul_vec3(r_b.cross(impulse));
            }
            if dyn_a {
                r[0].linear += impulse * inv_m_a;
                r[0].angular += inv_i_a.mul_vec3(r_a.cross(impulse));
            }
        }
    }
}
