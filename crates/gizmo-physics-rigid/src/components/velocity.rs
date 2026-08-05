use gizmo_math::Vec3;
use serde::{Deserialize, Serialize};
#[cfg(feature = "reflect")]
use bevy_reflect::Reflect;

/// Motion state of a body: its current linear and angular velocity, plus a previous-step
/// copy of both that position integration reads.
///
/// The `pre_*` pair is engine bookkeeping, not user input. The pipeline maintains it — it is
/// snapshotted at the top of each step, before gravity and accumulated forces are applied,
/// and refreshed again from the solved velocities at the end of the constraint solve, which
/// is the last write before position integration runs. Callers therefore do not maintain the
/// pair themselves, and a velocity written from game code between steps is picked up either
/// way.
///
/// It is not optional: the ECS bridge only hands a body to the simulation when it carries
/// this alongside a [`RigidBody`](super::RigidBody), a `Transform` and a `Collider`. Writing
/// to it on a static or sleeping body has no effect, though — position integration returns
/// early for both.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "reflect", derive(Reflect))]
pub struct Velocity {
    /// Velocity in metres per second, in world space.
    ///
    /// The solver and [`Integrator::apply_impulse_at_point`](crate::Integrator::apply_impulse_at_point)
    /// treat this as the velocity of the body's **centre of mass** — they measure lever arms
    /// from `Transform::position + rotation · RigidBody::center_of_mass`, and the velocity of
    /// any other point `p` is `linear + angular × (p − that centre)`. Position integration,
    /// however, adds `linear` straight to `Transform::position`. The two readings agree only
    /// while the centre-of-mass offset is zero or the body is not spinning.
    pub linear: Vec3,
    /// Angular velocity in radians per second about a **world**-space axis, in scaled-axis
    /// form: the direction is the axis and the length is the rate. These are not Euler
    /// rates — reading `.y` as "yaw per second" is only valid for a body spinning purely
    /// about world Y.
    ///
    /// Integration rotates the transform by `Quat::from_scaled_axis(ω · dt)` applied on the
    /// left, and skips the rotation entirely when the angular speed for the step is under
    /// about `1e-4` rad/s, rather than normalising a near-identity quaternion.
    pub angular: Vec3,
    /// Önceki karenin hızı (Diferansiyel hesap / Heun's method için)
    #[serde(skip)]
    #[cfg_attr(feature = "reflect", reflect(ignore))]
    pub pre_linear: Vec3,
    /// Angular counterpart of [`pre_linear`](Self::pre_linear), in the same units and frame
    /// as [`angular`](Self::angular).
    ///
    /// Skipped by both serde and reflection, so a deserialized or snapshot-restored body
    /// resumes with this at zero instead of its saved value. That does not cost
    /// reproducibility: the same snapshot always restores the same zeros, and the pipeline
    /// overwrites the pair during the step anyway.
    #[serde(skip)]
    #[cfg_attr(feature = "reflect", reflect(ignore))]
    pub pre_angular: Vec3,
}

impl Velocity {
    /// Body moving at `linear` (world-space m/s) with no spin.
    ///
    /// Seeds [`pre_linear`](Self::pre_linear) with the same value rather than zero, keeping
    /// the pair self-consistent; the angular pair starts at `Vec3::ZERO`.
    pub fn new(linear: Vec3) -> Self {
        Self {
            linear,
            angular: Vec3::ZERO,
            pre_linear: linear,
            pre_angular: Vec3::ZERO,
        }
    }

    /// Sets the world-space angular velocity (rad/s, scaled-axis), keeping
    /// [`pre_angular`](Self::pre_angular) in step with it just as [`new`](Self::new) seeds
    /// `pre_linear`.
    ///
    /// It overwrites rather than accumulates, so chaining it twice keeps only the last
    /// value, and it does not touch [`linear`](Self::linear).
    pub fn with_angular(mut self, angular: Vec3) -> Self {
        self.angular = angular;
        self.pre_angular = angular;
        self
    }
}

impl Default for Velocity {
    fn default() -> Self {
        Self::new(Vec3::ZERO)
    }
}

gizmo_core::impl_component!(Velocity);
