//! # Bevy'nin `3d/orthographic` örneğinin Gizmo Engine portu
//!
//! Dört küp ve bir zemin, **ortografik** kamerayla: perspektif kaçış noktası olmadığı için
//! eşit büyüklükteki küpler ekranda da eşit görünür — izometrik görünümlü oyunların ve CAD
//! uygulamalarının kamerası budur.
//!
//! Bu, deponun ortografik projeksiyonu gösteren İLK demosu. Motorda karşılığı zaten var ve
//! Bevy'nin ifadesiyle birebir örtüşüyor: Bevy `ScalingMode::FixedVertical { viewport_height: 6.0 }`
//! diyor, Gizmo [`ProjectionMode::Orthographic`] `{ height: 6.0 }` diyor — ikisi de "görüş
//! hacminin DİKEY uzanımı 6 dünya birimi, yatay olan en-boy oranından türetilir" demek.
//!
//! Tek pürüz: `CameraBundle` bu alanı taşımıyor (projeksiyon `Camera` bileşeninin kendi alanı),
//! bu yüzden kamera `spawn_camera` ile kurulduktan sonra bileşene doğrudan yazılıyor. Motorda
//! kapatılacak bir boşluk olarak not düşmeye değer; bugünkü hâliyle üç satır.
//!
//! Sayısal parite notları (Bevy 0.19 kaynağına karşı):
//!   * Kamera `(5, 5, 5)`'ten orijine bakar, dikey uzanım 6 birim.
//!   * Zemin 5×5, rengi `srgb(0.3, 0.5, 0.3)` → doğrusal `(0.073, 0.214, 0.073)`.
//!   * Dört küp `(±1.5, 0.5, ±1.5)`'te, 1×1×1, rengi `srgb(0.8, 0.7, 0.6)` → doğrusal
//!     `(0.604, 0.448, 0.319)`.
//!   * Nokta ışığı `(3, 8, 5)`'te.
//!
//! ## Kontroller
//!   * **Sağ-tık + fare** — bak · **WASDQE** — kamera · **Shift** — hızlı hareket
//!     (kamera hareket etse de projeksiyon ortografik kalır — asıl görülecek şey de bu).

use gizmo::core::query::Mut;
use gizmo::prelude::*;
use gizmo::renderer::components::ProjectionMode;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// `Color::srgb(0.8, 0.7, 0.6)`'nın doğrusal karşılığı.
const CUBE_ALBEDO: Vec4 = Vec4::new(0.603_827_2, 0.447_988_8, 0.318_546_9, 1.0);
/// `Color::srgb(0.3, 0.5, 0.3)`'ün doğrusal karşılığı.
const GROUND_ALBEDO: Vec4 = Vec4::new(0.073_239_45, 0.214_041_14, 0.073_239_45, 1.0);
/// Bevy'nin `viewport_height`'i: görüş hacminin dikey uzanımı, dünya birimi.
const VIEWPORT_HEIGHT: f32 = 6.0;

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Bevy Orthographic", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );

            // Zemin — 5×5 düzlem.
            let plane = AssetManager::create_plane(&scene.renderer.device, 5.0);
            scene.world.spawn_bundle((
                Transform::new(Vec3::ZERO),
                GlobalTransform::default(),
                plane,
                Material::new(white.clone()).with_pbr(GROUND_ALBEDO, 1.0, 0.0),
                MeshRenderer::new(),
            ));

            // Dört küp — köşelerde, yarısı zeminin üstünde.
            let cube_mesh = AssetManager::create_cube(&scene.renderer.device);
            for (x, z) in [(1.5, 1.5), (1.5, -1.5), (-1.5, 1.5), (-1.5, -1.5)] {
                scene.world.spawn_bundle((
                    Transform::new(Vec3::new(x, 0.5, z)).with_scale(Vec3::splat(0.5)),
                    GlobalTransform::default(),
                    cube_mesh.clone(),
                    Material::new(white.clone()).with_pbr(CUBE_ALBEDO, 0.5, 0.0),
                    MeshRenderer::new(),
                ));
            }

            scene.world.spawn_bundle(PointLightBundle {
                position: Vec3::new(3.0, 8.0, 5.0),
                intensity: 40.0,
                radius: 40.0,
                ..Default::default()
            });

            // Kamera (5, 5, 5) → orijin, sonra projeksiyonu ortografiğe çevir.
            scene.spawn_camera(state, Vec3::new(5.0, 5.0, 5.0), Vec3::ZERO);
            if let Some(mut q) = scene.world.query_mut::<(Mut<Camera>, Mut<Transform>)>() {
                for (_entity, (mut camera, _transform)) in q.iter_mut() {
                    camera.projection = ProjectionMode::Orthographic {
                        height: VIEWPORT_HEIGHT,
                    };
                }
            }
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}
