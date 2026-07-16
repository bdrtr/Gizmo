//! D6 (6-DOF) joint + per-axis drives solver — extracted verbatim from the former 1236-line joint_types.rs.
//! One `impl JointSolver` block per joint kind; `pub(crate)` methods stay callable
//! from `solve_joints` regardless of file (inherent impls compose across modules).

use super::super::*;

impl JointSolver {

    /// Generic 6-DOF joint: for each of 3 linear + 3 angular DOFs (in the joint `frame`),
    /// Locked drives the relative motion to zero, Limited clamps it to `[lower, upper]`,
    /// Free leaves it. Composed entirely from the 1-DOF constraint primitives, so Fixed
    /// (all locked), Slider (one linear Free) and Hinge (one angular Free) are special cases.
    /// The angular part uses the small-angle rotation vector (accurate where axes are locked
    /// near zero; a limited axis is approximate for large ranges).
    pub(crate) fn solve_d6_joint(
        &self,
        joint: &mut Joint,
        rigid_bodies: &[RigidBody],
        transforms: &[Transform],
        velocities: &mut [Velocity],
        idx_a: usize,
        idx_b: usize,
        dt: f32,
    ) {
        let (la, lb, break_force, break_torque) = (
            joint.local_anchor_a,
            joint.local_anchor_b,
            joint.break_force,
            joint.break_torque,
        );
        let JointData::D6(ref mut data) = joint.data else {
            return;
        };
        let rot_a = transforms[idx_a].rotation;
        let anchor_a = transforms[idx_a].position + rot_a * la;
        let anchor_b = transforms[idx_b].position + transforms[idx_b].rotation * lb;
        let r_a = anchor_a - transforms[idx_a].position;
        let r_b = anchor_b - transforms[idx_b].position;
        let delta = anchor_b - anchor_a; // position error (world)
        let unit = [Vec3::X, Vec3::Y, Vec3::Z];
        let compliance = data.compliance;

        // ── Linear DOFs (error convention: target - current = -offset for a lock) ──
        let mut lin_impulse = 0.0;
        for (i, mode) in data.linear.iter().enumerate() {
            let axis_w = rot_a * (data.frame * unit[i]);
            let offset = delta.dot(axis_w);
            let (error, lo_clamp, hi_clamp) = match *mode {
                D6Motion::Free => continue,
                D6Motion::Locked => (-offset, f32::NEG_INFINITY, f32::INFINITY),
                D6Motion::Limited { lower, upper } => {
                    if offset > upper {
                        (upper - offset, f32::NEG_INFINITY, 0.0)
                    } else if offset < lower {
                        (lower - offset, 0.0, f32::INFINITY)
                    } else {
                        continue;
                    }
                }
            };
            lin_impulse += self
                .apply_linear_constraint_soft(
                    rigid_bodies, transforms, velocities, idx_a, idx_b, axis_w, r_a, r_b, error,
                    dt, lo_clamp, hi_clamp, compliance,
                )
                .abs();
        }

        // ── Angular DOFs (small-angle rotation vector projected onto the frame axes) ──
        let relative_rot = rot_a.inverse() * transforms[idx_b].rotation;
        let initial = match data.initial_relative_rotation {
            None => {
                data.initial_relative_rotation = Some(relative_rot);
                if lin_impulse / dt > break_force {
                    joint.is_broken = true;
                    tracing::debug!(
                        entity_a = ?joint.entity_a,
                        entity_b = ?joint.entity_b,
                        applied_force = lin_impulse / dt,
                        break_force,
                        "D6 joint broke (linear force exceeded break threshold)"
                    );
                }
                return;
            }
            Some(rot) => rot,
        };
        let swing = initial.inverse() * relative_rot;
        let q = if swing.w < 0.0 { -swing } else { swing };
        let rvec = 2.0 * Vec3::new(q.x, q.y, q.z);
        let mut ang_impulse = 0.0;
        for (i, mode) in data.angular.iter().enumerate() {
            let axis_local = data.frame * unit[i];
            let angle = rvec.dot(axis_local);
            let axis_w = rot_a * axis_local;
            let (error, lo_clamp, hi_clamp) = match *mode {
                D6Motion::Free => continue,
                D6Motion::Locked => (-angle, f32::NEG_INFINITY, f32::INFINITY),
                D6Motion::Limited { lower, upper } => {
                    if angle > upper {
                        (upper - angle, f32::NEG_INFINITY, 0.0)
                    } else if angle < lower {
                        (lower - angle, 0.0, f32::INFINITY)
                    } else {
                        continue;
                    }
                }
            };
            ang_impulse += self
                .apply_angular_constraint_soft(
                    rigid_bodies, transforms, velocities, idx_a, idx_b, axis_w, error, dt,
                    lo_clamp, hi_clamp, compliance,
                )
                .abs();
        }

        if lin_impulse / dt > break_force || ang_impulse / dt > break_torque {
            joint.is_broken = true;
            tracing::debug!(
                entity_a = ?joint.entity_a,
                entity_b = ?joint.entity_b,
                applied_force = lin_impulse / dt,
                break_force,
                applied_torque = ang_impulse / dt,
                break_torque,
                "D6 joint broke (force/torque exceeded break threshold)"
            );
        }
    }

    /// Force-based per-axis DRIVES for a D6 joint (motor + spring), applied once per step
    /// next to the other force-based joints. Each enabled drive is a spring-damper pulling
    /// its DOF toward `target_position`/`target_velocity`, force-limited by `max_force`.
    /// Reads `initial_relative_rotation` (set by `solve_d6_joint` in the velocity loop).
    pub(crate) fn solve_d6_drives(
        &self,
        joint: &Joint,
        rigid_bodies: &[RigidBody],
        transforms: &[Transform],
        velocities: &mut [Velocity],
        idx_a: usize,
        idx_b: usize,
        dt: f32,
    ) {
        let JointData::D6(data) = joint.data else {
            return;
        };
        if !data
            .linear_drives
            .iter()
            .chain(data.angular_drives.iter())
            .any(|d| d.enabled)
        {
            return;
        }

        let rot_a = transforms[idx_a].rotation;
        let anchor_a = transforms[idx_a].position + rot_a * joint.local_anchor_a;
        let anchor_b =
            transforms[idx_b].position + transforms[idx_b].rotation * joint.local_anchor_b;
        let r_a = anchor_a - transforms[idx_a].position;
        let r_b = anchor_b - transforms[idx_b].position;
        let delta = anchor_b - anchor_a;
        let unit = [Vec3::X, Vec3::Y, Vec3::Z];

        // ── Linear drives (slider-spring sign: positive impulse reduces the offset) ──
        for (i, dr) in data.linear_drives.iter().enumerate() {
            if !dr.enabled {
                continue;
            }
            let axis_w = rot_a * (data.frame * unit[i]);
            let offset = delta.dot(axis_w);
            let v_a = velocities[idx_a].linear + velocities[idx_a].angular.cross(r_a);
            let v_b = velocities[idx_b].linear + velocities[idx_b].angular.cross(r_b);
            let rel_vel = (v_b - v_a).dot(axis_w);
            let limit = if dr.max_force > 0.0 { dr.max_force } else { f32::INFINITY };
            let force = (dr.stiffness * (offset - dr.target_position)
                + dr.damping * (rel_vel - dr.target_velocity))
                .clamp(-limit, limit);
            let impulse_mag = force * dt;
            if impulse_mag.abs() < 1e-10 {
                continue;
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

        // ── Angular drives (hinge-spring sign: torque restores toward the target angle) ──
        let Some(initial) = data.initial_relative_rotation else {
            return;
        };
        let relative_rot = rot_a.inverse() * transforms[idx_b].rotation;
        let swing = initial.inverse() * relative_rot;
        let q = if swing.w < 0.0 { -swing } else { swing };
        let rvec = 2.0 * Vec3::new(q.x, q.y, q.z);
        for (i, dr) in data.angular_drives.iter().enumerate() {
            if !dr.enabled {
                continue;
            }
            let axis_local = data.frame * unit[i];
            let angle = rvec.dot(axis_local);
            let axis_w = rot_a * axis_local;
            let ang_rel = (velocities[idx_b].angular - velocities[idx_a].angular).dot(axis_w);
            let limit = if dr.max_force > 0.0 { dr.max_force } else { f32::INFINITY };
            let torque = (dr.stiffness * (angle - dr.target_position)
                + dr.damping * (ang_rel - dr.target_velocity))
                .clamp(-limit, limit);
            let torque_impulse = -torque * dt;
            if torque_impulse.abs() < 1e-12 {
                continue;
            }
            let inv_i_a = rigid_bodies[idx_a].inv_world_inertia_tensor(transforms[idx_a].rotation);
            let inv_i_b = rigid_bodies[idx_b].inv_world_inertia_tensor(transforms[idx_b].rotation);
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
}
