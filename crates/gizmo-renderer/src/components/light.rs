use gizmo_math::Vec3;

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct PointLight {
    pub color: Vec3,
    pub intensity: f32,
    pub radius: f32,
}

impl PointLight {
    pub fn new(color: Vec3, intensity: f32) -> Self {
        Self { color, intensity, radius: 10.0 }
    }
}

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct DirectionalLight {
    pub color: Vec3,
    pub intensity: f32,
    pub is_sun: bool,
}

impl DirectionalLight {
    pub fn new(color: Vec3, intensity: f32, is_sun: bool) -> Self {
        Self { color, intensity, is_sun }
    }
}

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct SpotLight {
    pub color: Vec3,
    pub intensity: f32,
    pub radius: f32,
    pub inner_angle: f32,
    pub outer_angle: f32,
}

impl SpotLight {
    pub fn new(color: Vec3, intensity: f32, radius: f32, inner_angle: f32, outer_angle: f32) -> Self {
        Self { color, intensity, radius, inner_angle, outer_angle }
    }
}
