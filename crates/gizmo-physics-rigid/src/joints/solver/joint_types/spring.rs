//! Generic spring-joint solver — extracted verbatim from the former 1236-line joint_types.rs.
//! One `impl JointSolver` block per joint kind; `pub(crate)` methods stay callable
//! from `solve_joints` regardless of file (inherent impls compose across modules).

use super::super::*;

impl JointSolver {

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
            tracing::debug!(
                entity_a = ?joint.entity_a,
                entity_b = ?joint.entity_b,
                applied_force = clamped_impulse.abs() / dt,
                break_force = joint.break_force,
                "Spring joint broke (force exceeded break threshold)"
            );
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
}
