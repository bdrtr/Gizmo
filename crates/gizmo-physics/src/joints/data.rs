use gizmo_core::Entity;
use gizmo_math::{Quat, Vec3};
use serde::{Deserialize, Serialize};


#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum JointType {
    Fixed,
    Hinge,
    BallSocket,
    Slider,
    Spring,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Joint {
    pub entity_a: Entity,
    pub entity_b: Entity,
    pub joint_type: JointType,
    pub local_anchor_a: Vec3,
    pub local_anchor_b: Vec3,
    pub break_force: f32,
    pub break_torque: f32,
    pub is_broken: bool,
    pub collision_enabled: bool,
    pub data: JointData,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum JointData {
    Fixed,
    Hinge(HingeJointData),
    BallSocket(BallSocketJointData),
    Slider(SliderJointData),
    Spring(SpringJointData),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct HingeJointData {
    pub axis: Vec3,
    pub use_limits: bool,
    pub lower_limit: f32,
    pub upper_limit: f32,
    pub use_motor: bool,
    pub motor_target_velocity: f32,
    pub motor_max_force: f32,
    pub current_angle: f32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct BallSocketJointData {
    pub use_cone_limit: bool,
    pub cone_limit_angle: f32,
    #[serde(default)]
    pub initial_relative_rotation: Option<Quat>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SliderJointData {
    pub axis: Vec3,
    pub use_limits: bool,
    pub lower_limit: f32,
    pub upper_limit: f32,
    pub use_motor: bool,
    pub motor_target_velocity: f32,
    pub motor_max_force: f32,
    pub current_position: f32,
    #[serde(default)]
    pub initial_relative_rotation: Option<Quat>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SpringJointData {
    pub rest_length: f32,
    pub stiffness: f32,
    pub damping: f32,
    pub min_length: f32,
    pub max_length: f32,
}

impl Joint {
    pub fn fixed(entity_a: Entity, entity_b: Entity, local_anchor_a: Vec3, local_anchor_b: Vec3) -> Self {
        Self {
            entity_a, entity_b,
            joint_type: JointType::Fixed,
            local_anchor_a, local_anchor_b,
            break_force: 0.0, break_torque: 0.0,
            is_broken: false, collision_enabled: false,
            data: JointData::Fixed,
        }
    }

    pub fn hinge(entity_a: Entity, entity_b: Entity, local_anchor_a: Vec3, local_anchor_b: Vec3, axis: Vec3) -> Self {
        Self {
            entity_a, entity_b,
            joint_type: JointType::Hinge,
            local_anchor_a, local_anchor_b,
            break_force: 0.0, break_torque: 0.0,
            is_broken: false, collision_enabled: false,
            data: JointData::Hinge(HingeJointData {
                axis: axis.normalize(),
                use_limits: false,
                lower_limit: -std::f32::consts::PI,
                upper_limit: std::f32::consts::PI,
                use_motor: false,
                motor_target_velocity: 0.0,
                motor_max_force: 0.0,
                current_angle: 0.0,
            }),
        }
    }

    pub fn ball_socket(entity_a: Entity, entity_b: Entity, local_anchor_a: Vec3, local_anchor_b: Vec3) -> Self {
        Self {
            entity_a, entity_b,
            joint_type: JointType::BallSocket,
            local_anchor_a, local_anchor_b,
            break_force: 0.0, break_torque: 0.0,
            is_broken: false, collision_enabled: false,
            data: JointData::BallSocket(BallSocketJointData {
                use_cone_limit: false,
                cone_limit_angle: std::f32::consts::PI,
                initial_relative_rotation: None,
            }),
        }
    }

    pub fn slider(entity_a: Entity, entity_b: Entity, local_anchor_a: Vec3, local_anchor_b: Vec3, axis: Vec3) -> Self {
        Self {
            entity_a, entity_b,
            joint_type: JointType::Slider,
            local_anchor_a, local_anchor_b,
            break_force: 0.0, break_torque: 0.0,
            is_broken: false, collision_enabled: false,
            data: JointData::Slider(SliderJointData {
                axis: axis.normalize(),
                use_limits: false,
                lower_limit: -10.0, upper_limit: 10.0,
                use_motor: false,
                motor_target_velocity: 0.0, motor_max_force: 0.0,
                current_position: 0.0,
                initial_relative_rotation: None,
            }),
        }
    }

    pub fn spring(entity_a: Entity, entity_b: Entity, local_anchor_a: Vec3, local_anchor_b: Vec3, rest_length: f32, stiffness: f32, damping: f32) -> Self {
        Self {
            entity_a, entity_b,
            joint_type: JointType::Spring,
            local_anchor_a, local_anchor_b,
            break_force: 0.0, break_torque: 0.0,
            is_broken: false, collision_enabled: false,
            data: JointData::Spring(SpringJointData {
                rest_length, stiffness, damping,
                min_length: 0.0, max_length: f32::INFINITY,
            }),
        }
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
        if self.break_force > 0.0 && applied_force > self.break_force {
            self.is_broken = true;
            return true;
        }
        if self.break_torque > 0.0 && applied_torque > self.break_torque {
            self.is_broken = true;
            return true;
        }
        false
    }
}

