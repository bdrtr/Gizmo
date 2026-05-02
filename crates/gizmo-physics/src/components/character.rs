use gizmo_math::Vec3;
use serde::{Deserialize, Serialize};


#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CharacterController {
    pub speed: f32,
    pub jump_speed: f32,
    pub gravity: f32,
    pub max_slope_angle: f32, // in radians
    pub step_height: f32,
    pub is_grounded: bool,
    pub velocity: Vec3, // Internal velocity for jumping/falling
    pub target_velocity: Vec3, // Desired movement from input
    pub height: f32,
    pub radius: f32,
}

impl Default for CharacterController {
    fn default() -> Self {
        Self {
            speed: 5.0,
            jump_speed: 5.0,
            gravity: 9.81,
            max_slope_angle: 45.0_f32.to_radians(),
            step_height: 0.3,
            is_grounded: false,
            velocity: Vec3::ZERO,
            target_velocity: Vec3::ZERO,
            height: 2.0,
            radius: 0.5,
        }
    }
}


gizmo_core::impl_component!(CharacterController);
