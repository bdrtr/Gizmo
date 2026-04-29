use crate::collision::ContactManifold;
use crate::components::{RigidBody, Transform, Velocity};
use gizmo_math::{Mat3, Vec3};

/// Physics constraint solver using Sequential Impulse method
pub struct ConstraintSolver {
    pub iterations: usize,
    pub baumgarte: f32, // Position correction factor
    pub slop: f32,      // Allowed penetration before correction
}

impl Default for ConstraintSolver {
    fn default() -> Self {
        Self {
            iterations: 10,
            baumgarte: 0.2,
            slop: 0.01,
        }
    }
}

impl ConstraintSolver {
    pub fn new(iterations: usize) -> Self {
        Self {
            iterations,
            ..Default::default()
        }
    }

    /// Solve all contact constraints
    pub fn solve_contacts(
        &self,
        manifolds: &[ContactManifold],
        bodies_a: &mut [(RigidBody, Transform, Velocity)],
        bodies_b: &mut [(RigidBody, Transform, Velocity)],
        dt: f32,
    ) {
        if manifolds.is_empty() {
            return;
        }

        // Warm starting could be added here for better convergence

        // Iteratively solve constraints
        for _ in 0..self.iterations {
            for (manifold_idx, manifold) in manifolds.iter().enumerate() {
                let (rb_a, transform_a, vel_a) = &mut bodies_a[manifold_idx];
                let (rb_b, transform_b, vel_b) = &mut bodies_b[manifold_idx];

                for contact in &manifold.contacts {
                    self.solve_contact_constraint(
                        rb_a,
                        transform_a,
                        vel_a,
                        rb_b,
                        transform_b,
                        vel_b,
                        contact.point,
                        contact.normal,
                        contact.penetration,
                        manifold.friction,
                        manifold.restitution,
                        dt,
                    );
                }
            }
        }
    }

    /// Solve a single contact constraint
    #[allow(clippy::too_many_arguments)]
    fn solve_contact_constraint(
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
        // Skip if both bodies are static or kinematic
        if !rb_a.is_dynamic() && !rb_b.is_dynamic() {
            return;
        }

        let r_a = contact_point - transform_a.position;
        let r_b = contact_point - transform_b.position;

        // Calculate relative velocity at contact point
        let vel_a_at_contact = vel_a.linear + vel_a.angular.cross(r_a);
        let vel_b_at_contact = vel_b.linear + vel_b.angular.cross(r_b);
        let relative_vel = vel_b_at_contact - vel_a_at_contact;

        let vel_along_normal = relative_vel.dot(normal);

        // Don't resolve if velocities are separating
        if vel_along_normal > 0.0 {
            return;
        }

        // Calculate impulse magnitude
        let inv_mass_a = rb_a.inv_mass();
        let inv_mass_b = rb_b.inv_mass();

        let inv_inertia_a = rb_a.inv_world_inertia_tensor(transform_a.rotation);
        let inv_inertia_b = rb_b.inv_world_inertia_tensor(transform_b.rotation);

        let r_a_cross_n = r_a.cross(normal);
        let r_b_cross_n = r_b.cross(normal);

        let angular_factor_a = (inv_inertia_a * r_a_cross_n).cross(r_a);
        let angular_factor_b = (inv_inertia_b * r_b_cross_n).cross(r_b);

        let inv_mass_sum = inv_mass_a + inv_mass_b + angular_factor_a.dot(normal) + angular_factor_b.dot(normal);

        if inv_mass_sum < 1e-6 {
            return;
        }

        // Apply restitution (bounciness)
        let e = restitution;
        let numerator = -(1.0 + e) * vel_along_normal;

        // Add position correction (Baumgarte stabilization)
        let bias = (self.baumgarte / dt) * (penetration - self.slop).max(0.0);
        let j = (numerator + bias) / inv_mass_sum;

        // Apply normal impulse
        let impulse = normal * j;

        if rb_a.is_dynamic() {
            vel_a.linear -= impulse * inv_mass_a;
            vel_a.angular -= inv_inertia_a * r_a.cross(impulse);
        }

        if rb_b.is_dynamic() {
            vel_b.linear += impulse * inv_mass_b;
            vel_b.angular += inv_inertia_b * r_b.cross(impulse);
        }

        // Friction
        self.apply_friction(
            rb_a,
            transform_a,
            vel_a,
            rb_b,
            transform_b,
            vel_b,
            r_a,
            r_b,
            normal,
            friction,
            j,
            &inv_inertia_a,
            &inv_inertia_b,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_friction(
        &self,
        rb_a: &RigidBody,
        _transform_a: &Transform,
        vel_a: &mut Velocity,
        rb_b: &RigidBody,
        _transform_b: &Transform,
        vel_b: &mut Velocity,
        r_a: Vec3,
        r_b: Vec3,
        normal: Vec3,
        friction: f32,
        normal_impulse: f32,
        inv_inertia_a: &Mat3,
        inv_inertia_b: &Mat3,
    ) {
        // Calculate tangent velocity
        let vel_a_at_contact = vel_a.linear + vel_a.angular.cross(r_a);
        let vel_b_at_contact = vel_b.linear + vel_b.angular.cross(r_b);
        let relative_vel = vel_b_at_contact - vel_a_at_contact;

        let tangent_vel = relative_vel - normal * relative_vel.dot(normal);
        let tangent_vel_mag = tangent_vel.length();

        if tangent_vel_mag < 1e-6 {
            return;
        }

        let tangent = tangent_vel / tangent_vel_mag;

        let inv_mass_a = rb_a.inv_mass();
        let inv_mass_b = rb_b.inv_mass();

        let r_a_cross_t = r_a.cross(tangent);
        let r_b_cross_t = r_b.cross(tangent);

        let angular_factor_a = (*inv_inertia_a * r_a_cross_t).cross(r_a);
        let angular_factor_b = (*inv_inertia_b * r_b_cross_t).cross(r_b);

        let inv_mass_sum = inv_mass_a + inv_mass_b + angular_factor_a.dot(tangent) + angular_factor_b.dot(tangent);

        if inv_mass_sum < 1e-6 {
            return;
        }

        let jt = -tangent_vel_mag / inv_mass_sum;

        // Coulomb friction
        let friction_impulse = if jt.abs() < friction * normal_impulse.abs() {
            tangent * jt
        } else {
            tangent * (-friction * normal_impulse.abs())
        };

        if rb_a.is_dynamic() {
            vel_a.linear -= friction_impulse * inv_mass_a;
            vel_a.angular -= *inv_inertia_a * r_a.cross(friction_impulse);
        }

        if rb_b.is_dynamic() {
            vel_b.linear += friction_impulse * inv_mass_b;
            vel_b.angular += *inv_inertia_b * r_b.cross(friction_impulse);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_math::Quat;

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
            &mut rb_a,
            &transform_a,
            &mut vel_a,
            &mut rb_b,
            &transform_b,
            &mut vel_b,
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            0.1,
            0.5,
            0.5,
            0.016,
        );

        // After collision, velocities should change
        assert!(vel_a.linear.y < 1.0);
        assert!(vel_b.linear.y > -1.0);
    }
}
