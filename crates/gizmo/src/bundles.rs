//! Bevy tarzı önceden tanımlanmış Bundle yapıları.
//!
//! Bir entity'ye birden fazla bileşeni tek seferde eklemek için kullanılır.
//!
//! ```ignore
//! world.spawn_bundle(CameraBundle {
//!     position: Vec3::new(0.0, 3.0, 10.0),
//!     fov: 60.0_f32.to_radians(),
//!     ..default()
//! });
//! ```

use crate::core::{Bundle, Entity, EntityName, World};
use crate::math::{Quat, Vec3};
use crate::physics::Transform;
use crate::renderer::components::{
    Camera, DirectionalLight, LightRole, Material, Mesh, MeshRenderer, PointLight, SpotLight,
};

// ============================================================
//  DirectionalLightBundle
// ============================================================

/// Yönlü ışık (güneş) için hazır bundle.
pub struct DirectionalLightBundle {
    pub rotation: Quat,
    pub color: Vec3,
    pub intensity: f32,
    pub role: LightRole,
}

impl Default for DirectionalLightBundle {
    fn default() -> Self {
        Self {
            rotation: Quat::from_rotation_x(-std::f32::consts::PI / 4.0),
            color: Vec3::new(1.0, 1.0, 1.0),
            intensity: 3.0,
            role: LightRole::Sun,
        }
    }
}

impl Bundle for DirectionalLightBundle {
    fn apply(self, world: &mut World, entity: Entity) {
        world.add_component(entity, Transform::new(Vec3::ZERO).with_rotation(self.rotation));
        world.add_component(entity, crate::physics::GlobalTransform::default());
        world.add_component(entity, DirectionalLight::new(self.color, self.intensity, self.role));
    }
}

// ============================================================
//  PointLightBundle
// ============================================================

/// Nokta ışığı için hazır bundle.
pub struct PointLightBundle {
    pub position: Vec3,
    pub color: Vec3,
    pub intensity: f32,
    pub radius: f32,
}

impl Default for PointLightBundle {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            color: Vec3::new(1.0, 1.0, 1.0),
            intensity: 5.0,
            radius: 20.0,
        }
    }
}

impl Bundle for PointLightBundle {
    fn apply(self, world: &mut World, entity: Entity) {
        world.add_component(entity, Transform::new(self.position));
        world.add_component(entity, crate::physics::GlobalTransform::default());
        world.add_component(entity, PointLight::new(self.color, self.intensity, self.radius));
    }
}

// ============================================================
//  SpotLightBundle
// ============================================================

/// Spot ışığı için hazır bundle.
pub struct SpotLightBundle {
    pub position: Vec3,
    pub rotation: Quat,
    pub color: Vec3,
    pub intensity: f32,
    pub radius: f32,
    pub inner_angle: f32,
    pub outer_angle: f32,
}

impl Default for SpotLightBundle {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            color: Vec3::new(1.0, 1.0, 1.0),
            intensity: 10.0,
            radius: 30.0,
            inner_angle: 0.4,
            outer_angle: 0.6,
        }
    }
}

impl Bundle for SpotLightBundle {
    fn apply(self, world: &mut World, entity: Entity) {
        world.add_component(
            entity,
            Transform::new(self.position).with_rotation(self.rotation),
        );
        world.add_component(entity, crate::physics::GlobalTransform::default());
        world.add_component(
            entity,
            SpotLight::new(self.color, self.intensity, self.radius, self.inner_angle, self.outer_angle),
        );
    }
}

// ============================================================
//  CameraBundle
// ============================================================

/// Kamera için hazır bundle.
pub struct CameraBundle {
    pub position: Vec3,
    pub fov: f32,
    pub near: f32,
    pub far: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub primary: bool,
}

impl Default for CameraBundle {
    fn default() -> Self {
        Self {
            position: Vec3::new(0.0, 5.0, 10.0),
            fov: std::f32::consts::FRAC_PI_3,
            near: 0.1,
            far: 1500.0,
            yaw: 0.0,
            pitch: 0.0,
            primary: true,
        }
    }
}

impl Bundle for CameraBundle {
    fn apply(self, world: &mut World, entity: Entity) {
        world.add_component(entity, Transform::new(self.position));
        world.add_component(entity, crate::physics::GlobalTransform::default());
        world.add_component(
            entity,
            Camera::new(self.fov, self.near, self.far, self.yaw, self.pitch, self.primary),
        );
    }
}

// ============================================================
//  MeshBundle
// ============================================================

/// Mesh + Material + MeshRenderer için hazır bundle.
///
/// ```ignore
/// world.spawn_bundle(
///     MeshBundle::new(renderer.create_cube(), my_material)
///         .with_name("Oyuncu")
///         .at(Vec3::new(0.0, 5.0, 0.0))
/// );
/// ```
pub struct MeshBundle {
    pub position: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
    pub mesh: crate::core::asset::Handle<Mesh>,
    pub material: crate::core::asset::Handle<Material>,
    pub name: Option<String>,
}

impl MeshBundle {
    /// Yeni bir MeshBundle oluşturur (mesh ve material zorunlu).
    pub fn new(mesh: crate::core::asset::Handle<Mesh>, material: crate::core::asset::Handle<Material>) -> Self {
        Self {
            position: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
            mesh,
            material,
            name: None,
        }
    }

    /// Pozisyon ayarlar.
    pub fn at(mut self, position: Vec3) -> Self {
        self.position = position;
        self
    }

    /// Rotasyon ayarlar.
    pub fn with_rotation(mut self, rotation: Quat) -> Self {
        self.rotation = rotation;
        self
    }

    /// Ölçek ayarlar.
    pub fn with_scale(mut self, scale: Vec3) -> Self {
        self.scale = scale;
        self
    }

    /// İsim verir.
    pub fn with_name(mut self, name: &str) -> Self {
        self.name = Some(name.to_string());
        self
    }
}

impl Bundle for MeshBundle {
    fn apply(self, world: &mut World, entity: Entity) {
        world.add_component(
            entity,
            Transform::new(self.position)
                .with_rotation(self.rotation)
                .with_scale(self.scale),
        );
        world.add_component(entity, crate::physics::GlobalTransform::default());
        world.add_component(entity, self.mesh);
        world.add_component(entity, self.material);
        world.add_component(entity, MeshRenderer::new());
        if let Some(name) = self.name {
            world.add_component(entity, EntityName(name));
        }
    }
}
