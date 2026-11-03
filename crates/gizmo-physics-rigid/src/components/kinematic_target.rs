//! [`KinematicTarget`] — where a kinematic body should be at the end of the next step.

use gizmo_math::{Quat, Vec3};
use serde::{Deserialize, Serialize};
#[cfg(feature = "reflect")]
use bevy_reflect::Reflect;

/// The pose a [`BodyType::Kinematic`](crate::components::BodyType::Kinematic) body is driven to,
/// one physics step at a time.
///
/// A kinematic body moves by its [`Velocity`](crate::components::Velocity), and a game that knows
/// *where* it wants the body — a tilting board under the mouse, a door at an angle, a platform on
/// a path — had to turn that into a velocity itself, and with a fixed-step driver that is wrong:
/// a frame runs zero, one or several steps, so a velocity computed from the frame's `dt` turns
/// the body by the wrong amount. With this component the physics step does it: before each step
/// the body's velocity is set to what reaches the target **in that step** (so a ball on the board
/// is carried and pushed by the motion), and after the step the body is put on the target
/// exactly, so nothing drifts. Rotation takes the short way round.
///
/// The target stays until it is changed or removed: a body that has arrived holds there with a
/// zero velocity. On a body that is not kinematic it does nothing.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "reflect", derive(Reflect))]
pub struct KinematicTarget {
    /// Where the body's `Transform::position` should be, world space.
    pub position: Vec3,
    /// Where its `Transform::rotation` should be.
    pub rotation: Quat,
}

impl KinematicTarget {
    /// A target pose.
    #[must_use]
    pub fn new(position: Vec3, rotation: Quat) -> Self {
        Self { position, rotation }
    }

    /// The velocity — linear, angular — that takes a body from `position`/`rotation` to this
    /// target in `dt`, read by the solver about the body's centre of mass, which sits at
    /// `com_offset` (world space, from `position`).
    #[must_use]
    pub fn velocity_from(&self, position: Vec3, rotation: Quat, com_offset: Vec3, dt: f32) -> (Vec3, Vec3) {
        if dt <= 0.0 {
            return (Vec3::ZERO, Vec3::ZERO);
        }
        let mut delta = self.rotation * rotation.inverse();
        if delta.w < 0.0 {
            delta = -delta;
        }
        let (axis, angle) = delta.to_axis_angle();
        let angular = if angle.is_finite() && angle > 1e-7 { axis * (angle / dt) } else { Vec3::ZERO };
        // The pose turns about `position`; the solver reads `linear` as the centre of mass's
        // velocity, which a turn about another point moves by `ω × offset`.
        let linear = (self.position - position) / dt + angular.cross(com_offset);
        (linear, angular)
    }
}

gizmo_core::impl_component!(KinematicTarget);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_velocity_reaches_the_target_in_one_step_the_short_way() {
        let t = KinematicTarget::new(Vec3::new(1.0, 0.0, 0.0), Quat::from_rotation_z(0.2));
        let (lin, ang) = t.velocity_from(Vec3::ZERO, Quat::IDENTITY, Vec3::ZERO, 0.5);
        assert!((lin - Vec3::new(2.0, 0.0, 0.0)).length() < 1e-5);
        assert!((ang - Vec3::new(0.0, 0.0, 0.4)).length() < 1e-5);
        // 350° the long way is 10° the short way.
        let t = KinematicTarget::new(Vec3::ZERO, Quat::from_rotation_y(-10f32.to_radians()));
        let (_, ang) = t.velocity_from(Vec3::ZERO, Quat::from_rotation_y(0.0), Vec3::ZERO, 1.0);
        assert!((ang.y + 10f32.to_radians()).abs() < 1e-4, "{ang:?}");
        // A centre of mass off the pivot moves with the turn.
        let t = KinematicTarget::new(Vec3::ZERO, Quat::from_rotation_z(0.1));
        let (lin, _) = t.velocity_from(Vec3::ZERO, Quat::IDENTITY, Vec3::new(1.0, 0.0, 0.0), 1.0);
        assert!((lin - Vec3::new(0.0, 0.1, 0.0)).length() < 1e-4, "{lin:?}");
        assert_eq!(t.velocity_from(Vec3::ZERO, Quat::IDENTITY, Vec3::ZERO, 0.0), (Vec3::ZERO, Vec3::ZERO));
    }
}
