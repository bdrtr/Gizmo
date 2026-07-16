//! Hinge-joint (+ torsional spring) solver — extracted verbatim from the former 1236-line joint_types.rs.
//! One `impl JointSolver` block per joint kind; `pub(crate)` methods stay callable
//! from `solve_joints` regardless of file (inherent impls compose across modules).

use super::super::*;

impl JointSolver {

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
            tracing::debug!(
                entity_a = ?joint.entity_a,
                entity_b = ?joint.entity_b,
                applied_torque = total_ang_impulse / dt,
                break_torque = joint.break_torque,
                "Hinge joint broke (torque exceeded break threshold)"
            );
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
}
