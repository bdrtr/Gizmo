//! # Bevy'nin `3d/parenting` örneğinin Gizmo Engine portu
//!
//! İki küp: biri ebeveyn, biri çocuk. Ebeveyn kendi X ekseni etrafında döner ve çocuk —
//! kendisi hiç döndürülmediği hâlde — onunla birlikte savrulur. Örneğin tek konusu bu:
//! **ebeveynin dönüşümü çocuğa yayılıyor mu.**
//!
//! Bu, deponun hiyerarşiyi gösteren İLK demosu: 39 demonun hiçbiri `Parent`/`Children`
//! kullanmıyordu, dolayısıyla `TransformPropagateSystem` de hiçbir vitrinde görünmüyordu —
//! oysa basit-sahne döngüsü onu zaten her frame çalıştırıyor. Bağ tek çağrıyla kuruluyor:
//! [`HierarchyExt::add_child`]. (Motorun kendi kuralı: bu çağrı döngü kuracak bir bağı
//! sessizce reddeder, yani çocuğu kendi torununun altına asamazsınız.)
//!
//! **Bevy'de dönüş yine elle yazılır** (`Rotator` bileşeni + `rotator_system`); Gizmo'da
//! [`Spin`] + [`SpinPlugin`] aynı işi bileşen olarak veriyor — burada X ekseni, 3,0 rad/s,
//! Bevy'deki `transform.rotate_x(3.0 * dt)` ile birebir.
//!
//! Sayısal parite notları (Bevy 0.19 kaynağına karşı):
//!   * Küpler `Cuboid::new(2, 2, 2)` — Gizmo'nun hazır küp mesh'i zaten 2 birim, ölçek yok.
//!   * Ebeveyn `(0, 0, 1)`'de, çocuk ebeveyne göre `(0, 0, 3)`'te (yani dünyada dönüşle
//!     birlikte savrulan bir yörünge çizer).
//!   * Malzeme `Color::srgb(0.8, 0.7, 0.6)`. Gizmo'nun `albedo`'su **doğrusal** uzayda
//!     olduğundan sRGB→doğrusal çevirisi burada elle yapıldı: `(0.603, 0.448, 0.318)`.
//!   * Nokta ışığı `(4, 5, -4)`'te, kamera `(5, 10, 10)`'dan orijine bakar.
//!
//! ## Kontroller
//!   * **Sağ-tık + fare** — bak · **WASDQE** — kamera · **Shift** — hızlı hareket

use gizmo::core::HierarchyExt;
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Bevy'nin `rotator_system`'i: X ekseninde 3,0 rad/s.
const ROTATOR_SPEED: f32 = 3.0;

/// `Color::srgb(0.8, 0.7, 0.6)`'nın doğrusal karşılığı — Gizmo albedo'yu doğrusal okur.
const CUBE_ALBEDO: Vec4 = Vec4::new(0.603_827_2, 0.447_988_8, 0.318_546_9, 1.0);

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Bevy Parenting", 1280, 720)
        .with_simple_scene(|scene, state| {
            // İki küp aynı mesh'i ve aynı malzemeyi paylaşır (Bevy'de de tek handle klonlanır).
            let cube_mesh = AssetManager::create_cube(&scene.renderer.device);
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let material = Material::new(white).with_pbr(CUBE_ALBEDO, 0.5, 0.0);

            // Ebeveyn küp — dönen olan.
            let parent = scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, 0.0, 1.0)),
                GlobalTransform::default(),
                cube_mesh.clone(),
                material.clone(),
                MeshRenderer::new(),
                Spin::new(Vec3::X, ROTATOR_SPEED),
            ));

            // Çocuk küp — konumu EBEVEYNE GÖRE; kendi `Spin`'i yok.
            let child = scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, 0.0, 3.0)),
                GlobalTransform::default(),
                cube_mesh,
                material,
                MeshRenderer::new(),
            ));

            // Bağ. Bundan sonrasını `TransformPropagateSystem` halleder.
            scene.world.add_child(parent, child);

            // Nokta ışığı ve kamera.
            scene.world.spawn_bundle(PointLightBundle {
                position: Vec3::new(4.0, 5.0, -4.0),
                intensity: 40.0,
                radius: 40.0,
                ..Default::default()
            });
            scene.spawn_camera(state, Vec3::new(5.0, 10.0, 10.0), Vec3::ZERO);
        })
        .add_plugin(SpinPlugin)
        .run()
        .expect("uygulama çalıştırılamadı");
}
