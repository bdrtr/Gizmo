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
        entity: gizmo_core::entity::Entity,
        rb: &mut RigidBody,
        vel: &mut Velocity,
        dt: f32,
    ) -> Result<(), crate::error::GizmoError> {
        if !rb.is_dynamic() || rb.is_sleeping {
            return Ok(());
        }

        // NaN Check before any mathematical operations
        if vel.linear.x.is_nan() || vel.linear.y.is_nan() || vel.linear.z.is_nan() ||
           vel.angular.x.is_nan() || vel.angular.y.is_nan() || vel.angular.z.is_nan() ||
           rb.linear_damping.is_nan() || rb.angular_damping.is_nan() {
            return Err(crate::error::GizmoError::NaNVelocity(entity));
        }

        // Apply gravity
        if rb.use_gravity {
            vel.linear += self.gravity * dt;
        }

        let lin_decay = (-rb.linear_damping  * dt).exp();
        let ang_decay = (-rb.angular_damping * dt).exp();
        vel.linear  *= lin_decay;
        vel.angular *= ang_decay;
        
        // Mathematical Sanity Check (Only runs in debug mode)
        debug_assert!(vel.linear.x.is_finite() && vel.linear.y.is_finite() && vel.linear.z.is_finite(), "Linear velocity hit infinity!");
        debug_assert!(vel.angular.x.is_finite() && vel.angular.y.is_finite() && vel.angular.z.is_finite(), "Angular velocity hit infinity!");

        // Enforce any axis locks (e.g. for 2.5D)
        let old_lin = vel.linear;
        let old_ang = vel.angular;
        rb.enforce_locks(vel);
        
        if old_lin != vel.linear || old_ang != vel.angular {
            tracing::trace!("2.5D (or axis lock) kısıtlaması uygulandı: Entity {:?}", entity);
        }

        // Update sleep state
        rb.update_sleep_state(vel);
        Ok(())
    }

    /// Integrate positions from velocities
    pub fn integrate_positions(
        &self,
        entity: gizmo_core::entity::Entity,
        rb: &RigidBody,
        transform: &mut Transform,
        vel: &Velocity,
        dt: f32,
    ) -> Result<(), crate::error::GizmoError> {
        if rb.is_static() || rb.is_sleeping {
            return Ok(());
        }

        let mut masked_vel = *vel;
        rb.enforce_locks(&mut masked_vel);

        // Update position
        transform.position += masked_vel.linear * dt;

        if transform.position.x.is_nan() || transform.position.y.is_nan() || transform.position.z.is_nan() {
            return Err(crate::error::GizmoError::NaNPosition(entity));
        }

        // Update rotation using quaternion integration
        if masked_vel.angular.length_squared() > 1e-8 {
            let angular_vel_quat = Quat::from_scaled_axis(masked_vel.angular * dt);
            transform.rotation = (transform.rotation * angular_vel_quat).normalize();
            
            if transform.rotation.x.is_nan() || transform.rotation.y.is_nan() || transform.rotation.z.is_nan() || transform.rotation.w.is_nan() {
                return Err(crate::error::GizmoError::NaNPosition(entity));
            }
        }

        // Update transform matrix
        transform.update_local_matrix();
        
        // Mathematical Sanity Check
        debug_assert!(transform.position.x.is_finite() && transform.position.y.is_finite() && transform.position.z.is_finite(), "Position hit infinity!");
        
        Ok(())
    }

    /// Full integration step (velocity + position)
    pub fn integrate(
        &self,
        entity: gizmo_core::entity::Entity,
        rb: &mut RigidBody,
        transform: &mut Transform,
        vel: &mut Velocity,
        dt: f32,
    ) -> Result<(), crate::error::GizmoError> {
        self.integrate_velocities(entity, rb, vel, dt)?;
        self.integrate_positions(entity, rb, transform, vel, dt)?;
        Ok(())
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

        let global_com = transform.position + transform.rotation.mul_vec3(rb.center_of_mass);
        let r = point - global_com;
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
        rb.wake_up(); // Ensure body is awake
        let mut vel = Velocity::default();
        let entity = gizmo_core::entity::Entity::new(0, 0);

        integrator.integrate_velocities(entity, &mut rb, &mut vel, 1.0).unwrap();

        // After 1 second, velocity should be approximately gravity * exp(-damping * dt)
        let expected_vel = integrator.gravity.y * (-rb.linear_damping * 1.0_f32).exp();
        assert!((vel.linear.y - expected_vel).abs() < 0.1);
    }

    #[test]
    fn test_position_integration() {
        let integrator = Integrator::default();
        let rb = RigidBody::default();
        let mut transform = Transform::new(Vec3::ZERO);
        let vel = Velocity::new(Vec3::new(1.0, 0.0, 0.0));
        let entity = gizmo_core::entity::Entity::new(0, 0);

        integrator.integrate_positions(entity, &rb, &mut transform, &vel, 1.0).unwrap();

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
        let entity = gizmo_core::entity::Entity::new(0, 0);

        integrator.integrate_velocities(entity, &mut rb, &mut vel, 1.0).unwrap();

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

    #[test]
    fn test_2_5d_constraints() {
        let integrator = Integrator::default();
        let mut rb = RigidBody::default();
        rb.body_type = crate::components::BodyType::Dynamic;
        // Lock Z axis movement and X/Y rotations (Classic 2.5D Platformer locks)
        rb.lock_translation_z = true;
        rb.lock_rotation_x = true;
        rb.lock_rotation_y = true;
        
        // Apply explosive velocity in all directions
        let mut vel = Velocity::new(Vec3::new(10.0, 5.0, -100.0));
        vel.angular = Vec3::new(10.0, 10.0, 10.0);
        
        let entity = gizmo_core::entity::Entity::new(0, 0);

        // Run velocity integration
        integrator.integrate_velocities(entity, &mut rb, &mut vel, 1.0).unwrap();

        // Linear X and Y should remain (slightly damped), Z should be aggressively set to 0.0
        assert!(vel.linear.x > 0.0);
        assert_eq!(vel.linear.z, 0.0);
        
        // Angular X and Y should be strictly 0.0, Z should remain (damped)
        assert_eq!(vel.angular.x, 0.0);
        assert_eq!(vel.angular.y, 0.0);
        assert!(vel.angular.z > 0.0);
    }
}
