use crate::components::{RigidBody, Transform, Velocity};
use gizmo_math::{Quat, Vec3};

/// Semi-implicit Euler physics integrator.
///
/// Velocity is updated first (with forces & damping), then position is
/// integrated from the new velocity.  This order gives better energy
/// conservation than explicit Euler at essentially no extra cost.
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

    // ------------------------------------------------------------------ //
    //  Velocity integration                                               //
    // ------------------------------------------------------------------ //

    /// Apply forces (gravity, accumulated forces) and damping, then update
    /// velocity with semi-implicit Euler.
    ///
    /// Returns [`GizmoError::NaNVelocity`] when a NaN is detected *before*
    /// any mutation so the caller receives the body in its pre-error state.
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

        // ── Pre-mutation NaN guard ────────────────────────────────────────
        if !vel.linear.is_finite() || !vel.angular.is_finite() {
            return Err(crate::error::GizmoError::NaNVelocity(entity));
        }
        if rb.linear_damping.is_nan() || rb.angular_damping.is_nan() {
            return Err(crate::error::GizmoError::NaNVelocity(entity));
        }

        // ── Gravity ───────────────────────────────────────────────────────
        if rb.use_gravity {
            vel.linear += self.gravity * dt;
        }

        // ── Accumulated forces / torques ──────────────────────────────────
        // Drain the accumulator so forces are applied exactly once per step.
        let inv_mass = rb.inv_mass();
        if inv_mass > 0.0 {
            vel.linear += rb.force_accumulator * inv_mass * dt;
            let inv_inertia = rb.inv_world_inertia_tensor_identity(); // body-space shortcut
            vel.angular += inv_inertia * rb.torque_accumulator * dt;
        }
        rb.clear_forces();

        // ── Exponential damping ───────────────────────────────────────────
        // exp(-d*dt) keeps energy decay frame-rate independent.
        vel.linear *= (-rb.linear_damping * dt).exp();
        vel.angular *= (-rb.angular_damping * dt).exp();

        // ── Axis locks (e.g. 2.5-D platformer) ───────────────────────────
        let pre_lin = vel.linear;
        let pre_ang = vel.angular;
        rb.enforce_locks(vel);

        if vel.linear != pre_lin || vel.angular != pre_ang {
            tracing::trace!(
                "Axis-lock constraint applied: entity={:?}  Δlin={:?}  Δang={:?}",
                entity,
                vel.linear - pre_lin,
                vel.angular - pre_ang,
            );
        }

        // ── Post-mutation sanity (debug only) ─────────────────────────────
        debug_assert!(
            vel.linear.is_finite(),
            "Linear velocity became non-finite after integration! entity={entity:?}"
        );
        debug_assert!(
            vel.angular.is_finite(),
            "Angular velocity became non-finite after integration! entity={entity:?}"
        );

        // ── Speed warning ─────────────────────────────────────────────────
        let speed_sq = vel.linear.length_squared();
        if speed_sq > 1_000_000.0 {
            tracing::warn!(
                "Entity {:?} is moving at {:.1} m/s — tunneling / explosion risk.",
                entity,
                speed_sq.sqrt(),
            );
        }

        // ── Sleep bookkeeping ─────────────────────────────────────────────
        rb.update_sleep_state(vel);

        Ok(())
    }

    // ------------------------------------------------------------------ //
    //  Position integration                                               //
    // ------------------------------------------------------------------ //

    /// Integrate translation and rotation from the current velocity.
    ///
    /// Rotation is updated with quaternion axis-angle integration, which
    /// remains accurate for large angular velocities without the drift that
    /// Euler-angle approaches suffer from.
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

        // Apply axis locks to a local copy — do not mutate the stored velocity here.
        let mut masked = *vel;
        rb.enforce_locks(&mut masked);

        // ── Translation ───────────────────────────────────────────────────
        transform.position += masked.linear * dt;

        if !transform.position.is_finite() {
            return Err(crate::error::GizmoError::NaNPosition(entity));
        }

        // ── Rotation ──────────────────────────────────────────────────────
        // Only integrate when angular speed is non-negligible to avoid
        // normalising a near-zero quaternion.
        let ang_speed_sq = masked.angular.length_squared();
        if ang_speed_sq > 1e-8 {
            let delta_rot = Quat::from_scaled_axis(masked.angular * dt);
            transform.rotation = (delta_rot * transform.rotation).normalize();

            if !transform.rotation.is_finite() {
                return Err(crate::error::GizmoError::NaNPosition(entity));
            }
        }

        // ── Rebuild local matrix ──────────────────────────────────────────
        transform.update_local_matrix();

        debug_assert!(
            transform.position.is_finite(),
            "Position became non-finite after integration! entity={entity:?}"
        );

        Ok(())
    }

    // ------------------------------------------------------------------ //
    //  Combined step                                                      //
    // ------------------------------------------------------------------ //

    /// Convenience: velocity integration followed by position integration.
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

    // ------------------------------------------------------------------ //
    //  Force / impulse helpers                                            //
    // ------------------------------------------------------------------ //

    /// Apply an instantaneous impulse at a world-space point.
    ///
    /// Produces both a linear velocity change and a torque-impulse about the
    /// centre of mass.
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

        let inv_mass = rb.inv_mass();
        vel.linear += impulse * inv_mass;

        let global_com = transform.position + transform.rotation.mul_vec3(rb.center_of_mass);
        let r = point - global_com;
        let inv_inertia = rb.inv_world_inertia_tensor(transform.rotation);
        vel.angular += inv_inertia * r.cross(impulse);
    }

    /// Apply a continuous force at a world-space point over `dt`.
    ///
    /// Internally converts the force to an impulse (`F·dt`) and delegates to
    /// [`apply_impulse_at_point`].
    pub fn apply_force_at_point(
        rb: &RigidBody,
        transform: &Transform,
        vel: &mut Velocity,
        force: Vec3,
        point: Vec3,
        dt: f32,
    ) {
        Self::apply_impulse_at_point(rb, transform, vel, force * dt, point);
    }

    /// Apply a central force (at the centre of mass, no torque).
    pub fn apply_force(rb: &RigidBody, vel: &mut Velocity, force: Vec3, dt: f32) {
        if !rb.is_dynamic() {
            return;
        }
        vel.linear += force * rb.inv_mass() * dt;
    }

    /// Apply a pure torque (no linear effect).
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

// ======================================================================= //
//  Tests                                                                   //
// ======================================================================= //

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entity(id: u32) -> gizmo_core::entity::Entity {
        gizmo_core::entity::Entity::new(id, 0)
    }

    // ------------------------------------------------------------------ //

    #[test]
    fn gravity_accelerates_downward() {
        let integrator = Integrator::default();
        let mut rb = RigidBody::default();
        rb.wake_up();

        let mut vel = Velocity::default();
        let entity = make_entity(0);

        integrator
            .integrate_velocities(entity, &mut rb, &mut vel, 1.0)
            .expect("integration must succeed");

        // After 1 s the expected vy = gravity.y * exp(-linear_damping * dt)
        let expected_vy = integrator.gravity.y * (-rb.linear_damping * 1.0_f32).exp();
        assert!(
            (vel.linear.y - expected_vy).abs() < 0.01,
            "vy={} expected≈{}",
            vel.linear.y,
            expected_vy
        );
    }

    #[test]
    fn position_advances_with_velocity() {
        let integrator = Integrator::default();
        let _rb = RigidBody::default(); // static by default → should still integrate
        let mut transform = Transform::new(Vec3::ZERO);
        let vel = Velocity::new(Vec3::new(1.0, 0.0, 0.0));
        let entity = make_entity(1);

        // Use a dynamic body so position integration actually runs.
        let mut dynamic_rb = RigidBody::default();
        dynamic_rb.body_type = crate::components::BodyType::Dynamic;
        dynamic_rb.wake_up();

        integrator
            .integrate_positions(entity, &dynamic_rb, &mut transform, &vel, 1.0)
            .expect("position integration must succeed");

        assert!(
            (transform.position.x - 1.0).abs() < 0.001,
            "position.x={} expected≈1.0",
            transform.position.x
        );
    }

    #[test]
    fn damping_reduces_velocity() {
        let integrator = Integrator::default();
        let mut rb = RigidBody {
            linear_damping: 0.1,
            ..Default::default()
        };
        rb.body_type = crate::components::BodyType::Dynamic;
        rb.wake_up();

        let mut vel = Velocity::new(Vec3::new(10.0, 0.0, 0.0));
        // Disable gravity so only damping acts on the velocity.
        rb.use_gravity = false;

        integrator
            .integrate_velocities(make_entity(2), &mut rb, &mut vel, 1.0)
            .expect("integration must succeed");

        assert!(
            vel.linear.x < 10.0,
            "damping must reduce velocity; got {}",
            vel.linear.x
        );
        assert!(vel.linear.x > 0.0, "velocity must stay positive");
    }

    #[test]
    fn impulse_changes_linear_velocity() {
        let mut rb = RigidBody::default();
        rb.body_type = crate::components::BodyType::Dynamic;

        let transform = Transform::new(Vec3::ZERO);
        let mut vel = Velocity::default();
        let impulse = Vec3::new(10.0, 0.0, 0.0);

        Integrator::apply_impulse_at_point(&rb, &transform, &mut vel, impulse, Vec3::ZERO);

        assert!(vel.linear.x > 0.0, "impulse must produce positive vx");
    }

    #[test]
    fn impulse_off_center_also_creates_torque() {
        let mut rb = RigidBody::default();
        rb.body_type = crate::components::BodyType::Dynamic;

        let transform = Transform::new(Vec3::ZERO);
        let mut vel = Velocity::default();
        // Apply impulse at +Y offset — should create angular velocity around Z.
        let point = Vec3::new(0.0, 1.0, 0.0);
        let impulse = Vec3::new(1.0, 0.0, 0.0);

        Integrator::apply_impulse_at_point(&rb, &transform, &mut vel, impulse, point);

        assert!(vel.linear.x > 0.0, "must have linear response");
        assert!(
            vel.angular.length_squared() > 0.0,
            "off-centre impulse must produce angular velocity"
        );
    }

    #[test]
    fn axis_locks_enforce_2_5d_constraints() {
        let integrator = Integrator::default();
        let mut rb = RigidBody::default();
        rb.body_type = crate::components::BodyType::Dynamic;
        rb.lock_translation_z = true;
        rb.lock_rotation_x = true;
        rb.lock_rotation_y = true;
        rb.wake_up();

        let mut vel = Velocity::new(Vec3::new(10.0, 5.0, -100.0));
        vel.angular = Vec3::new(10.0, 10.0, 10.0);

        integrator
            .integrate_velocities(make_entity(3), &mut rb, &mut vel, 1.0)
            .expect("integration must succeed");

        // Planar axes must survive (may be damped).
        assert!(vel.linear.x > 0.0, "X velocity must remain");
        assert!(vel.linear.y != 0.0, "Y velocity must remain");

        // Locked axis must be zeroed.
        assert_eq!(vel.linear.z, 0.0, "Z translation must be locked");
        assert_eq!(vel.angular.x, 0.0, "X rotation must be locked");
        assert_eq!(vel.angular.y, 0.0, "Y rotation must be locked");

        // Free rotation axis must survive.
        assert!(vel.angular.z > 0.0, "Z rotation must remain");
    }

    #[test]
    fn nan_velocity_returns_error() {
        let integrator = Integrator::default();
        let mut rb = RigidBody::default();
        rb.body_type = crate::components::BodyType::Dynamic;
        rb.wake_up();

        let mut vel = Velocity::new(Vec3::new(f32::NAN, 0.0, 0.0));

        let result = integrator.integrate_velocities(make_entity(4), &mut rb, &mut vel, 1.0);
        assert!(result.is_err(), "NaN velocity must return an error");
    }

    #[test]
    fn sleeping_body_is_not_integrated() {
        let integrator = Integrator::default();
        let mut rb = RigidBody::default();
        rb.body_type = crate::components::BodyType::Dynamic;
        // Do NOT call wake_up() — body stays asleep.

        let mut vel = Velocity::default();
        integrator
            .integrate_velocities(make_entity(5), &mut rb, &mut vel, 1.0)
            .expect("sleeping body integration must be a no-op");

        assert_eq!(
            vel.linear,
            Vec3::ZERO,
            "sleeping body must not gain velocity"
        );
    }
}
