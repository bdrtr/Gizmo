//! General-purpose VISUAL ROTATION component + system.
//!
//! Rotates a mesh (the ECS `Transform`) about a fixed axis at a fixed angular velocity.
//! Wheel, propeller, fan, turbine, planet, rotating platform... a single solution for all of
//! them — the demos are NO LONGER REQUIRED to write `transform.rotation = ...` by hand every
//! frame. Add the component, run [`SpinPlugin`] (or [`SpinSystem`] directly), let the engine
//! do the rotating.
//!
//! ```
//! use gizmo::prelude::*;
//! use gizmo::core::system::System;
//! use gizmo::systems::spin::SpinSystem;
//!
//! let mut world = World::new();
//! let wheel = world.spawn();
//! world.add_component(wheel, Transform::new(Vec3::ZERO));
//! // 30 rad/s, about the X axis.
//! world.add_component(wheel, Spin::new(Vec3::X, 30.0));
//!
//! // In an application `app.add_plugin(SpinPlugin)` runs this every frame;
//! // here we drive a single step by hand.
//! SpinSystem.run(&world, 0.1);
//!
//! let rotated = world.query::<&Transform>().unwrap().get(wheel.id()).unwrap().rotation;
//! assert!(rotated.angle_between(Quat::IDENTITY) > 0.0, "Spin must have applied a rotation");
//! ```

use gizmo_core::world::World;
use gizmo_math::{Quat, Vec3};
use gizmo_physics_core::Transform;

/// Rotates a `Transform` about the `axis` axis at `angular_velocity` (rad/s).
/// The rotation rides ON TOP OF `rest_rotation` (the model's authored pose).
/// `angular_velocity` can be changed every frame (e.g. tie wheel speed to vehicle speed).
#[derive(Debug, Clone, Copy)]
pub struct Spin {
    /// Rotation axis (body-local), gets normalized.
    pub axis: Vec3,
    /// Angular velocity, rad/s. Can be changed at runtime.
    pub angular_velocity: f32,
    /// The model's (authored) rotation before spinning — the rotation is applied on top of it.
    pub rest_rotation: Quat,
    /// Accumulated angle (rad) — managed by the system.
    pub angle: f32,
}

impl Spin {
    /// Component that rotates about the `axis` axis at `angular_velocity` (rad/s).
    pub fn new(axis: Vec3, angular_velocity: f32) -> Self {
        let axis = if axis.length_squared() > 1e-9 {
            axis.normalize()
        } else {
            Vec3::X
        };
        Self {
            axis,
            angular_velocity,
            rest_rotation: Quat::IDENTITY,
            angle: 0.0,
        }
    }

    /// Preserve the model's authored-pose rotation (essential on pre-rotated meshes such as a
    /// GLTF wheel — otherwise the pose is broken). Chainable.
    pub fn with_rest_rotation(mut self, rest: Quat) -> Self {
        self.rest_rotation = rest;
        self
    }
}

gizmo_core::impl_component!(Spin);

/// Advances every [`Spin`] each frame and applies it to `Transform.rotation`. [`SpinPlugin`]
/// adds this to the schedule; `SpinSystem.run(world, dt)` can also be called by hand.
pub struct SpinSystem;

impl gizmo_core::system::System for SpinSystem {
    fn access_info(&self) -> gizmo_core::system::AccessInfo {
        let mut info = gizmo_core::system::AccessInfo::new();
        info.is_exclusive = true; // Spin + Transform'a mutable erişir
        info
    }

    fn run(&mut self, world: &World, dt: f32) {
        // SAFETY: exclusive sistem; Spin ve Transform ayrı bileşen tipleri (disjoint),
        // scheduler bu çalışırken başka mutable alias vermez.
        if let Some(mut q) = unsafe {
            world.query_unchecked::<(
                gizmo_core::query::Mut<Spin>,
                gizmo_core::query::Mut<Transform>,
            )>()
        } {
            for (_id, (mut spin, mut t)) in q.iter_mut() {
                spin.angle += spin.angular_velocity * dt;
                t.rotation = spin.rest_rotation * Quat::from_axis_angle(spin.axis, spin.angle);
                t.update_local_matrix();
            }
        }
    }
}

/// Adds [`SpinSystem`] to the application's schedule → every mesh with a [`Spin`] component
/// rotates automatically.
pub struct SpinPlugin;

impl<State: 'static> crate::app::Plugin<State> for SpinPlugin {
    fn build(&self, app: &mut crate::app::App<State>) {
        app.schedule.add_di_system(
            gizmo_core::system::SystemConfig::new(Box::new(SpinSystem)).label("spin"),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_core::system::System;
    use gizmo_core::world::World;

    #[test]
    fn spin_system_rotates_transform_over_rest() {
        let mut world = World::new();
        let e = world.spawn();
        let rest = Quat::from_rotation_y(0.5);
        world.add_component(e, Transform::new(Vec3::ZERO));
        // 2 rad/s, X ekseni, yazar-duruş korunur.
        world.add_component(e, Spin::new(Vec3::X, 2.0).with_rest_rotation(rest));

        let mut sys = SpinSystem;
        // 1 s topla (dt=1/60 × 60).
        for _ in 0..60 {
            sys.run(&world, 1.0 / 60.0);
        }

        let t = world.borrow::<Transform>();
        let rot = t.get(e.id()).unwrap().rotation;
        // 1 s'de ~2 rad dönmüş olmalı, rest'in üzerine.
        let expected = rest * Quat::from_axis_angle(Vec3::X, 2.0);
        assert!(
            rot.dot(expected).abs() > 0.9999,
            "Spin rest'in üzerine ~2 rad döndürmeli"
        );
        // Spin bileşeninin biriken açısı da ~2.
        let spins = world.borrow::<Spin>();
        assert!((spins.get(e.id()).unwrap().angle - 2.0).abs() < 1e-3);
    }
}
