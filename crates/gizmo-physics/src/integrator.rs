use crate::components::{RigidBody, Transform, Velocity};
use gizmo_math::{Quat, Vec3};

/// Physics integrator for updating positions and velocities
pub struct Integrator {
    pub gravity: Vec3,
}

impl Default for Integrator {
    fn default() -> Self {
        Self {
            gravity: Vec3::new(0.0, -9.81, 0.0),
        }
    }
}

impl Integrator {
    pub fn new(gravity: Vec3) -> Self {
        Self { gravity }
    }

    /// Apply forces and integrate velocities (Semi-implicit Euler)
    pub fn integrate_velocities(
        &self,
        rb: &mut RigidBody,
        vel: &mut Velocity,
        dt: f32,
    ) {
        if !rb.is_dynamic() || rb.is_sleeping {
            return;
        }

        // Apply gravity
        if rb.use_gravity {
            vel.linear += self.gravity * dt;
        }

        // Apply damping
        vel.linear *= 1.0 - rb.linear_damping.min(1.0);
        vel.angular *= 1.0 - rb.angular_damping.min(1.0);

        // Update sleep state
        rb.update_sleep_state(vel);
    }

    /// Integrate positions from velocities
    pub fn integrate_positions(
        &self,
        rb: &RigidBody,
        transform: &mut Transform,
        vel: &Velocity,
        dt: f32,
    ) {
        if rb.is_static() || rb.is_sleeping {
            return;
        }

        // Update position
        transform.position += vel.linear * dt;

        // Update rotation using quaternion integration
        if vel.angular.length_squared() > 1e-8 {
            let angular_vel_quat = Quat::from_scaled_axis(vel.angular * dt * 0.5);
            transform.rotation = (transform.rotation * angular_vel_quat).normalize();
        }

        // Update transform matrix
        transform.update_local_matrix();
    }

    /// Full integration step (velocity + position)
    pub fn integrate(
        &self,
        rb: &mut RigidBody,
        transform: &mut Transform,
        vel: &mut Velocity,
        dt: f32,
    ) {
        self.integrate_velocities(rb, vel, dt);
        self.integrate_positions(rb, transform, vel, dt);
    }

    /// Apply an impulse at a point
    pub fn apply_impulse_at_point(
        rb: &RigidBody,
        transform: &Transform,
        vel: &mut Velocity,
        impulse: Vec3,
        point: Vec3,
    ) {
        if !rb.is_dynamic() {
            return;
        }

        vel.linear += impulse * rb.inv_mass();

        let r = point - transform.position;
        let torque_impulse = r.cross(impulse);
        let inv_inertia = rb.inv_world_inertia_tensor(transform.rotation);
        vel.angular += inv_inertia * torque_impulse;
    }

    /// Apply a force at a point (will be integrated over dt)
    pub fn apply_force_at_point(
        rb: &RigidBody,
        transform: &Transform,
        vel: &mut Velocity,
        force: Vec3,
        point: Vec3,
        dt: f32,
    ) {
        let impulse = force * dt;
        Self::apply_impulse_at_point(rb, transform, vel, impulse, point);
    }

    /// Apply central force (at center of mass)
    pub fn apply_force(
        rb: &RigidBody,
        vel: &mut Velocity,
        force: Vec3,
        dt: f32,
    ) {
        if !rb.is_dynamic() {
            return;
        }

        vel.linear += force * rb.inv_mass() * dt;
    }

    /// Apply torque
    pub fn apply_torque(
        rb: &RigidBody,
        transform: &Transform,
        vel: &mut Velocity,
        torque: Vec3,
        dt: f32,
    ) {
        if !rb.is_dynamic() {
            return;
        }

        let inv_inertia = rb.inv_world_inertia_tensor(transform.rotation);
        vel.angular += inv_inertia * torque * dt;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gravity_integration() {
        let integrator = Integrator::default();
        let mut rb = RigidBody::default();
        let mut vel = Velocity::default();

        integrator.integrate_velocities(&mut rb, &mut vel, 1.0);

        // After 1 second, velocity should be gravity
        assert!((vel.linear.y - integrator.gravity.y).abs() < 0.01);
    }

    #[test]
    fn test_position_integration() {
        let integrator = Integrator::default();
        let rb = RigidBody::default();
        let mut transform = Transform::new(Vec3::ZERO);
        let vel = Velocity::new(Vec3::new(1.0, 0.0, 0.0));

        integrator.integrate_positions(&rb, &mut transform, &vel, 1.0);

        // After 1 second at 1 m/s, should move 1 meter
        assert!((transform.position.x - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_damping() {
        let integrator = Integrator::default();
        let mut rb = RigidBody {
            linear_damping: 0.1,
            ..Default::default()
        };
        let mut vel = Velocity::new(Vec3::new(10.0, 0.0, 0.0));

        integrator.integrate_velocities(&mut rb, &mut vel, 1.0);

        // Velocity should be reduced by damping
        assert!(vel.linear.x < 10.0);
    }

    #[test]
    fn test_impulse_application() {
        let rb = RigidBody::default();
        let transform = Transform::new(Vec3::ZERO);
        let mut vel = Velocity::default();

        let impulse = Vec3::new(10.0, 0.0, 0.0);
        Integrator::apply_impulse_at_point(&rb, &transform, &mut vel, impulse, Vec3::ZERO);

        // Linear velocity should change
        assert!(vel.linear.x > 0.0);
    }
}
