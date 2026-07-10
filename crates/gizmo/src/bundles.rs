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
use gizmo_physics_core::Transform;
use crate::renderer::components::{
    Camera, DirectionalLight, LightRole, Material, Mesh, MeshRenderer, PointLight, SpotLight,
};

// ============================================================
//  DirectionalLightBundle
// ============================================================

/// Yönlü ışık (güneş) için hazır bundle.
#[derive(Debug, Clone, Copy, PartialEq)]
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
    fn get_infos() -> Vec<gizmo_core::archetype::ComponentInfo> { vec![] }
    unsafe fn write_to_archetype(self, _arch: &mut gizmo_core::archetype::Archetype, _row: usize, _tick: u32) {}
    fn apply(self, world: &mut World, entity: Entity) {
        world.add_component(
            entity,
            Transform::new(Vec3::ZERO).with_rotation(self.rotation),
        );
        world.add_component(entity, gizmo_physics_core::components::GlobalTransform::default());
        world.add_component(
            entity,
            DirectionalLight::new(self.color, self.intensity, self.role),
        );
    }
}

// ============================================================
//  PointLightBundle
// ============================================================

/// Nokta ışığı için hazır bundle.
#[derive(Debug, Clone, Copy, PartialEq)]
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
    fn get_infos() -> Vec<gizmo_core::archetype::ComponentInfo> { vec![] }
    unsafe fn write_to_archetype(self, _arch: &mut gizmo_core::archetype::Archetype, _row: usize, _tick: u32) {}
    fn apply(self, world: &mut World, entity: Entity) {
        world.add_component(entity, Transform::new(self.position));
        world.add_component(entity, gizmo_physics_core::components::GlobalTransform::default());
        world.add_component(
            entity,
            PointLight::new(self.color, self.intensity, self.radius),
        );
    }
}

// ============================================================
//  SpotLightBundle
// ============================================================

/// Spot ışığı için hazır bundle.
#[derive(Debug, Clone, Copy, PartialEq)]
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
    fn get_infos() -> Vec<gizmo_core::archetype::ComponentInfo> { vec![] }
    unsafe fn write_to_archetype(self, _arch: &mut gizmo_core::archetype::Archetype, _row: usize, _tick: u32) {}
    fn apply(self, world: &mut World, entity: Entity) {
        world.add_component(
            entity,
            Transform::new(self.position).with_rotation(self.rotation),
        );
        world.add_component(entity, gizmo_physics_core::components::GlobalTransform::default());
        world.add_component(
            entity,
            SpotLight::new(
                self.color,
                self.intensity,
                self.radius,
                self.inner_angle,
                self.outer_angle,
            ),
        );
    }
}

// ============================================================
//  CameraBundle
// ============================================================

/// Kamera için hazır bundle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraBundle {
    pub position: Vec3,
    pub fov: f32,
    pub near: f32,
    pub far: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub exposure: f32,
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
            exposure: 1.0,
            primary: true,
        }
    }
}

impl Bundle for CameraBundle {
    fn get_infos() -> Vec<gizmo_core::archetype::ComponentInfo> { vec![] }
    unsafe fn write_to_archetype(self, _arch: &mut gizmo_core::archetype::Archetype, _row: usize, _tick: u32) {}
    fn apply(self, world: &mut World, entity: Entity) {
        world.add_component(entity, Transform::new(self.position));
        world.add_component(entity, gizmo_physics_core::components::GlobalTransform::default());
        let mut cam = Camera::new(
            self.fov,
            self.near,
            self.far,
            self.yaw,
            self.pitch,
            self.primary,
        );
        cam.exposure = self.exposure;
        world.add_component(entity, cam);
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
    pub fn new(
        mesh: crate::core::asset::Handle<Mesh>,
        material: crate::core::asset::Handle<Material>,
    ) -> Self {
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
    fn get_infos() -> Vec<gizmo_core::archetype::ComponentInfo> { vec![] }
    unsafe fn write_to_archetype(self, _arch: &mut gizmo_core::archetype::Archetype, _row: usize, _tick: u32) {}
    fn apply(self, world: &mut World, entity: Entity) {
        world.add_component(
            entity,
            Transform::new(self.position)
                .with_rotation(self.rotation)
                .with_scale(self.scale),
        );
        world.add_component(entity, gizmo_physics_core::components::GlobalTransform::default());
        world.add_component(entity, self.mesh);
        world.add_component(entity, self.material);
        world.add_component(entity, MeshRenderer::new());
        if let Some(name) = self.name {
            world.add_component(entity, EntityName(name));
        }
    }
}

// ============================================================
//  RigidBodyBundle
// ============================================================

use gizmo_physics_core::Collider;
use gizmo_physics_rigid::components::{RigidBody, Velocity};

/// Fizik nesnesi oluşturmak için sıfır-yük (zero-overhead) Bundle.
/// Velocity veya Collider eklemeyi unutma hatalarını önler.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RigidBodyBundle {
    pub rigid_body: RigidBody,
    pub velocity: Velocity,
    pub collider: Collider,
}


impl RigidBodyBundle {
    pub fn dynamic(mass: f32) -> Self {
        Self {
            rigid_body: RigidBody::new(mass, true),
            ..Default::default()
        }
    }

    pub fn static_body() -> Self {
        Self {
            rigid_body: RigidBody::new_static(),
            ..Default::default()
        }
    }

    /// Kinematic body — user-driven motion (moving platforms, scripted blades).
    /// `new_kinematic` turns CCD on by default, so fast kinematic movers get
    /// tunnelling prevention without a separate `.with_ccd()`.
    pub fn kinematic() -> Self {
        Self {
            rigid_body: RigidBody::new_kinematic(),
            ..Default::default()
        }
    }

    pub fn with_collider(mut self, collider: Collider) -> Self {
        self.collider = collider;
        self
    }

    /// Give the body an initial linear velocity (the bundle otherwise spawns at rest).
    pub fn with_velocity(mut self, linear: Vec3) -> Self {
        self.velocity = Velocity::new(linear);
        self
    }

    /// Enable Continuous Collision Detection: the body is swept against obstacles
    /// each substep so it can't tunnel through thin/other geometry at high speed.
    /// Off by default (discrete detection) — turn it on for bullets, fast balls,
    /// anything that moves more than its own thickness per frame.
    pub fn with_ccd(mut self) -> Self {
        self.rigid_body.ccd_enabled = true;
        self
    }
}

impl Bundle for RigidBodyBundle {
    fn get_infos() -> Vec<gizmo_core::archetype::ComponentInfo> {
        <(RigidBody, Velocity, Collider)>::get_infos()
    }

    unsafe fn write_to_archetype(self, arch: &mut gizmo_core::archetype::Archetype, row: usize, tick: u32) {
        let mut rb = self.rigid_body;
        rb.update_inertia_from_collider(&self.collider);
        (rb, self.velocity, self.collider).write_to_archetype(arch, row, tick)
    }

    fn apply(self, world: &mut World, entity: Entity) {
        let mut rb = self.rigid_body;
        // Derive rotational inertia from the collider shape so callers don't have to
        // remember `calculate_*_inertia` — the default inertia otherwise gives wrong
        // spin dynamics. No-op for static/kinematic bodies (the calculators guard on
        // `is_dynamic`), and idempotent if the caller already set a matching inertia.
        rb.update_inertia_from_collider(&self.collider);
        (rb, self.velocity, self.collider).apply(world, entity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rigid_body_bundle_derives_inertia_from_collider() {
        // Spawn purely via the bundle — no manual calculate_*_inertia.
        let mut world = World::new();
        let e = world
            .spawn_bundle(RigidBodyBundle::dynamic(2.0).with_collider(Collider::sphere(0.5)));

        let rbs = world.borrow::<RigidBody>();
        let rb = rbs.get(e.id()).expect("rigid body spawned");

        // Solid sphere: I = 0.4·m·r² = 0.4·2·0.25 = 0.2 per axis (calculate_sphere_inertia).
        assert!(
            (rb.local_inertia - Vec3::splat(0.2)).length() < 1e-6,
            "bundle must derive sphere inertia from the collider, got {:?}",
            rb.local_inertia
        );
        // Regression: must NOT be the un-derived default Vec3::splat(1.0).
        assert!(
            (rb.local_inertia - Vec3::splat(1.0)).length() > 1e-3,
            "inertia must be derived, not left at the default"
        );
    }

    #[test]
    fn rigid_body_bundle_static_body_inertia_derivation_is_noop() {
        // Static bodies must spawn fine; inertia derivation is a no-op for them.
        let mut world = World::new();
        let e = world
            .spawn_bundle(RigidBodyBundle::static_body().with_collider(Collider::sphere(0.5)));
        assert!(world.borrow::<RigidBody>().get(e.id()).is_some());
    }
}
