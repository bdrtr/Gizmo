//! # Düzlemsel yansıma (ayna)
//!
//! Bir aynanın, bir su yüzeyinin ya da cilalı bir zeminin arkasındaki dünyayı göstermesi. Klasik
//! yolu şu: sahneyi aynanın düzlemine göre yansıtılmış bir kameradan bir dokuya çiz, sonra o
//! dokuyu aynanın malzemesi olarak bağla.
//!
//! ## Motorda yok, ve dört adımın üçü tıkalı
//!
//! | adım | Gizmo |
//! |------|-------|
//! | 1. yansıtılmış kamera matrisi | **elle** kurulabilir |
//! | 2. dokuya çizmek | **var** — `default_render_pass` bir `TextureView` alıyor |
//! | 3. o dokuyu malzemeye bağlamak | **yok** — bağlama grubu kurucusu `pub(crate)` |
//! | 4. aynanın düzleminde kırpmak | **yok** — eğik frustum yok, stencil tamponu yok |
//!
//! İkinci adımın çalıştığı [`render_to_texture`](../render_to_texture/index.html) demosunda
//! ölçüldü. Üçüncü adım, [`parallax_mapping`](../parallax_mapping/index.html)'in bulduğu duvarın
//! aynısı: `AssetManager::assemble_material_bind_group` `pub(crate)`, ve genel API'nin doku
//! bağlayan tek yolu diskten taban renk okumak. Çevrim dışı bir hedefi malzemeye bağlamanın
//! genel bir yolu yok.
//!
//! Dördüncü adım da eksik: aynanın **arkasında** kalan geometriyi kırpacak bir eğik frustum ya da
//! stencil yok, yani yansıtılmış çizim aynanın gerisindeki nesneleri de içine alır.
//!
//! ## Elde kalan tek yansıma: ekran uzayı
//!
//! [`ssr`](../ssr/index.html) demosunda ölçülen SSR çalışıyor — ama tanımı gereği **yalnız
//! ekranda olanı** yansıtıyor. Bir ayna için bu, en çok ihtiyaç duyulan şeyi (kameranın arkası)
//! dışarıda bırakmak demek. Demo bunu ölçüyor: aynanın karşısına konan bir nesne, kameradan
//! görünmediği sürece yansımada da yok.
//!
//! ## Ölçüldü — SSR'ın yansıtamadığı şey
//!
//! Pürüzsüz metalik bir zemin, ve karşısında parlak kırmızı bir sütun. Tek değişen sütunun
//! kameranın **önünde** mi **arkasında** mı olduğu. Ölçülen: zeminin renk dengesi
//! (2026-08-23, alt bölge, sol yarı):
//!
//! | sütunun yeri | zemin RGB | kırmızı − mavi |
//! |--------------|-----------|----------------|
//! | görüş alanında | (78,51 · 75,58 · 82,58) | **−4,07** |
//! | kameranın arkasında | (68,93 · 73,26 · 80,91) | **−11,98** |
//!
//! Sütun görünürken zemine **+7,91** kırmızı katıyor; ekrandan çıkınca o katkı **tamamen**
//! kayboluyor. Nesne hâlâ orada, hâlâ aynanın karşısında, ve yansımada yok.
//!
//! Gerçek bir aynanın en çok işe yaradığı durum tam olarak budur — kameranın göremediğini
//! göstermek. Ekran uzayı yansıması bunu yapısal olarak yapamıyor, ve bu sayı o yapısal sınırın
//! ölçüsü.
//!
//! ## Kontroller
//!   * `GIZMO_MIRROR_BEHIND=1` — nesneyi kameranın **arkasına** taşı
//!   * **Sağ-tık + fare / WASDQE** — kamera (ölçüm için dokunmayın)

use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

fn main() {
    let behind = std::env::var("GIZMO_MIRROR_BEHIND").is_ok();

    App::<SimpleSceneState>::new("Gizmo Engine - Mirror", 1280, 720)
        .with_simple_scene(move |scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;

            // "Ayna": olabildiğince pürüzsüz ve metalik bir zemin. SSR'ın çalışacağı yüzey bu.
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -1.0, 0.0)),
                GlobalTransform::default(),
                AssetManager::create_plane(device, 30.0),
                Material::new(white.clone()).with_pbr(Vec4::new(0.55, 0.57, 0.62, 1.0), 0.04, 0.9),
                MeshRenderer::new(),
            ));

            // Yansıması aranan nesne: parlak kırmızı bir sütun.
            // `behind` ise kameranın ARKASINA konuyor — ekranda görünmüyor.
            let z = if behind { 9.0 } else { -4.0 };
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, 0.9, z)).with_scale(Vec3::new(0.7, 2.0, 0.7)),
                GlobalTransform::default(),
                AssetManager::create_cube(device),
                Material::new(white.clone()).with_pbr(Vec4::new(0.95, 0.15, 0.10, 1.0), 0.4, 0.0),
                MeshRenderer::new(),
            ));

            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.3) * Quat::from_rotation_x(-0.6),
                intensity: 3.0,
                ..Default::default()
            });
            let _ = white;
            scene.spawn_camera(state, Vec3::new(0.0, 0.6, 5.0), Vec3::new(0.0, -0.4, -2.0));
            gizmo::gizmo_log!(Info, "nesne kameranın arkasında: {}", behind);
        })
        // SSR'ı BIRAKIYORUZ: basit sahne onu varsayılan olarak kapatıyor, ama ölçülecek tek
        // yansıma o.
        .set_render(|world, _state, encoder, view, renderer, _lt| {
            renderer.gpu_physics = None;
            renderer.gpu_fluid = None;
            renderer.gpu_particles = None;
            gizmo::systems::default_render_pass(world, encoder, view, renderer);
        })
        .set_ui(move |_world, _state, ctx| {
            gizmo::egui::Area::new("mr".into())
                .anchor(gizmo::egui::Align2::RIGHT_TOP, [-12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(420.0);
                        ui.heading("Düzlemsel yansıma");
                        ui.label(format!(
                            "nesne: {}",
                            if behind { "kameranın ARKASINDA" } else { "görüş alanında" }
                        ));
                        ui.separator();
                        ui.label("düzlemsel ayna YOK: yansıma düzlemi bileşeni yok,");
                        ui.label("eğik frustum yok, stencil tamponu yok.");
                        ui.separator();
                        ui.label("dokuya çizmek VAR, ama o dokuyu malzemeye");
                        ui.label("bağlamanın genel bir yolu YOK (pub(crate) kurucu).");
                        ui.separator();
                        ui.colored_label(
                            gizmo::egui::Color32::from_rgb(230, 160, 80),
                            "elde kalan: SSR — yalnız ekranda olanı yansıtır",
                        );
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}
