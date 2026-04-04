use yelbegen_math::Vec3;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    pub position: Vec3,
}

impl Transform {
    pub fn new(position: Vec3) -> Self {
        Self { position }
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
