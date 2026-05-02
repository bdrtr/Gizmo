use crate::collision::ContactManifold;
use crate::components::{RigidBody, Transform, Velocity};
use gizmo_math::{Mat3, Vec3};

pub struct ConstraintSolver {
    pub iterations: usize,
    pub baumgarte: f32,
    pub slop: f32,
    pub warm_start_factor: f32,
}

impl Default for ConstraintSolver {
    fn default() -> Self {
        Self {
            iterations: 10,
            baumgarte: 0.2,
            slop: 0.01,
            warm_start_factor: 0.8,
        }
    }
}

impl ConstraintSolver {
    pub fn new(iterations: usize) -> Self {
        Self { iterations, ..Default::default() }
    }

    /// Solve all contact constraints.
    /// `bodies_a[i]` and `bodies_b[i]` must correspond to `manifolds[i].entity_a/b`.
    /// Manifolds are updated with accumulated impulses for warm starting next frame.
    pub fn solve_contacts(
        &self,
        manifolds: &mut [ContactManifold],
        bodies_a: &mut [(RigidBody, Transform, Velocity)],
        bodies_b: &mut [(RigidBody, Transform, Velocity)],
        dt: f32,
    ) {
        if manifolds.is_empty() { return; }

        // ── Warm starting ────────────────────────────────────────────────────
        // Apply a fraction of last frame's accumulated impulses to speed up convergence.
        for mid in 0..manifolds.len() {
            if mid >= bodies_a.len() || mid >= bodies_b.len() { continue; }

            let inv_m_a = bodies_a[mid].0.inv_mass();
            let inv_m_b = bodies_b[mid].0.inv_mass();
            let inv_i_a = bodies_a[mid].0.inv_world_inertia_tensor(bodies_a[mid].1.rotation);
            let inv_i_b = bodies_b[mid].0.inv_world_inertia_tensor(bodies_b[mid].1.rotation);
            let dyn_a   = bodies_a[mid].0.is_dynamic();
            let dyn_b   = bodies_b[mid].0.is_dynamic();

            for contact in &manifolds[mid].contacts {
                let global_com_a = bodies_a[mid].1.position + bodies_a[mid].1.rotation.mul_vec3(bodies_a[mid].0.center_of_mass);
                let global_com_b = bodies_b[mid].1.position + bodies_b[mid].1.rotation.mul_vec3(bodies_b[mid].0.center_of_mass);
                let r_a = contact.point - global_com_a;
                let r_b = contact.point - global_com_b;

                let wn = contact.normal * (contact.normal_impulse * self.warm_start_factor);
                let wt = contact.tangent_impulse * self.warm_start_factor;

                if dyn_a {
                    bodies_a[mid].2.linear  -= wn * inv_m_a;
                    bodies_a[mid].2.angular -= inv_i_a * r_a.cross(wn);
                    bodies_a[mid].2.linear  -= wt * inv_m_a;
                    bodies_a[mid].2.angular -= inv_i_a * r_a.cross(wt);
                }
                if dyn_b {
                    bodies_b[mid].2.linear  += wn * inv_m_b;
                    bodies_b[mid].2.angular += inv_i_b * r_b.cross(wn);
                    bodies_b[mid].2.linear  += wt * inv_m_b;
                    bodies_b[mid].2.angular += inv_i_b * r_b.cross(wt);
                }
            }
        }

        // ── Iterative solving with accumulated-impulse clamping ───────────────
        for _ in 0..self.iterations {
            for mid in 0..manifolds.len() {
                if mid >= bodies_a.len() || mid >= bodies_b.len() { continue; }

                let friction    = manifolds[mid].friction;
                let restitution = manifolds[mid].restitution;

                for cid in 0..manifolds[mid].contacts.len() {
                    // ── read phase ────────────────────────────────────────────
                    let contact_point = manifolds[mid].contacts[cid].point;
                    let normal        = manifolds[mid].contacts[cid].normal;
                    let penetration   = manifolds[mid].contacts[cid].penetration;
                    let acc_normal    = manifolds[mid].contacts[cid].normal_impulse;
                    let acc_tangent   = manifolds[mid].contacts[cid].tangent_impulse;

                    let global_com_a = bodies_a[mid].1.position + bodies_a[mid].1.rotation.mul_vec3(bodies_a[mid].0.center_of_mass);
                    let global_com_b = bodies_b[mid].1.position + bodies_b[mid].1.rotation.mul_vec3(bodies_b[mid].0.center_of_mass);
                    let r_a = contact_point - global_com_a;
                    let r_b = contact_point - global_com_b;

                    let inv_m_a = bodies_a[mid].0.inv_mass();
                    let inv_m_b = bodies_b[mid].0.inv_mass();
                    let inv_i_a = bodies_a[mid].0.inv_world_inertia_tensor(bodies_a[mid].1.rotation);
                    let inv_i_b = bodies_b[mid].0.inv_world_inertia_tensor(bodies_b[mid].1.rotation);
                    let dyn_a   = bodies_a[mid].0.is_dynamic();
                    let dyn_b   = bodies_b[mid].0.is_dynamic();

                    if !dyn_a && !dyn_b { continue; }

                    let va = bodies_a[mid].2.linear + bodies_a[mid].2.angular.cross(r_a);
                    let vb = bodies_b[mid].2.linear + bodies_b[mid].2.angular.cross(r_b);
                    let rel_vel  = vb - va;
                    let vel_norm = rel_vel.dot(normal);

                    // ── normal impulse ────────────────────────────────────────
                    let r_a_x_n = r_a.cross(normal);
                    let r_b_x_n = r_b.cross(normal);
                    let ang_a   = (inv_i_a * r_a_x_n).cross(r_a);
                    let ang_b   = (inv_i_b * r_b_x_n).cross(r_b);
                    let k_n = inv_m_a + inv_m_b + ang_a.dot(normal) + ang_b.dot(normal);

                    if k_n < 1e-6 { continue; }

                    let bias     = (self.baumgarte / dt) * (penetration - self.slop).max(0.0);
                    
                    // Resting contact threshold (prevent infinite micro-bounces)
                    let e = if vel_norm < -1.0 { restitution } else { 0.0 };
                    
                    let delta_n  = (-(1.0 + e) * vel_norm + bias) / k_n;

                    // Accumulated-impulse clamping (normal must be ≥ 0)
                    let new_acc_n  = (acc_normal + delta_n).max(0.0);
                    let actual_n   = new_acc_n - acc_normal;
                    manifolds[mid].contacts[cid].normal_impulse = new_acc_n;

                    let imp_n = normal * actual_n;

                    if dyn_a {
                        bodies_a[mid].2.linear  -= imp_n * inv_m_a;
                        bodies_a[mid].2.angular -= inv_i_a * r_a.cross(imp_n);
                    }
                    if dyn_b {
                        bodies_b[mid].2.linear  += imp_n * inv_m_b;
                        bodies_b[mid].2.angular += inv_i_b * r_b.cross(imp_n);
                    }

                    // ── friction impulse ──────────────────────────────────────
                    // Recompute velocities after normal impulse
                    let va2 = bodies_a[mid].2.linear + bodies_a[mid].2.angular.cross(r_a);
                    let vb2 = bodies_b[mid].2.linear + bodies_b[mid].2.angular.cross(r_b);
                    let rel2 = vb2 - va2;

                    let tang_vel = rel2 - normal * rel2.dot(normal);
                    let tang_mag = tang_vel.length();
                    if tang_mag < 1e-6 { continue; }

                    let tangent = tang_vel / tang_mag;

                    let r_a_x_t = r_a.cross(tangent);
                    let r_b_x_t = r_b.cross(tangent);
                    let ang_at  = (inv_i_a * r_a_x_t).cross(r_a);
                    let ang_bt  = (inv_i_b * r_b_x_t).cross(r_b);
                    let k_t = inv_m_a + inv_m_b + ang_at.dot(tangent) + ang_bt.dot(tangent);

                    if k_t < 1e-6 { continue; }

                    let delta_t = -tang_mag / k_t;

                    let max_static_tang = manifolds[mid].static_friction * new_acc_n.abs();
                    let max_dynamic_tang = friction * new_acc_n.abs();
                    
                    let mut new_acc_t_along = acc_tangent.dot(tangent) + delta_t;
                    
                    // If it exceeds the static friction cone, it slips (dynamic friction)
                    if new_acc_t_along.abs() > max_static_tang {
                        new_acc_t_along = new_acc_t_along.clamp(-max_dynamic_tang, max_dynamic_tang);
                    }
                    
                    let actual_t_along  = new_acc_t_along - acc_tangent.dot(tangent);
                    manifolds[mid].contacts[cid].tangent_impulse =
                        tangent * new_acc_t_along;

                    let imp_t = tangent * actual_t_along;

                    if dyn_a {
                        bodies_a[mid].2.linear  -= imp_t * inv_m_a;
                        bodies_a[mid].2.angular -= inv_i_a * r_a.cross(imp_t);
                    }
                    if dyn_b {
                        bodies_b[mid].2.linear  += imp_t * inv_m_b;
                        bodies_b[mid].2.angular += inv_i_b * r_b.cross(imp_t);
                    }
                }
            }
        }
    }

    /// Kept for backwards compatibility / standalone use.
    #[allow(clippy::too_many_arguments)]
    pub fn solve_contact_constraint(
        &self,
        rb_a: &mut RigidBody,
        transform_a: &Transform,
        vel_a: &mut Velocity,
        rb_b: &mut RigidBody,
        transform_b: &Transform,
        vel_b: &mut Velocity,
        contact_point: Vec3,
        normal: Vec3,
        penetration: f32,
        friction: f32,
        restitution: f32,
        dt: f32,
    ) {
        if !rb_a.is_dynamic() && !rb_b.is_dynamic() { return; }

        let r_a = contact_point - transform_a.position;
        let r_b = contact_point - transform_b.position;

        let va = vel_a.linear + vel_a.angular.cross(r_a);
        let vb = vel_b.linear + vel_b.angular.cross(r_b);
        let rel_vel  = vb - va;
        let vel_norm = rel_vel.dot(normal);

        if vel_norm > 0.0 { return; }

        let inv_m_a = rb_a.inv_mass();
        let inv_m_b = rb_b.inv_mass();
        let inv_i_a = rb_a.inv_world_inertia_tensor(transform_a.rotation);
        let inv_i_b = rb_b.inv_world_inertia_tensor(transform_b.rotation);

        let ang_a = (inv_i_a * r_a.cross(normal)).cross(r_a);
        let ang_b = (inv_i_b * r_b.cross(normal)).cross(r_b);
        let k = inv_m_a + inv_m_b + ang_a.dot(normal) + ang_b.dot(normal);
        if k < 1e-6 { return; }

        let bias = (self.baumgarte / dt) * (penetration - self.slop).max(0.0);
        let j    = (-(1.0 + restitution) * vel_norm + bias) / k;
        let j    = j.max(0.0);

        let impulse = normal * j;

        if rb_a.is_dynamic() {
            vel_a.linear  -= impulse * inv_m_a;
            vel_a.angular -= inv_i_a * r_a.cross(impulse);
        }
        if rb_b.is_dynamic() {
            vel_b.linear  += impulse * inv_m_b;
            vel_b.angular += inv_i_b * r_b.cross(impulse);
        }

        self.apply_friction(rb_a, vel_a, rb_b, vel_b, r_a, r_b, normal, friction, j, &inv_i_a, &inv_i_b);
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_friction(
        &self,
        rb_a: &RigidBody, vel_a: &mut Velocity,
        rb_b: &RigidBody, vel_b: &mut Velocity,
        r_a: Vec3, r_b: Vec3,
        normal: Vec3, friction: f32, normal_impulse: f32,
        inv_i_a: &Mat3, inv_i_b: &Mat3,
    ) {
        let va  = vel_a.linear + vel_a.angular.cross(r_a);
        let vb  = vel_b.linear + vel_b.angular.cross(r_b);
        let rel = vb - va;

        let tang_vel = rel - normal * rel.dot(normal);
        let tang_mag = tang_vel.length();
        if tang_mag < 1e-6 { return; }

        let tangent = tang_vel / tang_mag;

        let inv_m_a = rb_a.inv_mass();
        let inv_m_b = rb_b.inv_mass();

        let ang_a = (*inv_i_a * r_a.cross(tangent)).cross(r_a);
        let ang_b = (*inv_i_b * r_b.cross(tangent)).cross(r_b);
        let k = inv_m_a + inv_m_b + ang_a.dot(tangent) + ang_b.dot(tangent);
        if k < 1e-6 { return; }

        let jt = -tang_mag / k;

        let friction_impulse = if jt.abs() < friction * normal_impulse.abs() {
            tangent * jt
        } else {
            tangent * (-friction * normal_impulse.abs())
        };

        if rb_a.is_dynamic() {
            vel_a.linear  -= friction_impulse * inv_m_a;
            vel_a.angular -= *inv_i_a * r_a.cross(friction_impulse);
        }
        if rb_b.is_dynamic() {
            vel_b.linear  += friction_impulse * inv_m_b;
            vel_b.angular += *inv_i_b * r_b.cross(friction_impulse);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_solver_creation() {
        let solver = ConstraintSolver::new(10);
        assert_eq!(solver.iterations, 10);
    }

    #[test]
    fn test_collision_response() {
        let mut rb_a = RigidBody::default();
        let mut rb_b = RigidBody::default();
        let transform_a = Transform::new(Vec3::new(0.0, 0.0, 0.0));
        let transform_b = Transform::new(Vec3::new(0.0, 2.0, 0.0));
        let mut vel_a = Velocity::new(Vec3::new(0.0, 1.0, 0.0));
        let mut vel_b = Velocity::new(Vec3::new(0.0, -1.0, 0.0));

        let solver = ConstraintSolver::default();
        solver.solve_contact_constraint(
            &mut rb_a, &transform_a, &mut vel_a,
            &mut rb_b, &transform_b, &mut vel_b,
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            0.1, 0.5, 0.5, 0.016,
        );

        assert!(vel_a.linear.y < 1.0);
        assert!(vel_b.linear.y > -1.0);
    }

    #[test]
    fn test_normal_impulse_non_negative() {
        // Bodies approaching: normal impulse should push them apart, never pull.
        let mut rb_a = RigidBody::default();
        let mut rb_b = RigidBody::default();
        let transform_a = Transform::new(Vec3::ZERO);
        let transform_b = Transform::new(Vec3::new(0.0, 1.0, 0.0));
        let mut vel_a = Velocity::new(Vec3::new(0.0, 5.0, 0.0));
        let mut vel_b = Velocity::new(Vec3::new(0.0, -5.0, 0.0));

        let before_a = vel_a.linear;
        let before_b = vel_b.linear;

        let solver = ConstraintSolver::default();
        solver.solve_contact_constraint(
            &mut rb_a, &transform_a, &mut vel_a,
            &mut rb_b, &transform_b, &mut vel_b,
            Vec3::new(0.0, 0.5, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            0.05, 0.3, 0.0, 0.016,
        );

        // A should slow down (positive Y reduced), B should slow down (negative Y increased)
        assert!(vel_a.linear.y < before_a.y);
        assert!(vel_b.linear.y > before_b.y);
    }
}
