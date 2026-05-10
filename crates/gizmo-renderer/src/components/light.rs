use gizmo_math::Vec3;

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct PointLight {
    pub color: Vec3,
    pub intensity: f32,
    pub radius: f32,
}

impl PointLight {
    pub fn new(color: Vec3, intensity: f32, radius: f32) -> Self {
        let intensity = intensity.max(0.0);
        let radius = radius.max(0.001);
        Self {
            color,
            intensity,
            radius,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LightRole {
    Sun,
    Generic,
}

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct DirectionalLight {
    pub color: Vec3,
    pub intensity: f32,
    pub role: LightRole,
}

impl DirectionalLight {
    pub fn new(color: Vec3, intensity: f32, role: LightRole) -> Self {
        let intensity = intensity.max(0.0);
        Self {
            color,
            intensity,
            role,
        }
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
    pub fn new(
        color: Vec3,
        intensity: f32,
        radius: f32,
        inner_angle: f32,
        outer_angle: f32,
    ) -> Self {
        let intensity = intensity.max(0.0);
        let radius = radius.max(0.001);
        let inner_angle = inner_angle.min(outer_angle);
        Self {
            color,
            intensity,
            radius,
            inner_angle,
            outer_angle,
        }
    }
}
