use gizmo_physics_core::BodyHandle;
use gizmo_math::{Quat, Vec3};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Joint {
    pub entity_a: BodyHandle,
    pub entity_b: BodyHandle,
    pub local_anchor_a: Vec3,
    pub local_anchor_b: Vec3,
    pub break_force: f32,
    pub break_torque: f32,
    #[serde(skip)]
    pub is_broken: bool,
    pub collision_enabled: bool,
    pub data: JointData,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub enum JointData {
    Fixed,
    Hinge(HingeJointData),
    BallSocket(BallSocketJointData),
    Slider(SliderJointData),
    Spring(SpringJointData),
    Distance(DistanceJointData),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum JointType {
    Fixed,
    Hinge,
    BallSocket,
    Slider,
    Spring,
    Distance,
}

/// Compile-forced mapping so `JointType` (the authoring descriptor) and `JointData`
/// (the runtime payload) can never silently drift: adding a `JointData` variant without
/// a matching `JointType` is a compile error here. Used by the solver dispatch.
impl From<&JointData> for JointType {
    fn from(data: &JointData) -> Self {
        match data {
            JointData::Fixed => JointType::Fixed,
            JointData::Hinge(_) => JointType::Hinge,
            JointData::BallSocket(_) => JointType::BallSocket,
            JointData::Slider(_) => JointType::Slider,
            JointData::Spring(_) => JointType::Spring,
            JointData::Distance(_) => JointType::Distance,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[non_exhaustive]
pub struct HingeJointData {
    pub axis: Vec3,
    pub use_limits: bool,
    pub lower_limit: f32,
    pub upper_limit: f32,
    pub use_motor: bool,
    pub motor_target_velocity: f32,
    pub motor_max_force: f32,
    /// When true (and `use_motor`), the motor is a POSITION SERVO: it drives toward
    /// `motor_target_position` (target angle, rad) instead of holding a target velocity,
    /// force-limited by `motor_max_force`. When false it is the classic velocity motor.
    pub motor_is_servo: bool,
    pub motor_target_position: f32,
    #[serde(skip)]
    pub current_angle: f32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[non_exhaustive]
pub struct BallSocketJointData {
    pub use_cone_limit: bool,
    pub cone_limit_angle: f32,
    #[serde(default)]
    pub initial_relative_rotation: Option<Quat>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[non_exhaustive]
pub struct SliderJointData {
    pub axis: Vec3,
    pub use_limits: bool,
    pub lower_limit: f32,
    pub upper_limit: f32,
    pub use_motor: bool,
    pub motor_target_velocity: f32,
    pub motor_max_force: f32,
    /// When true (and `use_motor`), the motor is a POSITION SERVO driving toward
    /// `motor_target_position` (target offset along the axis) instead of a target velocity.
    pub motor_is_servo: bool,
    pub motor_target_position: f32,
    #[serde(skip)]
    pub current_position: f32,
    #[serde(default)]
    pub initial_relative_rotation: Option<Quat>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[non_exhaustive]
pub struct SpringJointData {
    pub rest_length: f32,
    pub stiffness: f32,
    pub damping: f32,
    pub min_length: f32,
    pub max_length: Option<f32>,
}

/// Distance/rope joint: keeps the anchor separation within `[min_length, max_length]`
/// as a HARD (inequality) constraint — unlike `Spring`, which is a soft force toward a
/// rest length. A **rope** is `{min: 0, max: L}`: it only pulls when taut (`len > L`)
/// and is limp when slack (`len < L`), so a released slack body free-falls until the
/// rope catches it — no rigid-rod snap. A **rigid rod** is `{min: L, max: L}`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[non_exhaustive]
pub struct DistanceJointData {
    pub min_length: f32,
    pub max_length: f32,
}

impl Joint {
    pub fn joint_type(&self) -> &'static str {
        match &self.data {
            JointData::Fixed => "Fixed",
            JointData::Hinge(_) => "Hinge",
            JointData::BallSocket(_) => "BallSocket",
            JointData::Slider(_) => "Slider",
            JointData::Spring(_) => "Spring",
            JointData::Distance(_) => "Distance",
        }
    }

    pub fn fixed(
        entity_a: BodyHandle,
        entity_b: BodyHandle,
        local_anchor_a: Vec3,
        local_anchor_b: Vec3,
    ) -> Self {
        debug_assert_ne!(
            entity_a, entity_b,
            "Joint: entity_a and entity_b must be different"
        );
        Self {
            entity_a,
            entity_b,
            local_anchor_a,
            local_anchor_b,
            break_force: f32::INFINITY,
            break_torque: f32::INFINITY,
            is_broken: false,
            collision_enabled: false,
            data: JointData::Fixed,
        }
    }

    pub fn hinge(
        entity_a: BodyHandle,
        entity_b: BodyHandle,
        local_anchor_a: Vec3,
        local_anchor_b: Vec3,
        axis: Vec3,
    ) -> Self {
        debug_assert_ne!(
            entity_a, entity_b,
            "Joint: entity_a and entity_b must be different"
        );
        let safe_axis = if axis.length_squared() > 1e-6 {
            axis.normalize()
        } else {
            Vec3::Y
        };
        Self {
            entity_a,
            entity_b,
            local_anchor_a,
            local_anchor_b,
            break_force: f32::INFINITY,
            break_torque: f32::INFINITY,
            is_broken: false,
            collision_enabled: false,
            data: JointData::Hinge(HingeJointData {
                axis: safe_axis,
                use_limits: false,
                lower_limit: -std::f32::consts::PI,
                upper_limit: std::f32::consts::PI,
                use_motor: false,
                motor_target_velocity: 0.0,
                motor_max_force: 0.0,
                motor_is_servo: false,
                motor_target_position: 0.0,
                current_angle: 0.0,
            }),
        }
    }

    pub fn ball_socket(
        entity_a: BodyHandle,
        entity_b: BodyHandle,
        local_anchor_a: Vec3,
        local_anchor_b: Vec3,
    ) -> Self {
        debug_assert_ne!(
            entity_a, entity_b,
            "Joint: entity_a and entity_b must be different"
        );
        Self {
            entity_a,
            entity_b,
            local_anchor_a,
            local_anchor_b,
            break_force: f32::INFINITY,
            break_torque: f32::INFINITY,
            is_broken: false,
            collision_enabled: false,
            data: JointData::BallSocket(BallSocketJointData {
                use_cone_limit: false,
                cone_limit_angle: std::f32::consts::PI,
                initial_relative_rotation: None,
            }),
        }
    }

    pub fn slider(
        entity_a: BodyHandle,
        entity_b: BodyHandle,
        local_anchor_a: Vec3,
        local_anchor_b: Vec3,
        axis: Vec3,
    ) -> Self {
        debug_assert_ne!(
            entity_a, entity_b,
            "Joint: entity_a and entity_b must be different"
        );
        let safe_axis = if axis.length_squared() > 1e-6 {
            axis.normalize()
        } else {
            Vec3::Y
        };
        Self {
            entity_a,
            entity_b,
            local_anchor_a,
            local_anchor_b,
            break_force: f32::INFINITY,
            break_torque: f32::INFINITY,
            is_broken: false,
            collision_enabled: false,
            data: JointData::Slider(SliderJointData {
                axis: safe_axis,
                use_limits: false,
                lower_limit: -10.0,
                upper_limit: 10.0,
                use_motor: false,
                motor_target_velocity: 0.0,
                motor_max_force: 0.0,
                motor_is_servo: false,
                motor_target_position: 0.0,
                current_position: 0.0,
                initial_relative_rotation: None,
            }),
        }
    }

    pub fn spring(
        entity_a: BodyHandle,
        entity_b: BodyHandle,
        local_anchor_a: Vec3,
        local_anchor_b: Vec3,
        rest_length: f32,
        stiffness: f32,
        damping: f32,
    ) -> Self {
        debug_assert_ne!(
            entity_a, entity_b,
            "Joint: entity_a and entity_b must be different"
        );
        Self {
            entity_a,
            entity_b,
            local_anchor_a,
            local_anchor_b,
            break_force: f32::INFINITY,
            break_torque: f32::INFINITY,
            is_broken: false,
            collision_enabled: false,
            data: JointData::Spring(SpringJointData {
                rest_length,
                stiffness,
                damping,
                min_length: 0.0,
                max_length: None,
            }),
        }
    }

    /// Distance joint: constrains the anchor separation to `[min_length, max_length]`
    /// as a hard inequality. `min == max` ⇒ rigid rod; `min == 0` ⇒ rope (see [`Self::rope`]).
    pub fn distance(
        entity_a: BodyHandle,
        entity_b: BodyHandle,
        local_anchor_a: Vec3,
        local_anchor_b: Vec3,
        min_length: f32,
        max_length: f32,
    ) -> Self {
        debug_assert_ne!(
            entity_a, entity_b,
            "Joint: entity_a and entity_b must be different"
        );
        Self {
            entity_a,
            entity_b,
            local_anchor_a,
            local_anchor_b,
            break_force: f32::INFINITY,
            break_torque: f32::INFINITY,
            is_broken: false,
            collision_enabled: false,
            data: JointData::Distance(DistanceJointData {
                min_length: min_length.max(0.0),
                max_length: max_length.max(min_length.max(0.0)),
            }),
        }
    }

    /// Rope: inextensible but can go slack. The anchors cannot separate beyond `length`
    /// (pulls when taut), but may come closer (limp when slack) — a released slack body
    /// free-falls until the rope catches, with no rigid-rod snap. Shorthand for
    /// `distance(.., 0.0, length)`.
    pub fn rope(
        entity_a: BodyHandle,
        entity_b: BodyHandle,
        local_anchor_a: Vec3,
        local_anchor_b: Vec3,
        length: f32,
    ) -> Self {
        Self::distance(entity_a, entity_b, local_anchor_a, local_anchor_b, 0.0, length)
    }

    pub fn with_break_force(mut self, force: f32, torque: f32) -> Self {
        self.break_force = force;
        self.break_torque = torque;
        self
    }

    pub fn with_collision(mut self, enabled: bool) -> Self {
        self.collision_enabled = enabled;
        self
    }

    pub fn check_break(&mut self, applied_force: f32, applied_torque: f32) -> bool {
        if applied_force > self.break_force {
            self.is_broken = true;
            return true;
        }
        if applied_torque > self.break_torque {
            self.is_broken = true;
            return true;
        }
        false
    }
}
