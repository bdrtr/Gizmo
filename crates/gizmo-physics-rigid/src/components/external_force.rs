//! [`ExternalForce`] — a force that keeps pushing, every physics step, until it is changed.

use gizmo_math::Vec3;
use serde::{Deserialize, Serialize};
#[cfg(feature = "reflect")]
use bevy_reflect::Reflect;

/// A force and torque applied to a body on **every** physics step while this component is on it —
/// a boat's engine, a thruster, a steady wind, a conveyor's pull.
///
/// The body's own accumulators ([`RigidBody::force_accumulator`]) are emptied by every step, and a
/// game writes them once a frame: with a fixed-step driver a frame runs zero, one or several
/// steps, so the same push became a different acceleration at every frame rate. Measured
/// (2026-11-04, `FixedPhysics` at 60 Hz, 1 kg, 10 N for one simulated second): written with `=`
/// every frame, 10.0 m/s at 60 fps and 5.0 at 30 (the second step of each frame got nothing);
/// written with `+=`, 10.0 at 60 fps and 24.0 at 144 (frames without a step piled up). With this
/// component it is 10.0 at all three.
///
/// World space, and about the centre of mass: [`at_point`](Self::at_point) turns a force at a
/// point into the force and torque that do the same.
///
/// [`RigidBody::force_accumulator`]: crate::components::RigidBody::force_accumulator
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "reflect", derive(Reflect))]
pub struct ExternalForce {
    /// Newtons, world space, through the centre of mass.
    pub force: Vec3,
    /// Newton-metres, world space.
    pub torque: Vec3,
}

impl ExternalForce {
    /// A force through the centre of mass.
    #[must_use]
    pub fn new(force: Vec3) -> Self {
        Self { force, torque: Vec3::ZERO }
    }

    /// `force` applied at `point`, for a body whose centre of mass is at `center_of_mass` (both
    /// world space): the same force plus the torque its lever arm gives.
    #[must_use]
    pub fn at_point(force: Vec3, point: Vec3, center_of_mass: Vec3) -> Self {
        Self { force, torque: (point - center_of_mass).cross(force) }
    }

    /// The same push with `torque` added.
    #[must_use]
    pub fn with_torque(mut self, torque: Vec3) -> Self {
        self.torque += torque;
        self
    }
}

gizmo_core::impl_component!(ExternalForce);
