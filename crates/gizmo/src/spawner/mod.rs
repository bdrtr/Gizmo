//! Command API — for spawning an object in a single line inside the setup closure.
//!
//! ```no_run
//! use gizmo::prelude::*;
//! # use gizmo::spawner::Commands;
//! App::<()>::new("demo", 1280, 720)
//!     .set_setup(|world, renderer| {
//!         let mut cmd = Commands::new(world, renderer);
//!         cmd.spawn_cube(Vec3::new(0.0, 0.0, -10.0), Color::RED).with_name("Player");
//!         cmd.spawn_camera(Vec3::new(0.0, 2.0, 5.0));
//!     })
//!     .run()
//!     .unwrap();
//! ```
//!
//! `no_run` because `App::run` opens a winit window and creates a wgpu surface: it cannot
//! execute in a doc-test, and would never return if it could.

use crate::color::Color;
use gizmo_core::{Entity, EntityName, World};
use gizmo_math::{Quat, Vec3};
use gizmo_physics_core::{Collider, Transform};
use gizmo_physics_rigid::components::{RigidBody, Velocity};
use gizmo_renderer::{
    asset::AssetManager,
    components::{Camera, DirectionalLight, Material, MeshRenderer, PointLight},
    Renderer,
};

// ─── Hata Tipleri ───────────────────────────────────────────────────────────

/// Errors that can arise during GLTF/GLB spawn operations.
///
/// 1.0 error contract: a concrete, `match`-able type instead of a stringly-typed error.
///
/// `#[non_exhaustive]`: as the loader in the lower layer (`gizmo-renderer`) moves to
/// a concrete error type, new variants (Io, Parse, GpuUpload, …) may be added here;
/// for that reason consumers must keep a `_ =>` arm.
#[derive(Debug)]
#[non_exhaustive]
pub enum GltfLoadError {
    /// The underlying loader (`AssetManager::load_gltf_scene` /
    /// `load_gltf_from_import`) returned an error. `path` is the file path that was
    /// being loaded; `source` is the lower layer's error description.
    ///
    /// Note: because the lower layer still returns `Result<_, String>`, the source
    /// is carried here as a `String`; when the lower layer moves to a concrete
    /// `Error` type this variant will be updated so that it chains with
    /// `#[source]`.
    Load {
        /// The file path that was being loaded.
        path: String,
        /// The error description coming from the lower layer.
        source: String,
    },
}

impl std::fmt::Display for GltfLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GltfLoadError::Load { path, source } => {
                write!(f, "GLTF '{}' yuklenemedi: {}", path, source)
            }
        }
    }
}

impl std::error::Error for GltfLoadError {}

// ─── Commands ─────────────────────────────────────────────────────────────────

/// A spawning front end: primitives with their meshes, materials and bodies in one call.
///
/// Each `spawn_*` returns an [`EntityBuilder`] so the entity can be adjusted before it is
/// finished — colour, physics, name.
pub struct Commands<'a> {
    /// The world entities are spawned into.
    pub world: &'a mut World,
    /// The live renderer, for creating the meshes and textures.
    pub renderer: &'a Renderer,
    /// The asset manager used for created assets; built on demand.
    pub asset_manager: Option<AssetManager>,
}

impl<'a> Drop for Commands<'a> {
    fn drop(&mut self) {
        if let Some(am) = self.asset_manager.take() {
            self.world.insert_resource(am);
        }
    }
}

impl<'a> Commands<'a> {
    /// A spawner over this world and renderer.
    pub fn new(world: &'a mut World, renderer: &'a Renderer) -> Self {
        let am = world.remove_resource::<AssetManager>().unwrap_or_default();
        Self {
            world,
            renderer,
            asset_manager: Some(am),
        }
    }

    // ── Primitifler ────────────────────────────────────────────────────────────

    /// Spawns a colored cube in a single line. `.with_name()` can be added via the builder chain.
    pub fn spawn_cube(&mut self, pos: Vec3, color: Color) -> EntityBuilder<'_, 'a> {
        let mesh = AssetManager::create_cube(&self.renderer.device);
        let bg = self.asset_manager.as_mut().unwrap().create_white_texture(
            &self.renderer.device,
            &self.renderer.queue,
            &self.renderer.scene.texture_bind_group_layout,
        );
        let mat = Material::new(bg).with_unlit(color.to_vec4());
        let id = spawn_mesh_entity(self.world, pos, mesh, mat);
        EntityBuilder {
            commands: self,
            entity: id,
        }
    }

    /// Spawns a colored sphere in a single line.
    pub fn spawn_sphere(&mut self, pos: Vec3, radius: f32, color: Color) -> EntityBuilder<'_, 'a> {
        let mesh = AssetManager::create_sphere(&self.renderer.device, radius, 20, 20);
        let bg = self.asset_manager.as_mut().unwrap().create_white_texture(
            &self.renderer.device,
            &self.renderer.queue,
            &self.renderer.scene.texture_bind_group_layout,
        );
        let mat = Material::new(bg).with_unlit(color.to_vec4());
        let id = spawn_mesh_entity(self.world, pos, mesh, mat);
        EntityBuilder {
            commands: self,
            entity: id,
        }
    }

    /// Spawns a planar ground in a single line.
    pub fn spawn_plane(&mut self, pos: Vec3, size: f32, color: Color) -> EntityBuilder<'_, 'a> {
        let mesh = AssetManager::create_plane(&self.renderer.device, size);
        let bg = self.asset_manager.as_mut().unwrap().create_white_texture(
            &self.renderer.device,
            &self.renderer.queue,
            &self.renderer.scene.texture_bind_group_layout,
        );
        let mat = Material::new(bg).with_unlit(color.to_vec4());
        let id = spawn_mesh_entity(self.world, pos, mesh, mat);
        EntityBuilder {
            commands: self,
            entity: id,
        }
    }

    /// Loads an .obj model from disk and spawns it.
    pub fn spawn_model(&mut self, pos: Vec3, path: &str) -> EntityBuilder<'_, 'a> {
        tracing::debug!(path, ?pos, "spawn_model: diskten .obj model yükleniyor");
        let mesh = self
            .asset_manager
            .as_mut()
            .unwrap()
            .load_obj(&self.renderer.device, path);
        let bg = self.asset_manager.as_mut().unwrap().create_white_texture(
            &self.renderer.device,
            &self.renderer.queue,
            &self.renderer.scene.texture_bind_group_layout,
        );
        let mat = Material::new(bg);
        let id = spawn_mesh_entity(self.world, pos, mesh, mat);
        EntityBuilder {
            commands: self,
            entity: id,
        }
    }

    // ── Kamera ────────────────────────────────────────────────────────────────

    /// Spawns the primary 3D perspective camera.
    /// `yaw = -π/2` (looking towards −X), `pitch = 0` (level).
    pub fn spawn_camera(&mut self, pos: Vec3) -> EntityBuilder<'_, 'a> {
        if let Some(mut cameras) = self.world.query_mut::<gizmo_core::prelude::Mut<Camera>>() {
            for (_, mut c) in cameras.iter_mut() {
                c.primary = false;
            }
        }
        let id = self.world.spawn();
        let trans = Transform::new(pos);

        self.world.add_component(id, trans);
        self.world.add_component(
            id,
            Camera {
                fov: 60.0_f32.to_radians(),
                near: 0.1,
                far: 1000.0,
                yaw: -std::f32::consts::FRAC_PI_2,
                pitch: 0.0,
                exposure: 1.0,
                primary: true,
                projection: Default::default(),
            },
        );
        EntityBuilder {
            commands: self,
            entity: id,
        }
    }

    /// Camera with customizable `fov` (degrees), `near`, `far`.
    pub fn spawn_camera_with(
        &mut self,
        pos: Vec3,
        fov_deg: f32,
        near: f32,
        far: f32,
    ) -> EntityBuilder<'_, 'a> {
        if let Some(mut cameras) = self.world.query_mut::<gizmo_core::prelude::Mut<Camera>>() {
            for (_, mut c) in cameras.iter_mut() {
                c.primary = false;
            }
        }
        let id = self.world.spawn();
        let trans = Transform::new(pos);

        self.world.add_component(id, trans);
        self.world.add_component(
            id,
            Camera {
                fov: fov_deg.to_radians(),
                near,
                far,
                yaw: -std::f32::consts::FRAC_PI_2,
                pitch: 0.0,
                exposure: 1.0,
                primary: true,
                projection: Default::default(),
            },
        );
        EntityBuilder {
            commands: self,
            entity: id,
        }
    }

    // ── Işıklar ─────────────────────────────────────────────────────────────────────────

    /// Spawns a point light.
    pub fn spawn_point_light(
        &mut self,
        pos: Vec3,
        color: Color,
        intensity: f32,
    ) -> EntityBuilder<'_, 'a> {
        let id = self.world.spawn();
        let trans = Transform::new(pos);

        self.world.add_component(id, trans);
        self.world.add_component(
            id,
            PointLight::new(
                gizmo_math::Vec3::new(color.0.x, color.0.y, color.0.z),
                intensity,
                10.0,
            ),
        );
        EntityBuilder {
            commands: self,
            entity: id,
        }
    }

    /// Spawns a directional light (sun/moon).
    /// `direction`: the normalized light direction (pointing downwards = negative Y).
    pub fn spawn_sun(
        &mut self,
        _direction: Vec3,
        color: Color,
        intensity: f32,
    ) -> EntityBuilder<'_, 'a> {
        let id = self.world.spawn();
        let pos = Vec3::ZERO; // DirectionalLight position is largely irrelevant
        let trans = Transform::new(pos);

        self.world.add_component(id, trans);
        self.world.add_component(
            id,
            DirectionalLight {
                color: Vec3::new(color.0.x, color.0.y, color.0.z),
                intensity,
                role: crate::renderer::components::LightRole::Sun,
            },
        );
        EntityBuilder {
            commands: self,
            entity: id,
        }
    }

    // ── Sahne Yardımcıları ────────────────────────────────────────────────────────────

    /// Spawns a skybox (a very large cube with inverted faces). The color determines the
    /// background color.
    pub fn spawn_skybox(&mut self, color: Color) -> EntityBuilder<'_, 'a> {
        // Skip existing check since is_skybox is removed

        // Wait, best approach for skybox is ignoring the duplication request if exists, but we must return an EntityBuilder...
        let mesh = AssetManager::create_inverted_cube(&self.renderer.device);
        let bg = self.asset_manager.as_mut().unwrap().create_white_texture(
            &self.renderer.device,
            &self.renderer.queue,
            &self.renderer.scene.texture_bind_group_layout,
        );
        let mat = Material::new(bg).with_unlit(color.to_vec4()).with_skybox();
        let id = self.world.spawn();
        let mut trans = Transform::new(Vec3::ZERO);
        trans.scale = Vec3::new(500.0, 500.0, 500.0);
        trans.update_local_matrix();

        self.world.add_component(id, trans);
        self.world.add_component(id, mesh);
        self.world.add_component(id, mat);
        self.world.add_component(id, MeshRenderer::new());
        EntityBuilder {
            commands: self,
            entity: id,
        }
    }

    // ── Fizik Spawn ─────────────────────────────────────────────────────────────────────────

    /// Spawns a dynamic cube that participates in the physics simulation.
    /// `half_extents`: Half size on each axis. `mass`: mass in kg (0 = static).
    pub fn spawn_rigid_cube(
        &mut self,
        pos: Vec3,
        half_extents: Vec3,
        color: Color,
        mass: f32,
    ) -> EntityBuilder<'_, 'a> {
        let mesh = AssetManager::create_cube(&self.renderer.device);
        let bg = self.asset_manager.as_mut().unwrap().create_white_texture(
            &self.renderer.device,
            &self.renderer.queue,
            &self.renderer.scene.texture_bind_group_layout,
        );
        let mat = Material::new(bg).with_unlit(color.to_vec4());
        let id = spawn_mesh_entity(self.world, pos, mesh, mat);
        // Scale'i half_extents ile eşleştir
        {
            let mut trans_store = self.world.borrow_mut::<Transform>();
            if let Some(mut trans) = trans_store.get_mut(id.id()) {
                trans.scale = half_extents * 2.0;
                trans.update_local_matrix();
            }
        }
        let mut rb = if mass > 0.0 {
            RigidBody::new(mass, true)
        } else {
            RigidBody::new_static()
        };
        let col = Collider::box_collider(half_extents);
        rb.update_inertia_from_collider(&col);
        self.world.add_component(id, rb);
        if mass > 0.0 {
            self.world.add_component(id, Velocity::new(Vec3::ZERO));
        }
        self.world.add_component(id, col);
        EntityBuilder {
            commands: self,
            entity: id,
        }
    }

    /// Spawns a dynamic sphere that participates in the physics simulation.
    pub fn spawn_rigid_sphere(
        &mut self,
        pos: Vec3,
        radius: f32,
        color: Color,
        mass: f32,
    ) -> EntityBuilder<'_, 'a> {
        let mesh = AssetManager::create_sphere(&self.renderer.device, radius, 16, 16);
        let bg = self.asset_manager.as_mut().unwrap().create_white_texture(
            &self.renderer.device,
            &self.renderer.queue,
            &self.renderer.scene.texture_bind_group_layout,
        );
        let mat = Material::new(bg).with_unlit(color.to_vec4());
        let id = spawn_mesh_entity(self.world, pos, mesh, mat);
        let mut rb = if mass > 0.0 {
            RigidBody::new(mass, true)
        } else {
            RigidBody::new_static()
        };
        let col = Collider::sphere(radius);
        rb.update_inertia_from_collider(&col);
        self.world.add_component(id, rb);
        if mass > 0.0 {
            self.world.add_component(id, Velocity::new(Vec3::ZERO));
        }
        self.world.add_component(id, col);
        EntityBuilder {
            commands: self,
            entity: id,
        }
    }

    /// Spawns a static (non-moving) ground plane.
    pub fn spawn_static_plane(
        &mut self,
        pos: Vec3,
        size: f32,
        color: Color,
    ) -> EntityBuilder<'_, 'a> {
        let mesh = AssetManager::create_plane(&self.renderer.device, size);
        let bg = self.asset_manager.as_mut().unwrap().create_white_texture(
            &self.renderer.device,
            &self.renderer.queue,
            &self.renderer.scene.texture_bind_group_layout,
        );
        let mat = Material::new(bg).with_pbr(color.to_vec4(), 0.9, 0.0);
        let id = spawn_mesh_entity(self.world, pos, mesh, mat);
        self.world.add_component(id, RigidBody::new_static());
        self.world.add_component(
            id,
            Collider::box_collider(Vec3::new(size / 2.0, 0.05, size / 2.0)),
        );
        EntityBuilder {
            commands: self,
            entity: id,
        }
    }

    // ── Görsel Yardımcılar ──────────────────────────────────────────────────────────────────────

    /// Loads a textured material and applies it to a cube.
    pub fn spawn_textured_cube(&mut self, pos: Vec3, texture_path: &str) -> EntityBuilder<'_, 'a> {
        let mesh = AssetManager::create_cube(&self.renderer.device);
        let bg = match self.asset_manager.as_mut().unwrap().load_material_texture(
            &self.renderer.device,
            &self.renderer.queue,
            &self.renderer.scene.texture_bind_group_layout,
            texture_path,
        ) {
            Ok(bg) => bg,
            Err(e) => {
                // Sessiz `.unwrap_or_else(|_| ...)` yutması yerine: hatayı BAĞLAMLA logla,
                // sonra görsel-bozulmayı önlemek için beyaz dokuya düş.
                tracing::warn!(
                    path = texture_path,
                    error = %e,
                    "spawn_textured_cube: doku yüklenemedi, beyaz dokuya düşülüyor"
                );
                self.asset_manager.as_mut().unwrap().create_white_texture(
                    &self.renderer.device,
                    &self.renderer.queue,
                    &self.renderer.scene.texture_bind_group_layout,
                )
            }
        };
        let mat = Material::new(bg);
        let id = spawn_mesh_entity(self.world, pos, mesh, mat);
        EntityBuilder {
            commands: self,
            entity: id,
        }
    }

    /// Loads a textured material and applies it to a plane.
    pub fn spawn_textured_plane(
        &mut self,
        pos: Vec3,
        size: f32,
        texture_path: &str,
    ) -> EntityBuilder<'_, 'a> {
        let mesh = AssetManager::create_plane(&self.renderer.device, size);
        let bg = match self.asset_manager.as_mut().unwrap().load_material_texture(
            &self.renderer.device,
            &self.renderer.queue,
            &self.renderer.scene.texture_bind_group_layout,
            texture_path,
        ) {
            Ok(bg) => bg,
            Err(e) => {
                // Sessiz `.unwrap_or_else(|_| ...)` yutması yerine: hatayı BAĞLAMLA logla,
                // sonra görsel-bozulmayı önlemek için beyaz dokuya düş.
                tracing::warn!(
                    path = texture_path,
                    error = %e,
                    "spawn_textured_plane: doku yüklenemedi, beyaz dokuya düşülüyor"
                );
                self.asset_manager.as_mut().unwrap().create_white_texture(
                    &self.renderer.device,
                    &self.renderer.queue,
                    &self.renderer.scene.texture_bind_group_layout,
                )
            }
        };
        let mat = Material::new(bg);
        let id = spawn_mesh_entity(self.world, pos, mesh, mat);
        EntityBuilder {
            commands: self,
            entity: id,
        }
    }

}

// ─── EntityBuilder — Zincir API ───────────────────────────────────────────────

/// Chain builder for adding extra components to a spawned entity.
pub struct EntityBuilder<'b, 'a> {
    commands: &'b mut Commands<'a>,
    entity: Entity,
}

impl<'b, 'a> EntityBuilder<'b, 'a> {
    /// Assign a name (tag) to the entity. It can be found inside update with
    /// `world.entity_named("...")`.
    pub fn with_name(self, name: &str) -> Self {
        self.commands
            .world
            .add_component(self.entity, EntityName(name.to_string()));
        self
    }

    /// Add any extra component.
    pub fn with<C: gizmo_core::Component + 'static>(self, component: C) -> Self {
        self.commands.world.add_component(self.entity, component);
        self
    }

    /// Consume and return the Entity ID.
    pub fn id(self) -> Entity {
        self.entity
    }
}

impl<'b, 'a> From<EntityBuilder<'b, 'a>> for Entity {
    fn from(b: EntityBuilder<'b, 'a>) -> Entity {
        b.entity
    }
}

// ─── Yardımcı: Mesh entity oluştur ────────────────────────────────────────────────────────────────

pub(super) fn spawn_mesh_entity(
    world: &mut World,
    pos: Vec3,
    mesh: gizmo_renderer::components::Mesh,
    mat: Material,
) -> Entity {
    let id = world.spawn();
    let mut trans = Transform::new(pos);
    trans.update_local_matrix();

    world.add_component(id, trans);
    world.add_component(id, gizmo_physics_core::components::GlobalTransform::default());
    let source = mesh.source.clone();
    world.add_component(id, mesh);
    world.add_component(id, mat);
    world.add_component(id, MeshRenderer::new());
    // Tüm primitif/rigid/textured/model mesh spawn'ları buradan geçer → tek noktada,
    // sıcak-yol-güvenli trace! (kapalıyken bedelsiz) ile spawn edilen mesh + konum kaydı.
    tracing::trace!(entity = id.id(), source = %source, ?pos, "mesh entity spawn'landı");
    id
}

// ─── WorldExt Trait — Update içinde kısa sorgular ─────────────────────────────

/// Convenience methods added on top of World.
/// Brought in automatically with `use gizmo::prelude::*;`.
pub trait WorldExt {
    /// Find the Entity ID (u32) by name.
    fn entity_named(&self, name: &str) -> Option<u32>;

    /// Modify an entity's Transform by name. The Transform matrix is updated automatically.
    fn move_entity_named<F: FnMut(&mut gizmo_physics_core::Transform)>(&mut self, name: &str, f: F);

    /// Get an entity's world position by name.
    fn position_of(&self, name: &str) -> Option<Vec3>;

    /// Modify any component by name.
    ///
    /// # Example
    /// ```
    /// use gizmo::prelude::*;
    /// # #[derive(Clone)] struct Health(u32);
    /// # gizmo::core::impl_component!(Health);
    /// # let mut world = World::new();
    /// # let e = world.spawn();
    /// # world.add_component(e, EntityName("Player".to_string()));
    /// # world.add_component(e, Health(100));
    /// // The second type parameter is the closure's own, so it can only be `_`.
    /// world.modify::<Health, _>("Player", |h| h.0 -= 30);
    /// assert_eq!(world.borrow::<Health>().get(e.id()).unwrap().0, 70);
    ///
    /// // An unknown name is a silent no-op, not a panic.
    /// world.modify::<Health, _>("Nobody", |h| h.0 = 0);
    /// assert_eq!(world.borrow::<Health>().get(e.id()).unwrap().0, 70);
    /// ```
    fn modify<T: gizmo_core::Component + 'static, F: FnMut(&mut T)>(&mut self, name: &str, f: F);
}

impl WorldExt for World {
    fn entity_named(&self, name: &str) -> Option<u32> {
        let mut names = self.query::<&EntityName>()?;
        for (id, n) in names.iter_mut() {
            if n.0 == name {
                return Some(id);
            }
        }
        None
    }

    fn move_entity_named<F: FnMut(&mut gizmo_physics_core::Transform)>(&mut self, name: &str, mut f: F) {
        let target: Option<u32> = {
            if let Some(mut names) = self.query::<&EntityName>() {
                let mut found = None;
                for (id, n) in names.iter_mut() {
                    if n.0 == name {
                        found = Some(id);
                        break;
                    }
                }
                found
            } else {
                None
            }
        };
        if let Some(target_id) = target {
            if let Some(mut transforms) =
                self.query_mut::<gizmo_core::prelude::Mut<gizmo_physics_core::Transform>>()
            {
                for (tid, mut trans) in transforms.iter_mut() {
                    if tid == target_id {
                        f(&mut trans);
                        trans.update_local_matrix();
                    }
                }
            }
        }
    }

    fn position_of(&self, name: &str) -> Option<Vec3> {
        let target_id = self.entity_named(name)?;
        let transforms = self.borrow::<gizmo_physics_core::Transform>();
        transforms.get(target_id).map(|t| t.position)
    }

    fn modify<T: gizmo_core::Component + 'static, F: FnMut(&mut T)>(
        &mut self,
        name: &str,
        mut f: F,
    ) {
        let target: Option<u32> = {
            if let Some(mut names) = self.query::<&EntityName>() {
                let mut found = None;
                for (id, n) in names.iter_mut() {
                    if n.0 == name {
                        found = Some(id);
                        break;
                    }
                }
                found
            } else {
                None
            }
        };
        if let Some(target_id) = target {
            {
                let mut storage = self.borrow_mut::<T>();
                if let Some(mut comp) = storage.get_mut(target_id) {
                    f(&mut *comp);
                }
            }
        }
    }
}

// ─── InputExt Trait — KeyCode doğrudan kabul eden kısaltmalar ─────────────────
// gizmo-core'da winit bağımlılığı olmadığı için bu trait gizmo crate'inde tanımlıdır.

/// Ergonomic methods added on top of `Input`.
/// Brought in automatically with `use gizmo::prelude::*;`.
///
/// # Example
/// ```
/// use gizmo::prelude::*;
/// # use gizmo::core::input::Input;
/// # use winit::keyboard::KeyCode as Key;
/// # let mut input = Input::new();
/// # input.on_key_pressed(Key::KeyW as u32);
/// # input.on_key_pressed(Key::Space as u32);
/// let mut z = 0.0_f32;
/// let dt = 1.0 / 60.0;
/// if input.pressed(Key::KeyW) { z -= 5.0 * dt; }
/// let jumped = input.just_pressed(Key::Space);
///
/// assert!(z < 0.0 && jumped);
/// // `just_pressed` is edge-triggered: it is false on the next frame, `pressed` is not.
/// # input.begin_frame();
/// assert!(input.pressed(Key::KeyW) && !input.just_pressed(Key::Space));
/// ```
pub trait InputExt {
    /// Is the key held down? Takes `KeyCode` variants like `Key::KeyW`, `Key::Space` directly.
    fn pressed(&self, keycode: winit::keyboard::KeyCode) -> bool;

    /// Was the key pressed for the first time this frame? (one-shot trigger)
    fn just_pressed(&self, keycode: winit::keyboard::KeyCode) -> bool;

    /// Was the key released this frame?
    fn just_released(&self, keycode: winit::keyboard::KeyCode) -> bool;
}

impl InputExt for gizmo_core::input::Input {
    #[inline]
    fn pressed(&self, keycode: winit::keyboard::KeyCode) -> bool {
        self.is_key_pressed(keycode as u32)
    }
    #[inline]
    fn just_pressed(&self, keycode: winit::keyboard::KeyCode) -> bool {
        self.is_key_just_pressed(keycode as u32)
    }
    #[inline]
    fn just_released(&self, keycode: winit::keyboard::KeyCode) -> bool {
        self.is_key_just_released(keycode as u32)
    }
}

// glTF/GLB spawning (the heaviest, most distinct concern) lives in `gltf`; it re-composes onto
// `Commands` as inherent methods. Public paths (spawner::Commands/WorldExt/…) are unchanged.
mod gltf;
