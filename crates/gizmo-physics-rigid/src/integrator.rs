use crate::components::{RigidBody, Velocity};
use gizmo_physics_core::components::Transform;
use gizmo_physics_core::BodyHandle;
use gizmo_math::{Quat, Vec3};

/// Semi-implicit Euler physics integrator.
///
/// Velocity is updated first (with forces & damping), then position is
/// integrated from the new velocity.  This order gives better energy
/// conservation than explicit Euler at essentially no extra cost.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Integrator {
    pub gravity: Vec3,
    /// Hava yoğunluğu ρ (kg/m³), aerodinamik sürükleme F = ½·ρ·Cd·A·v² için. Deniz
    /// seviyesi ~1.225. Sürükleme yalnız gövdenin `drag_coefficient·drag_area > 0` ise
    /// uygulanır (opt-in, [`RigidBody::with_air_drag`]).
    pub air_density: f32,
    /// Rüzgar (hava kütlesinin) hızı, m/s. Sürükleme bağıl hıza (v − wind) karşı
    /// uygulanır → rüzgar tüneli/esinti: durağan cisim bile rüzgar yönünde itilir.
    /// Varsayılan sıfır (durağan hava).
    pub wind: Vec3,
}

impl Default for Integrator {
    fn default() -> Self {
        Self {
            gravity: Vec3::new(0.0, -9.81, 0.0),
            air_density: 1.225,
            wind: Vec3::ZERO,
        }
    }
}

impl Integrator {
    pub fn new(gravity: Vec3) -> Self {
        Self {
            gravity,
            ..Default::default()
        }
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
        entity: BodyHandle,
        rb: &mut RigidBody,
        rotation: Quat,
        vel: &mut Velocity,
        dt: f32,
    ) -> Result<(), gizmo_physics_core::GizmoError> {
        if !rb.is_dynamic() || rb.is_sleeping {
            return Ok(());
        }

        // ── Pre-mutation NaN guard ────────────────────────────────────────
        if !vel.linear.is_finite() || !vel.angular.is_finite() {
            return Err(gizmo_physics_core::GizmoError::NaNVelocity(entity));
        }
        if rb.linear_damping.is_nan() || rb.angular_damping.is_nan() {
            return Err(gizmo_physics_core::GizmoError::NaNVelocity(entity));
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
            // Birikmiş tork dünya uzayında; bu yüzden dünya-uzayı ters atalet tensörü
            // (R·I_local⁻¹·Rᵀ) kullanılmalı — gövde-uzayı kestirmesi dönmüş/anizotropik
            // cisimlerde yanlış eksende açısal ivme üretiyordu.
            let inv_inertia = rb.inv_world_inertia_tensor(rotation);
            vel.angular += inv_inertia * rb.torque_accumulator * dt;
        }
        rb.clear_forces();

        // ── Aerodynamic drag: F = ½·ρ·Cd·A·|v|², opposing velocity ─────────
        // Opt-in via `RigidBody::with_air_drag` (Cd·A > 0). Unlike `linear_damping`
        // (a velocity-LINEAR exponential decay), this is the physical v² drag, so a
        // falling body settles at a natural terminal speed v_term = √(2·m·g/(ρ·Cd·A)).
        // Applied semi-implicitly with a per-step clamp so drag can never reverse the
        // velocity in one frame (Δv ≤ current speed) — unconditionally stable.
        if inv_mass > 0.0 && rb.drag_coefficient > 0.0 && rb.drag_area > 0.0 {
            // Sürükleme, HAVAYA GÖRE bağıl hıza karşıdır: v_rel = v - wind. `wind` sıfırken
            // (varsayılan) durağan hava = v'ye karşı sürükleme. Rüzgar varsa durağan bir
            // cisim bile rüzgar yönünde itilir (rüzgar tüneli / esinti).
            let v_rel = vel.linear - self.wind;
            let speed = v_rel.length();
            if speed > 1e-5 {
                let drag_mag =
                    0.5 * self.air_density * rb.drag_coefficient * rb.drag_area * speed * speed;
                let dv = (drag_mag * inv_mass * dt).min(speed);
                vel.linear -= (v_rel / speed) * dv;
            }
        }

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

        // ── Speed warning (blow-up / tunnelling guard) ────────────────────
        // Gated behind a >1000 m/s threshold, so it only fires on a body that is already
        // exploding — warn-level is appropriate (rare, and a symptom of a real problem).
        let speed_sq = vel.linear.length_squared();
        if speed_sq > 1_000_000.0 {
            tracing::warn!(
                entity = ?entity,
                speed = speed_sq.sqrt(),
                "Body moving dangerously fast (>1000 m/s) — tunnelling / blow-up risk"
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
        entity: BodyHandle,
        rb: &RigidBody,
        transform: &mut Transform,
        vel: &Velocity,
        dt: f32,
    ) -> Result<(), gizmo_physics_core::GizmoError> {
        if rb.is_static() || rb.is_sleeping {
            return Ok(());
        }

        // Apply axis locks to a local copy — do not mutate the stored velocity here.
        let mut masked = *vel;
        rb.enforce_locks(&mut masked);

        // ── Translation ───────────────────────────────────────────────────
        // Heun's Method (Trapezoidal Rule) for position:
        transform.position += (masked.pre_linear + masked.linear) * 0.5 * dt;

        if !transform.position.is_finite() {
            return Err(gizmo_physics_core::GizmoError::NaNPosition(entity));
        }

        // ── Rotation ──────────────────────────────────────────────────────
        // Only integrate when angular speed is non-negligible to avoid
        // normalising a near-zero quaternion. Heun's Method for rotation:
        let avg_angular = (masked.pre_angular + masked.angular) * 0.5;
        let ang_speed_sq = avg_angular.length_squared();
        if ang_speed_sq > 1e-8 {
            let delta_rot = Quat::from_scaled_axis(avg_angular * dt);
            transform.rotation = (delta_rot * transform.rotation).normalize();

            if !transform.rotation.is_finite() {
                return Err(gizmo_physics_core::GizmoError::NaNPosition(entity));
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
        entity: BodyHandle,
        rb: &mut RigidBody,
        transform: &mut Transform,
        vel: &mut Velocity,
        dt: f32,
    ) -> Result<(), gizmo_physics_core::GizmoError> {
        self.integrate_velocities(entity, rb, transform.rotation, vel, dt)?;
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
#[allow(clippy::field_reassign_with_default)] // testlerde Default sonrası alan atama okunabilirlik için
mod tests {
    use super::*;

    fn make_entity(id: u32) -> BodyHandle {
        BodyHandle::from_id(id)
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
            .integrate_velocities(entity, &mut rb, Quat::IDENTITY, &mut vel, 1.0)
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
    fn heuns_method_trapezoidal_position_integration() {
        let integrator = Integrator::default();
        let mut dynamic_rb = RigidBody::default();
        dynamic_rb.body_type = crate::components::BodyType::Dynamic;
        dynamic_rb.wake_up();

        let mut transform = Transform::new(Vec3::ZERO);
        let mut vel = Velocity::default();
        // pre_linear is 0.0, linear is 10.0 => average is 5.0
        vel.pre_linear = Vec3::ZERO;
        vel.linear = Vec3::new(10.0, 0.0, 0.0);
        
        // pre_angular is 0.0, angular is PI (around Y) => average is PI/2
        vel.pre_angular = Vec3::ZERO;
        vel.angular = Vec3::new(0.0, std::f32::consts::PI, 0.0);

        let entity = make_entity(99);

        integrator
            .integrate_positions(entity, &dynamic_rb, &mut transform, &vel, 1.0)
            .expect("position integration must succeed");

        // The position should advance by exactly the average: 5.0
        assert!(
            (transform.position.x - 5.0).abs() < 0.001,
            "position.x={} expected≈5.0 due to Heun's method average",
            transform.position.x
        );

        // Rotation should advance by PI/2 around Y axis
        // The Y axis points up, Quat::from_scaled_axis(0, PI/2, 0) gives a quaternion where:
        // w = cos(PI/4) ≈ 0.707, y = sin(PI/4) ≈ 0.707
        assert!(
            (transform.rotation.y - (std::f32::consts::FRAC_PI_4).sin()).abs() < 0.001,
            "rotation.y={} expected≈{}",
            transform.rotation.y,
            (std::f32::consts::FRAC_PI_4).sin()
        );
        assert!(
            (transform.rotation.w - (std::f32::consts::FRAC_PI_4).cos()).abs() < 0.001,
            "rotation.w={} expected≈{}",
            transform.rotation.w,
            (std::f32::consts::FRAC_PI_4).cos()
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
            .integrate_velocities(make_entity(2), &mut rb, Quat::IDENTITY, &mut vel, 1.0)
            .expect("integration must succeed");

        assert!(
            vel.linear.x < 10.0,
            "damping must reduce velocity; got {}",
            vel.linear.x
        );
        assert!(vel.linear.x > 0.0, "velocity must stay positive");
    }

    /// Aerodinamik hava direnci tek adımda `F = ½·ρ·Cd·A·v²`'yi hıza KARŞI uygular.
    /// Yerçekimi ve lineer sönüm kapalıyken (izole), hız kaybı analitik Δv = F/m·dt ile
    /// eşleşmeli.
    #[test]
    fn air_drag_matches_half_rho_cd_a_v_squared_one_step() {
        let integrator = Integrator::default(); // air_density = 1.225
        let (mass, cd, area, v0) = (2.0_f32, 1.0_f32, 0.5_f32, 20.0_f32);
        let mut rb = RigidBody::new(mass, false).with_air_drag(cd, area);
        rb.linear_damping = 0.0; // yalnız drag kalsın
        rb.wake_up();
        let mut vel = Velocity::new(Vec3::new(v0, 0.0, 0.0));
        let dt = 1.0 / 240.0;

        integrator
            .integrate_velocities(make_entity(2), &mut rb, Quat::IDENTITY, &mut vel, dt)
            .expect("integration must succeed");

        let f = 0.5 * integrator.air_density * cd * area * v0 * v0;
        let expected_v = v0 - f / mass * dt;
        assert!(
            (vel.linear.x - expected_v).abs() < expected_v * 1e-3,
            "one-step drag v={} must match ½ρCdAv²/m·dt → {expected_v}",
            vel.linear.x
        );
        assert!(vel.linear.x < v0, "drag must reduce the speed");
    }

    /// Doğal davranış: yerçekimi + hava direnci altında düşen cisim, serbest düşüşe
    /// gitmek yerine analitik TERMINAL hıza oturur: v_term = √(2·m·g / (ρ·Cd·A)).
    #[test]
    fn air_drag_gives_natural_terminal_velocity() {
        let integrator = Integrator::default(); // gravity -9.81, air 1.225
        let (mass, cd, area) = (1.0_f32, 0.47_f32, 0.1_f32); // ~küre
        let mut rb = RigidBody::new(mass, true).with_air_drag(cd, area);
        rb.linear_damping = 0.0; // yalnız yerçekimi + gerçek drag
        rb.wake_up();
        let mut vel = Velocity::default();
        let dt = 1.0 / 240.0;
        for _ in 0..4800 {
            // ~20 s
            integrator
                .integrate_velocities(make_entity(2), &mut rb, Quat::IDENTITY, &mut vel, dt)
                .expect("integration must succeed");
        }

        let g = integrator.gravity.length();
        let v_term = (2.0 * mass * g / (integrator.air_density * cd * area)).sqrt();
        let speed = vel.linear.length();
        assert!(
            (speed - v_term).abs() < v_term * 0.02,
            "terminal speed {speed} must settle at v_term = √(2mg/ρCdA) = {v_term} (±2%)"
        );
        // Guard: without drag, 20 s of free fall would be ~196 m/s; drag caps it.
        assert!(
            speed < g * 5.0,
            "drag must cap the fall far below free-fall speed, got {speed}"
        );
    }

    /// Rüzgar (Integrator.wind) durağan bir sürüklemeli cismi rüzgar yönünde iter ve hız
    /// rüzgar hızına asimptot yapar (drag bağıl hıza v−wind karşı → denge v = wind).
    #[test]
    fn wind_pushes_a_stationary_drag_body_toward_wind_speed() {
        let mut integrator = Integrator::default();
        integrator.wind = Vec3::new(20.0, 0.0, 0.0); // +X rüzgar
        integrator.gravity = Vec3::ZERO; // izole: yalnız rüzgar sürüklemesi
        let mut rb = RigidBody::new(1.0, false).with_air_drag(1.0, 1.0);
        rb.linear_damping = 0.0;
        rb.wake_up();
        let mut vel = Velocity::default(); // durağan başla
        let dt = 1.0 / 240.0;
        for _ in 0..2400 {
            // 10 s
            integrator
                .integrate_velocities(make_entity(2), &mut rb, Quat::IDENTITY, &mut vel, dt)
                .expect("integration must succeed");
        }
        assert!(
            vel.linear.x > 15.0,
            "rüzgar cismi downwind (+X) itmeli, vx={}",
            vel.linear.x
        );
        assert!(
            vel.linear.x <= 20.01,
            "hız rüzgar hızını (20) aşmamalı — asimptot v_wind, vx={}",
            vel.linear.x
        );
        assert!(
            vel.linear.y.abs() < 1e-3 && vel.linear.z.abs() < 1e-3,
            "hareket yalnız rüzgar ekseninde olmalı: {:?}",
            vel.linear
        );
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
            .integrate_velocities(make_entity(3), &mut rb, Quat::IDENTITY, &mut vel, 1.0)
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

        let result = integrator.integrate_velocities(make_entity(4), &mut rb, Quat::IDENTITY, &mut vel, 1.0);
        assert!(result.is_err(), "NaN velocity must return an error");
    }

    #[test]
    fn sleeping_body_is_not_integrated() {
        let integrator = Integrator::default();
        let mut rb = RigidBody::default();
        rb.body_type = crate::components::BodyType::Dynamic;
        // Explicitly put the body to sleep — the integrator must skip it.
        rb.is_sleeping = true;

        let mut vel = Velocity::default();
        integrator
            .integrate_velocities(make_entity(5), &mut rb, Quat::IDENTITY, &mut vel, 1.0)
            .expect("sleeping body integration must be a no-op");

        assert_eq!(
            vel.linear,
            Vec3::ZERO,
            "sleeping body must not gain velocity"
        );
    }
}
