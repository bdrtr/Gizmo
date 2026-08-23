//! # Çıkartmalar
//!
//! Kurşun deliği, su birikintisi, yol çizgisi: yüzeye **yapıştırılan**, geometrisi kendine ait
//! olmayan doku. Nesnenin şeklini bilmek gerekmiyor — projektör kutusu derinlik tamponuyla
//! kesişiyor ve doku oraya düşüyor.
//!
//! ## Motorda var — ve iki çizim yolunda farklı derinlikte
//!
//! | yetenek | Gizmo |
//! |---------|-------|
//! | çıkartma bileşeni | var — [`Decal`], `bind_group` + `color` tinti |
//! | projektör hacmi | var — varlığın `Transform`'u kutuyu tanımlıyor |
//! | ertelenmiş yolda | var — G-tamponuna karşı çiziliyor |
//! | ileri yolda | **kısmen** — ayrı bir kayıt yolu (`record_forward_decals`) |
//! | kümelenmiş (clustered) çıkartma | **yok** — görünüm kümesi başına liste yok |
//! | çıkartma atlası / alt-dikdörtgen | **yok** — çıkartma başına tam doku |
//! | hangi G-tamponu kanalına yazacağını seçmek | **yok** |
//! | yumuşama açısı / kenar solması | **yok** |
//!
//! Yani çıkartmanın kendisi çalışıyor; şekillendirilemeyen şey **hangi kanala**, **ne kadar
//! yumuşak**, ve **kaç tanesinin ucuza** yazılacağı.
//!
//! ## Ölçüldü
//!
//! Aynı sahne iki kez, tek değişen çıkartmaların olup olmadığı. Üç çıkartma kutusu bir zemine ve
//! üç kutuya birden düşüyor. Ölçüldü (2026-08-23, 1904×1028, sol %55, HUD altı):
//!
//! | | değer |
//! |---|-------|
//! | farklı piksel | **%11,05** |
//! | en büyük kanal farkı | 113 |
//! | fark kutusu | `(424, 42)–(1569, 673)` |
//! | kırmızı − mavi, kapalı | −2,85 (nötr sahne) |
//! | kırmızı − mavi, açık | **+0,67** (turuncu tint) |
//!
//! Renk dengesinin işaret değiştirmesi, çıkartmanın gerçekten yüzeye yazıldığını gösteriyor —
//! ve fark kutusunun hem zemini hem kutuları kapsaması, projeksiyonun altındaki geometriyi
//! bilmediğini: aynı kutu neye denk gelirse onun üstüne düşüyor.
//!
//! ## Kontroller
//!   * `GIZMO_DECAL=0|1` — çıkartmaları kapat / aç
//!   * `GIZMO_DECAL_COUNT=<n>` — kaç çıkartma (varsayılan 3)
//!   * **Sağ-tık + fare / WASDQE** — kamera (ölçüm için dokunmayın)

use gizmo::prelude::*;
use gizmo::renderer::components::Decal;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

fn config() -> (bool, usize) {
    let on = !matches!(std::env::var("GIZMO_DECAL").as_deref(), Ok("0"));
    let n = std::env::var("GIZMO_DECAL_COUNT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3usize)
        .clamp(0, 64);
    (on, n)
}

fn main() {
    let (on, count) = config();

    App::<SimpleSceneState>::new("Gizmo Engine - Decal", 1280, 720)
        .with_simple_scene(move |scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            // Çıkartma dokusu: yerleşik dama deseni — yüzeye düştüğü yer bariz olsun.
            let checker = scene.asset_manager.create_checkerboard_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;

            // Üstüne düşecek yüzeyler: bir zemin ve birkaç kutu. Çıkartmanın hepsine birden
            // sarması, geometriyi bilmediğinin kanıtı.
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -1.0, 0.0)),
                GlobalTransform::default(),
                AssetManager::create_plane(device, 24.0),
                Material::new(white.clone()).with_pbr(Vec4::new(0.62, 0.60, 0.58, 1.0), 0.85, 0.0),
                MeshRenderer::new(),
            ));
            let cube = AssetManager::create_cube(device);
            for i in 0..3 {
                scene.world.spawn_bundle((
                    Transform::new(Vec3::new((i as f32 - 1.0) * 2.6, -0.4, 0.0))
                        .with_scale(Vec3::new(1.2, 1.2, 1.2)),
                    GlobalTransform::default(),
                    cube.clone(),
                    Material::new(white.clone()).with_pbr(
                        Vec4::new(0.55, 0.58, 0.66, 1.0),
                        0.6,
                        0.0,
                    ),
                    MeshRenderer::new(),
                ));
            }

            // Çıkartmalar: projektör kutuları. Kutunun içine ne düşerse üstüne yazılıyor.
            if on {
                for i in 0..count {
                    let t = if count <= 1 {
                        0.5
                    } else {
                        i as f32 / (count - 1) as f32
                    };
                    scene.world.spawn_bundle((
                        Transform::new(Vec3::new((t - 0.5) * 6.0, 0.0, 0.6))
                            .with_scale(Vec3::new(2.2, 3.0, 2.2)),
                        GlobalTransform::default(),
                        Decal::new(
                            checker.clone(),
                            Vec4::new(0.95, 0.35 + t * 0.5, 0.20, 0.85),
                        ),
                    ));
                }
            }

            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.4) * Quat::from_rotation_x(-0.65),
                intensity: 2.8,
                ..Default::default()
            });
            let _ = white;
            scene.spawn_camera(state, Vec3::new(0.0, 2.6, 7.4), Vec3::new(0.0, -0.4, 0.0));
            gizmo::gizmo_log!(Info, "çıkartma: {} · adet {}", on, count);
        })
        .set_ui(move |world, _state, ctx| {
            let has_state = world
                .get_resource::<gizmo::renderer::Renderer>()
                .is_some();
            let _ = has_state;
            gizmo::egui::Area::new("dc".into())
                .anchor(gizmo::egui::Align2::RIGHT_TOP, [-12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(410.0);
                        ui.heading("Çıkartmalar");
                        ui.label(format!("açık: {on} · adet: {count}"));
                        ui.separator();
                        ui.label("projektör kutusu = varlığın Transform'u.");
                        ui.label("altındaki geometriyi bilmiyor; derinliğe yazıyor.");
                        ui.separator();
                        ui.label("YOK: kümelenmiş çıkartma listesi");
                        ui.label("YOK: doku atlası / alt-dikdörtgen adresleme");
                        ui.label("YOK: hedef G-tamponu kanalı seçimi");
                        ui.label("YOK: yumuşama açısı, kenar solması");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}
