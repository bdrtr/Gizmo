use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiAgent {
    pub target: u32,
    pub speed: f32,
    pub max_force: f32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CarController {
    pub speed: f32,
    pub steering: f32, // direksiyon açısı (radyan)
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Wheel {
    pub is_front: bool,
    pub is_left: bool,
    pub base_rotation: f32, // tekerleğin teker gibi dönme açısı
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Raindrop;
