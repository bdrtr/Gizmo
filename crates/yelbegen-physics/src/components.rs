use yelbegen_math::{Vec3, Quat, Mat4};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    pub position: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl Transform {
    pub fn new(position: Vec3) -> Self {
        Self {
            position,
            rotation: Quat::IDENTITY,
            scale: Vec3::new(1.0, 1.0, 1.0),
        }
    }

    pub fn with_scale(mut self, scale: Vec3) -> Self {
        self.scale = scale;
        self
    }

    pub fn with_rotation(mut self, rotation: Quat) -> Self {
        self.rotation = rotation;
        self
    }

    /// Model matrisi: Translation * Rotation * Scale
    pub fn model_matrix(&self) -> Mat4 {
        Mat4::translation(self.position) * self.rotation.to_mat4() * Mat4::scale(self.scale)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Velocity {
    pub linear: Vec3,
}

impl Velocity {
    pub fn new(linear: Vec3) -> Self {
        Self { linear }
    }
}

// Fiziksel ağırlık ve dış güçlerin nasıl etki edeceğini belirten kütle özellikleri
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RigidBody {
    pub mass: f32, // Eğer mass == 0.0 ise bu obje sabittir (Duvar/Zemin) ve itilemez!
    pub restitution: f32, // Sekme katsayısı (0.0 = sekmez, 1.0 = sonsuz teper)
    pub friction: f32, // Sürtünme katsayısı
    pub use_gravity: bool, // Yerçekiminden etkileniyor mu?
}

impl RigidBody {
    pub fn new(mass: f32, restitution: f32, friction: f32, use_gravity: bool) -> Self {
        Self { mass, restitution, friction, use_gravity }
    }

    pub fn new_static() -> Self {
        Self { mass: 0.0, restitution: 0.0, friction: 1.0, use_gravity: false }
    }
}
