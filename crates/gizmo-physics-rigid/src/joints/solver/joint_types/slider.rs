//! Slider/prismatic-joint (+ suspension spring) solver — extracted verbatim from the former 1236-line joint_types.rs.
//! One `impl JointSolver` block per joint kind; `pub(crate)` methods stay callable
//! from `solve_joints` regardless of file (inherent impls compose across modules).

use super::super::*;

impl JointSolver {

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
