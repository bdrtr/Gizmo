//! # Alan ışıkları ve yumuşak gölgeler
//!
//! Bu demo altı ayrı yeteneği birden ele alıyor, çünkü altısının da cevabı aynı yerden geliyor:
//! **motorda bu ışık/gölge türü yok.**
//!
//! Motorda üç ışık var, ve üçü de **noktasal**: [`PointLight`], [`DirectionalLight`],
//! [`SpotLight`]. Alanı olan ışık, dokusu olan ışık, ve yumuşaklığı mesafeye göre değişen gölge
//! yok.
//!
//! | yetenek | Gizmo |
//! |---------|-------|
//! | dikdörtgen alan ışığı (LTC) | **yok** — `ltc`/`area_light` diye bir şey aranmıyor bile |
//! | küresel alan ışığı, yarıçapı gölgeyi yumuşatır | **yok** — `PointLight::radius` **menzil**, kaynak boyutu değil |
//! | ışığa doku (cookie/gobo) takmak | **yok** |
//! | ekran uzayında temas gölgesi | **yok** |
//! | mesafeye göre yumuşayan gölge (blocker araması) | **yok** — PCF **sabit ~1 teksel** |
//! | paralaks düzeltmeli küp haritası yansıması | **yok** |
//!
//! ## `PointLight::radius` bir tuzak
//!
//! `radius` adı **kaynağın fiziksel boyutunu** düşündürüyor: büyüdükçe gölgenin yumuşadığı, ışığın
//! alan ışığına dönüştüğü o boyut. Motorda aynı isim **menzil** demek — belgesi açık: *"düşüşün
//! sıfıra ulaştığı mesafe, ve aynı zamanda ayıklama sınırı."*
//!
//! Yani `radius`'u büyüten biri yumuşak gölge bekler, **daha geniş ve daha zayıf bir aydınlatma**
//! bulur. Demo bunu ölçüyor.
//!
//! ## Gölge yumuşaklığı sabittir
//!
//! `pcss`'in ölçtüğü şey "gölge, gölgeyi düşüren nesneden uzaklaştıkça yumuşar mı". Gizmo'da
//! yumuşaklık `cascade_params[1] = 1 / gölge haritası çözünürlüğü`, yani **sabit bir teksel** —
//! mesafeden bağımsız. Demo bunu da ölçüyor: aynı ışık, iki farklı yükseklikteki kutu, gölge
//! kenarının keskinliği karşılaştırılıyor.
//!
//! ## Ölçüldü — 1: `radius` menzil, kaynak boyutu değil
//!
//! Üç koşu, tek değişen nokta ışığının yarıçapı. Ölçüm sol yarı (panel sağda),
//! ölçüldü (2026-08-23, 948×492, kare 90):
//!
//! | `radius` | sahnenin ortalama parlaklığı |
//! |----------|------------------------------|
//! | 2 | 136,13 |
//! | 6 | 139,09 |
//! | 20 | **143,16** |
//!
//! Yarıçap on kat büyüyünce sahne **aydınlanıyor** — daha çok yüzey menzile giriyor. Kaynak
//! boyutu olsaydı aynı hareket ışığı bir alan ışığına çevirir ve gölgeleri yumuşatırdı; burada
//! yalnız erişim genişliyor. İsim aynı, anlam farklı.
//!
//! ## Ölçüldü — 2: gölge yumuşaklığı sabit (koddan)
//!
//! `pcss`'in ölçtüğü şey gölge kenarının gölgeyi düşüren nesneye uzaklıkla yumuşaması. Gizmo'da
//! bu değişken yok, ve kanıtı gölge örneklemesinin kendisinde (`shaders/shader.wgsl`):
//!
//! ```wgsl
//! let texel_size = scene.cascade_params.y;      // = 1 / gölge haritası çözünürlüğü
//! for (var x = -1; x <= 1; x++) {
//!     for (var y = -1; y <= 1; y++) {
//!         let offset = vec2<f32>(f32(x), f32(y)) * texel_size;
//! ```
//!
//! Çekirdek **sabit 3×3**, adımı **sabit bir teksel**, ve `cascade_params.y`'nin gerçekten
//! `1 / SHADOW_MAP_RES` olduğunu `frame_uniforms.rs`'deki bir doğrulama (`"y = PCF texel size"`)
//! iddia ediyor. PCSS'in değiştirdiği büyüklük — engelleyici mesafesine göre arama yarıçapı —
//! burada hiç hesaplanmıyor.
//!
//! ### Ölçüm notu: bunu pikselle ölçmeyi denedim, olmadı
//!
//! İlk plan iki kutunun (biri alçak, biri yüksek) gölge kenarlarını piksel piksel karşılaştırmaktı.
//! Olmadı: bu kurulumda zeminin gölge kontrastı ölçülemeyecek kadar zayıf — görüntünün en koyu
//! %5'i gölgeler değil, ufkun üstündeki **gökyüzü** çıktı. Sahneyi kontrastı zorlayacak biçimde
//! yeniden kurmak yerine iddia kodun kendisinden doğrulandı, çünkü orada kesin: değişken yok.
//!
//! ## Kontroller
//!   * `GIZMO_LIGHT_RADIUS=<sayı>` — nokta ışığının yarıçapı (varsayılan 6)
//!   * **Sağ-tık + fare / WASDQE** — kamera

use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Nokta ışığının yarıçapı — kaynak boyutu değil, menzil.
fn light_radius() -> f32 {
    std::env::var("GIZMO_LIGHT_RADIUS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(6.0f32)
        .clamp(0.5, 40.0)
}

fn main() {
    let radius = light_radius();
    App::<SimpleSceneState>::new("Gizmo Engine - Area Lights", 1280, 720)
        .with_simple_scene(move |scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;
            let cube = AssetManager::create_cube(device);

            // İki kutu, farklı yükseklikte. `pcss`'in ölçtüğü şey: alçaktakinin gölgesi keskin,
            // yüksektekinin yumuşak olmalı. Gizmo'da ikisi de aynı keskinlikte olacak.
            for (x, y) in [(-2.6f32, 0.25f32), (2.6, 2.2)] {
                scene.world.spawn_bundle((
                    Transform::new(Vec3::new(x, y, 0.0)).with_scale(Vec3::splat(0.5)),
                    GlobalTransform::default(),
                    cube.clone(),
                    Material::new(white.clone()).with_pbr(
                        Vec4::new(0.80, 0.55, 0.35, 1.0),
                        0.5,
                        0.0,
                    ),
                    MeshRenderer::new(),
                ));
            }

            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -0.3, 0.0)),
                GlobalTransform::default(),
                AssetManager::create_plane(device, 26.0),
                Material::new(white).with_pbr(Vec4::new(0.62, 0.62, 0.65, 1.0), 0.9, 0.0),
                MeshRenderer::new(),
            ));

            // Nokta ışığı: yarıçapı ölçüm koşuları arasında değişiyor.
            scene.world.spawn_bundle(PointLightBundle {
                position: Vec3::new(0.0, 2.6, 2.6),
                color: Vec3::new(1.0, 0.92, 0.80),
                intensity: 9.0,
                radius,
            });
            // Güneş: gölgeyi düşüren şey bu (nokta ışıkları gölge düşürmüyor).
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.25) * Quat::from_rotation_x(-0.7),
                intensity: 2.6,
                ..Default::default()
            });

            gizmo::gizmo_log!(Info, "nokta ışığı yarıçapı (MENZİL, kaynak boyutu değil): {}", radius);
            scene.spawn_camera(state, Vec3::new(0.0, 3.6, 8.0), Vec3::new(0.0, 0.3, 0.0));
        })
        .set_ui(move |_world, _state, ctx| {
            gizmo::egui::Area::new("area".into())
                .anchor(gizmo::egui::Align2::RIGHT_TOP, [-12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(430.0);
                        ui.heading("Alan ışıkları ve yumuşak gölgeler");
                        ui.label(format!("nokta ışığı yarıçapı: {radius}"));
                        ui.separator();
                        ui.label("motorda üç ışık var, üçü de NOKTASAL:");
                        ui.label("  PointLight · DirectionalLight · SpotLight");
                        ui.separator();
                        ui.label("YOK: dikdörtgen alan ışığı · küresel alan ışığı");
                        ui.label("YOK: ışık dokusu (cookie) · temas gölgesi");
                        ui.label("YOK: PCSS (mesafeyle yumuşayan gölge) · PCCM");
                        ui.separator();
                        ui.colored_label(
                            gizmo::egui::Color32::from_rgb(230, 160, 80),
                            "radius = MENZİL, kaynak boyutu DEĞİL",
                        );
                        ui.label("büyütmek gölgeyi yumuşatmaz, ışığı yayar ve zayıflatır.");
                        ui.separator();
                        ui.label("soldaki kutu alçak, sağdaki yüksek —");
                        ui.label("PCSS olsaydı gölge kenarları farklı olurdu.");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}
