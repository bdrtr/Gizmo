use super::data::*;
use crate::components::{RigidBody, Transform, Velocity};
use gizmo_math::Vec3;

pub struct JointSolver {
    pub iterations: usize,
    pub max_correction_speed: f32,
    pub max_angular_speed: f32,
    pub position_bias: f32,
}

impl Default for JointSolver {
    fn default() -> Self {
        Self {
            iterations: 10,
            max_correction_speed: 5.0,
            max_angular_speed: 5.0,
            position_bias: 0.3,
        }
    }
}

impl JointSolver {
    pub fn new(iterations: usize) -> Self {
        Self {
            iterations,
            ..Default::default()
        }
    }

    pub fn solve_joints(
        &self,
        joints: &mut [Joint],
        entity_index_map: &std::collections::HashMap<u32, usize>,
        rigid_bodies: &[RigidBody],
        transforms: &[Transform],
        velocities: &mut [Velocity],
        dt: f32,
    ) {
        for _ in 0..self.iterations {
            for joint in joints.iter_mut() {
                if joint.is_broken {
                    continue;
                }

                let idx_a = entity_index_map.get(&joint.entity_a.id()).copied();
                let idx_b = entity_index_map.get(&joint.entity_b.id()).copied();
                let (Some(idx_a), Some(idx_b)) = (idx_a, idx_b) else {
                    continue;
                };
                if idx_a == idx_b {
                    continue;
                }

                match joint.joint_type() {
                    "Fixed" => self.solve_fixed_joint(
                        joint,
                        rigid_bodies,
                        transforms,
                        velocities,
                        idx_a,
                        idx_b,
                        dt,
                    ),
                    "Hinge" => self.solve_hinge_joint(
                        joint,
                        rigid_bodies,
                        transforms,
                        velocities,
                        idx_a,
                        idx_b,
                        dt,
                    ),
                    "BallSocket" => self.solve_ball_socket_joint(
                        joint,
                        rigid_bodies,
                        transforms,
                        velocities,
                        idx_a,
                        idx_b,
                        dt,
                    ),
                    "Slider" => self.solve_slider_joint(
                        joint,
                        rigid_bodies,
                        transforms,
                        velocities,
                        idx_a,
                        idx_b,
                        dt,
                    ),
                    "Spring" => self.solve_spring_joint(
                        joint,
                        rigid_bodies,
                        transforms,
                        velocities,
                        idx_a,
                        idx_b,
                        dt,
                    ),
                    _ => {}
                }
            }
        }
    }

    // ── helpers ──────────────────────────────────────────────────────────────

    /// Two unit vectors perpendicular to `v`.
    fn perpendiculars(v: Vec3) -> (Vec3, Vec3) {
        let p1 = if v.x.abs() < 0.9 {
            v.cross(Vec3::X).normalize()
        } else {
            v.cross(Vec3::Y).normalize()
        };
        (p1, v.cross(p1))
    }

    /// Apply a 1-DOF angular velocity constraint along `direction`.
    /// `error` is the positional error in radians (positive = bodies need to rotate apart).
    fn apply_angular_constraint(
        &self,
        rigid_bodies: &[RigidBody],
        transforms: &[Transform],
        velocities: &mut [Velocity],
        idx_a: usize,
        idx_b: usize,
        direction: Vec3,
        error: f32,
        dt: f32,
        lambda_min: f32,
        lambda_max: f32,
    ) -> f32 {
        if direction.length_squared() < 1e-10 {
            return 0.0;
        }

        let inv_i_a = rigid_bodies[idx_a].inv_world_inertia_tensor(transforms[idx_a].rotation);
        let inv_i_b = rigid_bodies[idx_b].inv_world_inertia_tensor(transforms[idx_b].rotation);
        let w_a = velocities[idx_a].angular;
        let w_b = velocities[idx_b].angular;
        let dyn_a = rigid_bodies[idx_a].is_dynamic();
        let dyn_b = rigid_bodies[idx_b].is_dynamic();

        let k = direction.dot(inv_i_a * direction) + direction.dot(inv_i_b * direction);
        if k < 1e-10 {
            return 0.0;
        }

        let vel_err = (w_b - w_a).dot(direction);
        let position_bias = (self.position_bias * error / dt)
            .clamp(-self.max_angular_speed, self.max_angular_speed);
        let lambda = ((-vel_err + position_bias) / k).clamp(lambda_min, lambda_max);

        let delta_a = inv_i_a * direction * lambda;
        let delta_b = inv_i_b * direction * lambda;

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
        lambda
    }

    /// Apply a 1-DOF linear velocity constraint along `direction` at the anchor points.
    fn apply_linear_constraint(
        &self,
        rigid_bodies: &[RigidBody],
        transforms: &[Transform],
        velocities: &mut [Velocity],
        idx_a: usize,
        idx_b: usize,
        direction: Vec3,
        r_a: Vec3,
        r_b: Vec3,
        error: f32,
        dt: f32,
        lambda_min: f32,
        lambda_max: f32,
    ) -> f32 {
        let inv_m_a = rigid_bodies[idx_a].inv_mass();
        let inv_m_b = rigid_bodies[idx_b].inv_mass();
        let inv_i_a = rigid_bodies[idx_a].inv_world_inertia_tensor(transforms[idx_a].rotation);
        let inv_i_b = rigid_bodies[idx_b].inv_world_inertia_tensor(transforms[idx_b].rotation);
        let v_a = velocities[idx_a].linear + velocities[idx_a].angular.cross(r_a);
        let v_b = velocities[idx_b].linear + velocities[idx_b].angular.cross(r_b);
        let dyn_a = rigid_bodies[idx_a].is_dynamic();
        let dyn_b = rigid_bodies[idx_b].is_dynamic();

        let ang_a = (inv_i_a * r_a.cross(direction)).cross(r_a);
        let ang_b = (inv_i_b * r_b.cross(direction)).cross(r_b);
        let k = inv_m_a + inv_m_b + ang_a.dot(direction) + ang_b.dot(direction);
        if k < 1e-10 {
            return 0.0;
        }

        let rel_vel = (v_b - v_a).dot(direction);
        let position_bias = (self.position_bias * error / dt)
            .clamp(-self.max_correction_speed, self.max_correction_speed);
        let lambda = ((-rel_vel + position_bias) / k).clamp(lambda_min, lambda_max);

        let impulse = direction * lambda;

        if idx_a < idx_b {
            let (l, r) = velocities.split_at_mut(idx_b);
            if dyn_a {
                l[idx_a].linear -= impulse * inv_m_a;
                l[idx_a].angular -= inv_i_a * r_a.cross(impulse);
            }
            if dyn_b {
                r[0].linear += impulse * inv_m_b;
                r[0].angular += inv_i_b * r_b.cross(impulse);
            }
        } else {
            let (l, r) = velocities.split_at_mut(idx_a);
            if dyn_b {
                l[idx_b].linear += impulse * inv_m_b;
                l[idx_b].angular += inv_i_b * r_b.cross(impulse);
            }
            if dyn_a {
                r[0].linear -= impulse * inv_m_a;
                r[0].angular -= inv_i_a * r_a.cross(impulse);
            }
        }
        lambda
    }

    // ── joint solvers ─────────────────────────────────────────────────────────

    fn solve_fixed_joint(
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

        if err_len < 0.0001 {
            return;
        }

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

    fn solve_hinge_joint(
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

            let k = axis_w.dot(inv_i_a * axis_w) + axis_w.dot(inv_i_b * axis_w);
            if k > 1e-10 {
                let rel_vel = (w_b - w_a).dot(axis_w);
                let vel_err = data.motor_target_velocity - rel_vel;
                let max_impulse = data.motor_max_force * dt;
                let lambda = (vel_err / k).clamp(-max_impulse, max_impulse);

                let delta_a = inv_i_a * axis_w * lambda;
                let delta_b = inv_i_b * axis_w * lambda;

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

    fn solve_ball_socket_joint(
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
        if !data.use_cone_limit {
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

        // Compute the "swing" rotation of B away from its initial orientation (in A's frame)
        let swing_quat = initial_rot.inverse() * relative_rot;

        // Small-angle: angular error ≈ 2 * quat.xyz (when w ≥ 0)
        let swing_err_local = if swing_quat.w >= 0.0 {
            Vec3::new(swing_quat.x, swing_quat.y, swing_quat.z) * 2.0
        } else {
            -Vec3::new(swing_quat.x, swing_quat.y, swing_quat.z) * 2.0
        };

        let swing_angle = swing_err_local.length();
        if swing_angle <= data.cone_limit_angle || swing_angle < 1e-6 {
            return;
        }

        let excess = swing_angle - data.cone_limit_angle;
        let swing_dir_local = swing_err_local / swing_angle;

        // Convert error direction to world space
        let swing_dir_world = transforms[idx_a].rotation * swing_dir_local;

        let mut total_ang_impulse = 0.0;
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
        if total_ang_impulse / dt > joint.break_torque {
            joint.is_broken = true;
        }
    }

    fn solve_slider_joint(
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

        // 3. Along-axis limits
        if data.use_limits {
            if along < data.lower_limit {
                let err = data.lower_limit - along;
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
                        0.0,
                        f32::INFINITY,
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
            let max_impulse = data.motor_max_force * dt;

            let v_a = velocities[idx_a].linear + velocities[idx_a].angular.cross(r_a);
            let v_b = velocities[idx_b].linear + velocities[idx_b].angular.cross(r_b);
            let rel_vel = (v_b - v_a).dot(axis_w);
            let vel_err = data.motor_target_velocity - rel_vel;

            let inv_m_a = rigid_bodies[idx_a].inv_mass();
            let inv_m_b = rigid_bodies[idx_b].inv_mass();
            let inv_i_a = rigid_bodies[idx_a].inv_world_inertia_tensor(transforms[idx_a].rotation);
            let inv_i_b = rigid_bodies[idx_b].inv_world_inertia_tensor(transforms[idx_b].rotation);
            let dyn_a = rigid_bodies[idx_a].is_dynamic();
            let dyn_b = rigid_bodies[idx_b].is_dynamic();

            let ang_a = (inv_i_a * r_a.cross(axis_w)).cross(r_a);
            let ang_b = (inv_i_b * r_b.cross(axis_w)).cross(r_b);
            let k = inv_m_a + inv_m_b + ang_a.dot(axis_w) + ang_b.dot(axis_w);
            if k > 1e-10 {
                let lambda = (vel_err / k).clamp(-max_impulse, max_impulse);
                let impulse = axis_w * lambda;

                if idx_a < idx_b {
                    let (l, r) = velocities.split_at_mut(idx_b);
                    if dyn_a {
                        l[idx_a].linear -= impulse * inv_m_a;
                        l[idx_a].angular -= inv_i_a * r_a.cross(impulse);
                    }
                    if dyn_b {
                        r[0].linear += impulse * inv_m_b;
                        r[0].angular += inv_i_b * r_b.cross(impulse);
                    }
                } else {
                    let (l, r) = velocities.split_at_mut(idx_a);
                    if dyn_b {
                        l[idx_b].linear += impulse * inv_m_b;
                        l[idx_b].angular += inv_i_b * r_b.cross(impulse);
                    }
                    if dyn_a {
                        r[0].linear -= impulse * inv_m_a;
                        r[0].angular -= inv_i_a * r_a.cross(impulse);
                    }
                }
            }
        }
    }

    fn solve_spring_joint(
        &self,
        joint: &Joint,
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
                l[idx_a].angular += inv_i_a * r_a.cross(impulse);
            }
            if dyn_b {
                r[0].linear -= impulse * inv_m_b;
                r[0].angular -= inv_i_b * r_b.cross(impulse);
            }
        } else {
            let (l, r) = velocities.split_at_mut(idx_a);
            if dyn_b {
                l[idx_b].linear -= impulse * inv_m_b;
                l[idx_b].angular -= inv_i_b * r_b.cross(impulse);
            }
            if dyn_a {
                r[0].linear += impulse * inv_m_a;
                r[0].angular += inv_i_a * r_a.cross(impulse);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_core::entity::Entity;

    #[test]
    fn test_joint_creation() {
        let e1 = Entity::new(1, 0);
        let e2 = Entity::new(2, 0);
        let joint = Joint::fixed(e1, e2, Vec3::ZERO, Vec3::ZERO);
        assert_eq!(joint.joint_type(), "Fixed");
        assert!(!joint.is_broken);
    }

    #[test]
    fn test_hinge_joint() {
        let e1 = Entity::new(1, 0);
        let e2 = Entity::new(2, 0);
        let joint = Joint::hinge(e1, e2, Vec3::ZERO, Vec3::ZERO, Vec3::Y);
        assert_eq!(joint.joint_type(), "Hinge");
        if let JointData::Hinge(data) = joint.data {
            assert_eq!(data.axis, Vec3::Y);
        } else {
            panic!("expected hinge data");
        }
    }

    #[test]
    fn test_spring_joint() {
        let e1 = Entity::new(1, 0);
        let e2 = Entity::new(2, 0);
        let joint = Joint::spring(e1, e2, Vec3::ZERO, Vec3::ZERO, 1.0, 100.0, 10.0);
        if let JointData::Spring(data) = joint.data {
            assert_eq!(data.stiffness, 100.0);
            assert_eq!(data.damping, 10.0);
        } else {
            panic!("expected spring data");
        }
    }

    #[test]
    fn test_perpendiculars_orthogonality() {
        let v = Vec3::new(1.0, 0.0, 0.0);
        let (p1, p2) = JointSolver::perpendiculars(v);
        assert!(p1.dot(v).abs() < 1e-5);
        assert!(p2.dot(v).abs() < 1e-5);
        assert!(p1.dot(p2).abs() < 1e-5);
    }
}
