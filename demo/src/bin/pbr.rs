//! # PBR malzeme ızgarası
//!
//! Motorun malzeme sisteminin referans kartı: **11 × 5 küre ızgarası**, soldan sağa
//! `roughness` 0 → 1, aşağıdan yukarı `metallic` 0 → 1. Tek bir yönlü ışık altında, iki
//! parametrenin görüntüyü nasıl değiştirdiği tek bakışta okunuyor. Sol alttaki tek küre ise
//! **`unlit`**: ışıktan hiç etkilenmeyen düz renk, karşılaştırma için.
//!
//! ## Dürüst olmak gereken yer: ortam haritası (IBL) yok
//!
//! Metalik satırların gördüğü ışığın neredeyse tamamı bir ortam haritasından gelmelidir, çünkü
//! **saf metal difüz yansıtmaz**: yalnız çevresini yansıtır. Motorda ortam küp haritası /
//! yansıma sondası **yok** (bilinen boşluklarından biri). Sonuç şu: bu ızgarada `roughness`
//! ekseni beklendiği gibi okunur, ama üst satırlara doğru gidildikçe küreler **kararır** —
//! yansıtacak bir çevre olmadığı için. Bu demonun hatası değil, motorun eksiğinin görüntüsü;
//! demoyu "düzeltmek" için sahte bir ortam ışığı eklemedim.
//!
//! Sahnenin sayıları:
//!   * Küre yarıçapı 0,45; ızgara `x ∈ [−5, 5]`, `y ∈ [−2, 2]`; renk `#ffd891` → doğrusal
//!     `(1.000, 0.687, 0.283)`.
//!   * Kamera `(0, 0, 8)`'den orijine bakar ve **ortografiktir**. Görüş yüksekliği = pencere
//!     yüksekliği × 0,01; 720 piksellik pencerede **7,2 dünya birimi** — buraya o değer yazıldı.
//!   * Yönlü ışık `(50, 50, 50)`'den orijine bakar (yaw 45°, pitch −35,26°).
//!
//! ## Kontroller
//!   * **Sağ-tık + fare** — bak · **WASDQE** — kamera · **Shift** — hızlı hareket

use gizmo::core::query::Mut;
use gizmo::prelude::*;
use gizmo::renderer::components::ProjectionMode;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};
use std::f32::consts::FRAC_PI_4;

/// `#ffd891`, doğrusal uzayda.
const GOLD: Vec4 = Vec4::new(1.0, 0.686_685, 0.283_149, 1.0);
/// Güneşin (50, 50, 50) → orijin bakışının pitch'i: `asin(-1/√3)`.
const SUN_PITCH: f32 = -0.615_479_7;
/// Ölçek `0.01` × 720 piksellik pencere yüksekliği.
const VIEWPORT_HEIGHT: f32 = 7.2;

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - PBR", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let sphere = AssetManager::create_sphere(&scene.renderer.device, 0.45, 24, 32);

            // Izgara: X ekseni pürüzlülük, Y ekseni metaliklik.
            for y in -2..=2 {
                for x in -5..=5 {
                    let roughness = (x + 5) as f32 / 10.0;
                    let metallic = (y + 2) as f32 / 4.0;
                    scene.world.spawn_bundle((
                        Transform::new(Vec3::new(x as f32, y as f32 + 0.5, 0.0)),
                        GlobalTransform::default(),
                        sphere.clone(),
                        Material::new(white.clone()).with_pbr(GOLD, roughness, metallic),
                        MeshRenderer::new(),
                    ));
                }
            }

            // Işıktan etkilenmeyen küre — düz renk.
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(-5.0, -2.5, 0.0)),
                GlobalTransform::default(),
                sphere,
                Material::new(white).with_unlit(GOLD),
                MeshRenderer::new(),
            ));

            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(FRAC_PI_4) * Quat::from_rotation_x(SUN_PITCH),
                intensity: 2.0,
                ..Default::default()
            });

            // Ortografik kamera — küreler ekranın her yerinde aynı boyutta görünsün diye.
            scene.spawn_camera(state, Vec3::new(0.0, 0.0, 8.0), Vec3::ZERO);
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
