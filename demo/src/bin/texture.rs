//! # Doku ve albedo modülasyonu
//!
//! Aynı doku, üç farklı malzeme ayarıyla: **olduğu gibi**, **kırmızıya modüle edilmiş** ve
//! **maviye modüle edilmiş** — ikincisi ve üçüncüsü ayrıca yarı saydam. Üçü de `unlit`, yani
//! ışıktan etkilenmiyor; ekranda görünen şey dokunun kendisi ile malzemenin albedo'sunun
//! çarpımı.
//!
//! Motorun kuralı `Material::albedo`'nun kendi belgesinde yazılı: *"Base colour tint (linear),
//! multiplied into the albedo texture"*. Yani albedo bir renk değil, bir **çarpan**; beyaz
//! (1,1,1,1) dokuyu olduğu gibi bırakır, `(1,0,0,·)` yalnız kırmızı kanalı geçirir. Demo bunu üç
//! dörtgende yan yana koyuyor.
//!
//! **Doku diskten geliyor**: `assets/brick.jpg`, motorun kendi
//! [`AssetManager::load_material_texture`]'ı ile. Yükleme yolu çalışma dizinine göre çözülüyor —
//! demo depo kökünden çalıştırılmalı (`cargo run` zaten öyle yapar).
//!
//! ## Kontroller
//!   * **Sağ-tık + fare** — bak · **WASDQE** — kamera

use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};
use std::f32::consts::PI;

/// Dörtgenin genişliği ve en-boy oranı.
const QUAD_WIDTH: f32 = 8.0;
const QUAD_ASPECT: f32 = 0.25;
/// Dokunun yolu — deponun kendi varlığı.
const TEXTURE: &str = "assets/brick.jpg";

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Texture", 1280, 720)
        .with_simple_scene(|scene, state| {
            // Dokuyu diskten yükle; bulunamazsa beyaza düş (demo yine çalışsın, ama sessizce
            // yanlış görünmesin — hangi yolun denendiği günlüğe düşüyor).
            let texture = match scene.asset_manager.load_material_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
                TEXTURE,
            ) {
                Ok(bind_group) => bind_group,
                Err(error) => {
                    gizmo::gizmo_log!(
                        Warning,
                        "{TEXTURE} yüklenemedi ({error}); beyaz dokuya düşülüyor"
                    );
                    scene.asset_manager.create_white_texture(
                        &scene.renderer.device,
                        &scene.renderer.queue,
                        &scene.renderer.scene.texture_bind_group_layout,
                    )
                }
            };

            let quad = AssetManager::create_sprite_quad(
                &scene.renderer.device,
                QUAD_WIDTH,
                QUAD_WIDTH * QUAD_ASPECT,
            );
            // Dörtgenler X ekseninde −36° yatık duruyor.
            let tilt = Quat::from_rotation_x(-PI / 5.0);

            // Üç malzeme: beyaz çarpan (dokunun kendisi), kırmızı ve mavi modülasyon.
            for (z, albedo) in [
                (1.5, Vec4::new(1.0, 1.0, 1.0, 1.0)),
                (0.0, Vec4::new(1.0, 0.0, 0.0, 0.5)),
                (-1.5, Vec4::new(0.0, 0.0, 1.0, 0.5)),
            ] {
                scene.world.spawn_bundle((
                    Transform::new(Vec3::new(0.0, 0.0, z)).with_rotation(tilt),
                    GlobalTransform::default(),
                    quad.clone(),
                    // `with_unlit` hem gölgelemeyi kapatır hem — alfa 1'in altındaysa —
                    // malzemeyi saydam kovaya alır; ışıksızlık ve harmanlama tek çağrıda.
                    Material::new(texture.clone()).with_unlit(albedo).with_double_sided(true),
                    MeshRenderer::new(),
                ));
            }

            scene.spawn_camera(state, Vec3::new(0.0, 3.0, 8.0), Vec3::ZERO);
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}
