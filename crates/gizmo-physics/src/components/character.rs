use gizmo_math::Vec3;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CharacterController {
    pub speed: f32,
    pub jump_speed: f32,
    pub gravity: f32,
    pub max_slope_angle: f32, // in radians
    pub slope_slide_speed: f32,
    pub step_height: f32,
    
    #[serde(skip)]
    pub is_grounded: bool,
    
    pub target_velocity: Vec3, // Desired movement from input
    
    pub coyote_time: f32,
    #[serde(skip)]
    pub coyote_timer: f32,
    
    pub jump_buffer_time: f32,
    #[serde(skip)]
    pub jump_buffer_timer: f32,
}

impl Default for CharacterController {
    fn default() -> Self {
        Self {
            speed: 5.0,
            jump_speed: 5.0,
            gravity: 9.81,
            max_slope_angle: 45.0_f32.to_radians(),
            slope_slide_speed: 10.0,
            step_height: 0.3,
            is_grounded: false,
            target_velocity: Vec3::ZERO,
            coyote_time: 0.1,
            coyote_timer: 0.0,
            jump_buffer_time: 0.1,
            jump_buffer_timer: 0.0,
        }
    }
}

gizmo_core::impl_component!(CharacterController);
