use crate::color::Color;
/// Bevy benzeri Command API — setup closure içinde tek satırla nesne spawn etmek için.
///
/// # Örnek
/// ```rust,ignore
/// .set_setup(|world, renderer| {
///     let mut cmd = Commands::new(world, renderer);
///     cmd.spawn_cube(Vec3::new(0.0, 0.0, -10.0), Color::RED).with_name("Oyuncu");
///     cmd.spawn_camera(Vec3::new(0.0, 2.0, 5.0));
/// })
/// ```
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

/// GLTF/GLB spawn işlemlerinde oluşabilecek hatalar.
///
/// 1.0 hata kontratı: stringly-typed hata yerine somut, `match` edilebilir tip.
///
/// `#[non_exhaustive]`: alt katmandaki yükleyici (`gizmo-renderer`) somut hata
/// tipine geçtikçe burada yeni varyantlar (Io, Parse, GpuUpload, …) eklenebilir;
/// bu yüzden tüketiciler `_ =>` kolu bulundurmalıdır.
#[derive(Debug)]
#[non_exhaustive]
pub enum GltfLoadError {
    /// Alttaki yükleyici (`AssetManager::load_gltf_scene` /
    /// `load_gltf_from_import`) bir hata döndürdü. `path`, yüklenmeye çalışılan
    /// dosya yolu; `source`, alt katmanın hata açıklamasıdır.
    ///
    /// Not: alt katman henüz `Result<_, String>` döndürdüğü için kaynak burada
    /// bir `String` olarak taşınır; alt katman somut bir `Error` tipine
    /// geçtiğinde bu varyant `#[source]` ile zincirlenecek şekilde
    /// güncellenecektir.
    Load {
        /// Yüklenmeye çalışılan dosya yolu.
        path: String,
        /// Alt katmandan gelen hata açıklaması.
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

pub struct Commands<'a> {
    pub world: &'a mut World,
    pub renderer: &'a Renderer,
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
    pub fn new(world: &'a mut World, renderer: &'a Renderer) -> Self {
        let am = world.remove_resource::<AssetManager>().unwrap_or_default();
        Self {
            world,
            renderer,
            asset_manager: Some(am),
        }
    }

    // ── Primitifler ────────────────────────────────────────────────────────────

    /// Tek satırda renkli bir küp spawn eder. Builder zinciriyle `.with_name()` eklenebilir.
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

    /// Tek satırda renkli bir küre spawn eder.
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

    /// Tek satırda düzlemsel bir zemin spawn eder.
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

    /// Diskten bir .obj modeli yükler ve spawn eder.
    pub fn spawn_model(&mut self, pos: Vec3, path: &str) -> EntityBuilder<'_, 'a> {
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

    /// Birincil (primary) 3D perspektif kamera spawn eder.
    /// `yaw = -π/2` (−X'e bakıyor), `pitch = 0` (düz).
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

    /// `fov` (derece), `near`, `far` özelleştirilebilir kamera.
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

    /// Point light (nokta ışık) spawn eder.
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

    /// Directional light (güneş/ay) spawn eder.
    /// `direction`: normallenmiş ışık yönü (aşağı bakan = negatif Y).
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

    /// Skybox spawn eder (ters yüzlü çok büyük küp). Renk arka plan rengini belirler.
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

    /// Fizik simulasyonuna katılan dinamik bir küp spawn eder.
    /// `half_extents`: Her eksende yarı boyut. `mass`: kg cinsinden kütle (0 = statik).
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

    /// Fizik simulasyonuna katılan dinamik bir küre spawn eder.
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

    /// Statik (hareket etmeyen) zemin düzlemi spawn eder.
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

    /// Textureli bir materyal yükler ve bir küpe uygular.
    pub fn spawn_textured_cube(&mut self, pos: Vec3, texture_path: &str) -> EntityBuilder<'_, 'a> {
        let mesh = AssetManager::create_cube(&self.renderer.device);
        let bg = self
            .asset_manager
            .as_mut()
            .unwrap()
            .load_material_texture(
                &self.renderer.device,
                &self.renderer.queue,
                &self.renderer.scene.texture_bind_group_layout,
                texture_path,
            )
            .unwrap_or_else(|_| {
                self.asset_manager.as_mut().unwrap().create_white_texture(
                    &self.renderer.device,
                    &self.renderer.queue,
                    &self.renderer.scene.texture_bind_group_layout,
                )
            });
        let mat = Material::new(bg);
        let id = spawn_mesh_entity(self.world, pos, mesh, mat);
        EntityBuilder {
            commands: self,
            entity: id,
        }
    }

    /// Textureli bir materyal yükler ve bir düzleme uygular.
    pub fn spawn_textured_plane(
        &mut self,
        pos: Vec3,
        size: f32,
        texture_path: &str,
    ) -> EntityBuilder<'_, 'a> {
        let mesh = AssetManager::create_plane(&self.renderer.device, size);
        let bg = self
            .asset_manager
            .as_mut()
            .unwrap()
            .load_material_texture(
                &self.renderer.device,
                &self.renderer.queue,
                &self.renderer.scene.texture_bind_group_layout,
                texture_path,
            )
            .unwrap_or_else(|_| {
                self.asset_manager.as_mut().unwrap().create_white_texture(
                    &self.renderer.device,
                    &self.renderer.queue,
                    &self.renderer.scene.texture_bind_group_layout,
                )
            });
        let mat = Material::new(bg);
        let id = spawn_mesh_entity(self.world, pos, mesh, mat);
        EntityBuilder {
            commands: self,
            entity: id,
        }
    }

}

// ─── EntityBuilder — Zincir API ───────────────────────────────────────────────

/// Spawn edilen entity'e ek bileşenler eklemek için zincir builder.
pub struct EntityBuilder<'b, 'a> {
    commands: &'b mut Commands<'a>,
    entity: Entity,
}

impl<'b, 'a> EntityBuilder<'b, 'a> {
    /// Entity'e bir isim (tag) ata. Update içinde `world.entity_named("...")` ile bulunabilir.
    pub fn with_name(self, name: &str) -> Self {
        self.commands
            .world
            .add_component(self.entity, EntityName(name.to_string()));
        self
    }

    /// Herhangi bir ek bileşen ekle.
    pub fn with<C: gizmo_core::Component + 'static>(self, component: C) -> Self {
        self.commands.world.add_component(self.entity, component);
        self
    }

    /// Entity ID'sini tüket ve döndür.
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
    world.add_component(id, mesh);
    world.add_component(id, mat);
    world.add_component(id, MeshRenderer::new());
    id
}

/// GLTF hiyerarşisini düz (flat) olarak spawn eder — parent/child olmadan.
// ─── WorldExt Trait — Update içinde kısa sorgular ─────────────────────────────

/// World üzerine eklenen kolaylık metodları.
/// `use gizmo::prelude::*;` ile otomatik içeri alınır.
pub trait WorldExt {
    /// İsme göre Entity ID'sini (u32) bul.
    fn entity_named(&self, name: &str) -> Option<u32>;

    /// İsme göre entity'nin Transform'unu değiştir. Transform matrisi otomatik güncellenir.
    fn move_entity_named<F: FnMut(&mut gizmo_physics_core::Transform)>(&mut self, name: &str, f: F);

    /// İsme göre entity'nin dünya pozisyonunu al.
    fn position_of(&self, name: &str) -> Option<Vec3>;

    /// İsme göre herhangi bir bileşeni değiştir.
    ///
    /// # Örnek
    /// ```rust,ignore
    /// world.modify::<Camera>("Kamera", |cam| { cam.fov = 90.0_f32.to_radians(); });
    /// world.modify::<Material>("Top", |mat| { mat.albedo = Color::BLUE.to_vec4(); });
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

/// `Input` üzerine eklenen ergonomik metodlar.
/// `use gizmo::prelude::*;` ile otomatik içeri alınır.
///
/// # Örnek
/// ```rust,ignore
/// if input.pressed(Key::KeyW) { trans.position.z -= 5.0 * dt; }
/// if input.just_pressed(Key::Space) { player.jump(); }
/// ```
pub trait InputExt {
    /// Tuş basılı mı? `Key::KeyW`, `Key::Space` gibi `KeyCode` varyantlarını doğrudan alır.
    fn pressed(&self, keycode: winit::keyboard::KeyCode) -> bool;

    /// Tuş bu frame'de ilk kez mi basıldı? (tek seferlik tetikleme)
    fn just_pressed(&self, keycode: winit::keyboard::KeyCode) -> bool;

    /// Tuş bu frame'de mi bırakıldı?
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
