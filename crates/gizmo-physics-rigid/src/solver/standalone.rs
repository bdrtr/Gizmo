use super::ConstraintSolver;
use crate::components::{RigidBody, Velocity};
use gizmo_math::{Mat3, Vec3};
use gizmo_physics_core::components::Transform;

impl ConstraintSolver {
    // ─────────────────────────────────────────────────────────────────────────
    // Tek temas noktası için standalone solver (geriye dönük uyum)
    // ─────────────────────────────────────────────────────────────────────────

    /// Resolve one contact point between two bodies in isolation, applying a single
    /// normal impulse plus one friction impulse directly to `vel_a` / `vel_b`.
    ///
    /// A self-contained convenience path for embedders who do their own collision
    /// detection and just want a correct impulse response; the world's own stepping never
    /// calls it. It is a one-shot solve, not a substitute for the island solver: there is
    /// no warm-starting, no impulse accumulated across calls or iterations, and no
    /// coupling to the other contacts of the same manifold, so a stack or a resting pile
    /// solved this way will sag and jitter. Positional error is corrected with Baumgarte
    /// bias mixed into the velocity, regardless of the split-impulse / TGS-soft settings.
    ///
    /// Frames and units:
    /// - `contact_point` — world space, metres. Lever arms are measured from each body's
    ///   *centre of mass* (`transform.position` plus the rotated `center_of_mass` offset),
    ///   not from the transform origin.
    /// - `normal` — world space, must be unit length, oriented from `rb_a` toward `rb_b`.
    ///   `rb_a` is pushed along `−normal` and `rb_b` along `+normal`.
    /// - `penetration` — overlap depth in metres, positive when the shapes intersect. Only
    ///   the part above `slop` is corrected, and it is clamped at zero, so a negative
    ///   (speculative, not-yet-touching) value is treated as no penetration rather than as
    ///   a gap to close.
    /// - `static_friction` / `dynamic_friction` — dimensionless Coulomb coefficients.
    /// - `restitution` — bounciness in `0..=1`; ignored (treated as 0) unless the approach
    ///   speed exceeds [`restitution_velocity_threshold`](Self::restitution_velocity_threshold),
    ///   which is what keeps resting contacts from micro-bouncing.
    /// - `dt` — substep duration in seconds; used only to scale the Baumgarte bias. `dt`
    ///   of zero or less disables that bias instead of dividing by zero.
    ///
    /// Does nothing at all when neither body is dynamic, when the pair is already
    /// separating at this point, or when the combined inverse mass along the normal is
    /// negligible — the pair is then effectively immovable and the impulse would be
    /// unbounded. The rigid bodies are taken by `&mut` but are only read — the only
    /// mutations are to the two velocities. Sleep state is neither checked nor updated, so
    /// applying this to a sleeping body changes a velocity that the integrator will then
    /// ignore until something wakes the body.
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
        static_friction: f32,
        dynamic_friction: f32,
        restitution: f32,
        dt: f32,
    ) {
        if !rb_a.is_dynamic() && !rb_b.is_dynamic() {
            return;
        }

        let com_a = transform_a.position + transform_a.rotation.mul_vec3(rb_a.center_of_mass);
        let com_b = transform_b.position + transform_b.rotation.mul_vec3(rb_b.center_of_mass);

        let r_a = contact_point - com_a;
        let r_b = contact_point - com_b;

        let va = vel_a.linear + vel_a.angular.cross(r_a);
        let vb = vel_b.linear + vel_b.angular.cross(r_b);
        let rel_vel = vb - va;
        let vel_norm = rel_vel.dot(normal);

        if vel_norm > 0.0 {
            return;
        } // Ayrılıyor, işlem yapma

        let inv_m_a = rb_a.inv_mass();
        let inv_m_b = rb_b.inv_mass();
        let inv_i_a = rb_a.inv_world_inertia_tensor(transform_a.rotation);
        let inv_i_b = rb_b.inv_world_inertia_tensor(transform_b.rotation);

        let r_a_x_n = r_a.cross(normal);
        let r_b_x_n = r_b.cross(normal);
        let k = inv_m_a
            + inv_m_b
            + (inv_i_a.mul_vec3(r_a_x_n)).dot(r_a_x_n)
            + (inv_i_b.mul_vec3(r_b_x_n)).dot(r_b_x_n);

        if k < 1e-8 {
            return;
        }

        let inv_dt = if dt > 0.0 { 1.0 / dt } else { 0.0 };
        let bias = self.baumgarte * inv_dt * (penetration - self.slop).max(0.0);
        let e = if -vel_norm > self.restitution_velocity_threshold {
            restitution
        } else {
            0.0
        };
        let j = ((-(1.0 + e) * vel_norm + bias) / k).max(0.0);

        let impulse = normal * j;

        if rb_a.is_dynamic() {
            vel_a.linear -= impulse * inv_m_a;
            vel_a.angular -= inv_i_a.mul_vec3(r_a.cross(impulse));
        }
        if rb_b.is_dynamic() {
            vel_b.linear += impulse * inv_m_b;
            vel_b.angular += inv_i_b.mul_vec3(r_b.cross(impulse));
        }

        // Sürtünme
        self.apply_friction_standalone(
            rb_a,
            vel_a,
            rb_b,
            vel_b,
            r_a,
            r_b,
            normal,
            static_friction,
            dynamic_friction,
            j,
            &inv_i_a,
            &inv_i_b,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_friction_standalone(
        &self,
        rb_a: &RigidBody,
        vel_a: &mut Velocity,
        rb_b: &RigidBody,
        vel_b: &mut Velocity,
        r_a: Vec3,
        r_b: Vec3,
        normal: Vec3,
        static_friction: f32,
        dynamic_friction: f32,
        normal_impulse: f32,
        inv_i_a: &Mat3,
        inv_i_b: &Mat3,
    ) {
        let va = vel_a.linear + vel_a.angular.cross(r_a);
        let vb = vel_b.linear + vel_b.angular.cross(r_b);
        let rel = vb - va;
        let tang_v = rel - normal * rel.dot(normal);
        let tang_mag = tang_v.length();
        if tang_mag < 1e-8 {
            return;
        }
        let tangent = tang_v / tang_mag;

        let inv_m_a = rb_a.inv_mass();
        let inv_m_b = rb_b.inv_mass();

        let r_a_x_t = r_a.cross(tangent);
        let r_b_x_t = r_b.cross(tangent);
        let k = inv_m_a
            + inv_m_b
            + (inv_i_a.mul_vec3(r_a_x_t)).dot(r_a_x_t)
            + (inv_i_b.mul_vec3(r_b_x_t)).dot(r_b_x_t);
        if k < 1e-8 {
            return;
        }

        let max_static = static_friction * normal_impulse.abs();
        let max_dynamic = dynamic_friction * normal_impulse.abs();

        let delta_t = -tang_mag / k;

        let jt = if delta_t.abs() <= max_static {
            delta_t
        } else {
            delta_t.signum() * max_dynamic
        };

        let ft = tangent * jt;
        if rb_a.is_dynamic() {
            vel_a.linear -= ft * inv_m_a;
            vel_a.angular -= inv_i_a.mul_vec3(r_a.cross(ft));
        }
        if rb_b.is_dynamic() {
            vel_b.linear += ft * inv_m_b;
            vel_b.angular += inv_i_b.mul_vec3(r_b.cross(ft));
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Testler
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)] // testlerde Default sonrası alan atama okunabilirlik için
mod tests {
    use super::*;
    use gizmo_math::Vec3;

    #[test]
    fn test_solver_creation() {
        let solver = ConstraintSolver::new(20);
        assert_eq!(solver.iterations, 20);
    }

    #[test]
    fn test_collision_response() {
        let mut rb_a = RigidBody::default();
        let mut rb_b = RigidBody::default();
        rb_a.wake_up();
        rb_b.wake_up();
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
            0.6,
            0.5,
            0.5,
            0.016,
        );

        assert!(vel_a.linear.y < 1.0);
        assert!(vel_b.linear.y > -1.0);
    }

    #[test]
    fn test_normal_impulse_non_negative() {
        let mut rb_a = RigidBody::default();
        let mut rb_b = RigidBody::default();
        rb_a.wake_up();
        rb_b.wake_up();
        let transform_a = Transform::new(Vec3::ZERO);
        let transform_b = Transform::new(Vec3::new(0.0, 1.0, 0.0));
        let mut vel_a = Velocity::new(Vec3::new(0.0, 5.0, 0.0));
        let mut vel_b = Velocity::new(Vec3::new(0.0, -5.0, 0.0));

        let before_a = vel_a.linear.y;
        let before_b = vel_b.linear.y;

        let solver = ConstraintSolver::default();
        solver.solve_contact_constraint(
            &mut rb_a,
            &transform_a,
            &mut vel_a,
            &mut rb_b,
            &transform_b,
            &mut vel_b,
            Vec3::new(0.0, 0.5, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            0.05,
            0.4,
            0.3,
            0.0,
            0.016,
        );

        assert!(vel_a.linear.y < before_a);
        assert!(vel_b.linear.y > before_b);
    }

    #[test]
    fn test_resting_contact_no_bounce() {
        // Çok yavaş çarpışma → restitution sıfır olmalı
        let mut rb_a = RigidBody::default();
        let mut rb_b = RigidBody::default();
        rb_b.body_type = crate::components::rigid_body::BodyType::Static;
        rb_a.wake_up();
        let transform_a = Transform::new(Vec3::new(0.0, 1.05, 0.0));
        let transform_b = Transform::new(Vec3::ZERO);
        let mut vel_a = Velocity::new(Vec3::new(0.0, -0.1, 0.0)); // Çok yavaş düşüyor
        let mut vel_b = Velocity::default();

        let solver = ConstraintSolver::default();
        // Solver convention: rel_vel = vb - va, vel_norm = rel_vel.dot(normal)
        // A(y=1.05) düşüyor, B(y=0) duruyor. Normal A→B yönünde = (0,-1,0)
        // rel_vel = (0,0,0) - (0,-0.1,0) = (0,0.1,0)
        // vel_norm = (0,0.1,0).dot(0,-1,0) = -0.1 < 0 → yaklaşıyor ✓
        solver.solve_contact_constraint(
            &mut rb_a,
            &transform_a,
            &mut vel_a,
            &mut rb_b,
            &transform_b,
            &mut vel_b,
            Vec3::new(0.0, 0.525, 0.0), // temas noktası (iki cismin arası)
            Vec3::new(0.0, -1.0, 0.0),  // normal: A'dan B'ye (aşağı)
            0.05,
            0.6,
            0.5,
            0.0,
            0.016, // restitution=0.0 (resting contact)
        );
        assert!(
            vel_a.linear.y >= -0.01,
            "A should not bounce or sink significantly: {}",
            vel_a.linear.y
        );
        assert!(
            vel_b.linear.y <= 0.01,
            "B (static) should remain still: {}",
            vel_b.linear.y
        );
    }
}
