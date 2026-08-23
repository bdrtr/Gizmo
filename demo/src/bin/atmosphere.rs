//! # Gökyüzü ve çevre aydınlatması
//!
//! Sahnenin üstündeki gökyüzü ve ondan gelen dolaylı ışık. İkisi aynı fiziksel şeyin iki yüzü:
//! gökyüzü ne renkse, gölgede kalan yüzeylere düşen ortam ışığı da o renk olmalı.
//!
//! ## Ölçülen: gökyüzü küpü **hiç çizilmiyor** — ve sebebini bulamadım
//!
//! Demo bir gökyüzü küpü kurmakla başladı; kurmayınca kare hiç değişmedi.
//!
//! Kontrol (2026-08-23): sahne iki kez, tek fark gökyüzü varlığının doğurulup doğurulmaması.
//! **0 farklı piksel, maks 0, fark kutusu yok.** Ekranda görünen gökyüzü tamamen ertelenmiş
//! aydınlatmanın çevre terimi.
//!
//! Elediklerim — hiçbiri sebep değil:
//!
//! | denenen | sonuç |
//! |---------|-------|
//! | ölçek 400 · 100 · 30 · 12 | dördünde de 0 fark |
//! | `GlobalTransform` eklenmiş / eklenmemiş | ikisinde de 0 fark |
//! | `MaterialType::Skybox` / `Unlit` | ikisinde de 0 fark |
//! | ters küp / sıradan küp | ikisinde de 0 fark |
//! | uzaklık düzlemi | far = 1500, küp ±200'de — içeride |
//! | mesh geçerli mi | 24 indeksli köşe, sınırlar ±1, merkez kayması sıfır |
//! | boru hattı bağlanıyor mu | `forward.rs:115` `is_skybox` için `sky_pipeline`'ı bağlıyor |
//! | yüz ayıklaması | sky boru hattı `cull_mode: None` |
//! | alfa | `sky.wgsl` `vec4(sky_color, 1.0)` döndürüyor |
//! | derinlik | `Clear(1.0)` + `LessEqual`, yani geçmeli |
//! | geçiş sırası | ileri geçiş ertelenmiş aydınlatmadan **sonra** — üstüne yazılmıyor |
//!
//! Aynı sahnedeki öteki küpler (duvar, kutular) sorunsuz çiziliyor, yani genel bir çizim arızası
//! değil. **Sebep açık bir soru.** Bulmadan tahminle kapatmıyorum; ölçüm ve eleme listesi burada,
//! kaldığı yerden sürdürülebilir.
//!
//! ## Fiziksel atmosfer modeli yok
//!
//! | yetenek | Gizmo |
//! |---------|-------|
//! | bulanıklık (turbidity) / ozon / gezegen yarıçapı | **yok** |
//! | optik derinlik integrali, geçirgenlik LUT'u | **yok** |
//! | hava perspektifi (aerial perspective) | **yok** |
//! | gökyüzü rengini elle vermek | **yok** — `sky.wgsl`'in gradyan durakları gömülü sabit |
//! | küp haritası gökyüzü | **yok** — küp örnekleyici bağlaması yok, dışarıdan eklenemiyor |
//! | çevre haritasını döndürmek | **yok** |
//!
//! `sky.wgsl`'in tepe `(0,08 · 0,25 · 0,6)`, ufuk `(0,5 · 0,7 · 0,9)` ve zemin `(0,2 · 0,2 · 0,2)`
//! renkleri kodda sabit, ve yalnız güneşin rengiyle çarpılıyorlar. Preset'i de okumuyor — çevre
//! preset'ini yalnız `deferred_lighting.wgsl` görüyor (üç dal).
//!
//! Küp haritası kapısı çift kilitli: malzeme doku yerleşimi `build_layouts`'ta kuruluyor ve
//! `Layouts` `pub(super)`, `MaterialType` de kapalı bir enum. Altı yüzlü bir gökyüzü tek bir 2B
//! dokuya açılmak zorunda ([`Material::with_backdrop`]).
//!
//! Çevre haritasını döndürmek tek satırlık bir iş değil: yeni bir `SceneUniforms` alanı gerekir,
//! ve `shader_contract.rs` çevre alanlarını **bayt konumuyla** sabitliyor.
//!
//! ## Ölçülen: preset gerçekten çalışıyor — ama yalnız ertelenmiş yolda
//!
//! Üç preset, aynı sahne (2026-08-23, 948×1028):
//!
//! | preset | üst şerit RGB | duvarın gölgesindeki yer RGB |
//! |--------|---------------|------------------------------|
//! | 0 | (169,97 · 141,21 · 125,40) | (212,91 · 207,01 · 205,27) |
//! | 1 | (126,28 · 128,99 · 137,50) | (202,00 · 202,37 · 204,01) |
//! | 2 | (111,93 · 104,56 · 126,58) | (200,34 · 196,03 · 205,64) |
//!
//! Preset 0 sıcak, 1 nötr-mavi, 2 soğuk-mor: hem arka plan hem gölgedeki ortam ışığı birlikte
//! kayıyor. Yani çevre **tutarlı** — çünkü ikisini de aynı gölgelendirici üretiyor. Gökyüzü
//! malzemesi devreye girseydi bu tutarlılık bozulacaktı, çünkü o preset'i okumuyor.
//!
//! ## Kontroller
//!   * `GIZMO_ATMO_PRESET=0|1|2` — çevre preset'i
//!   * `GIZMO_ATMO_NOSKY=1` — gökyüzü küpünü hiç doğurma (kontrol)
//!   * `GIZMO_ATMO_SKYSCALE=<n>` · `GIZMO_ATMO_UNLIT=1` · `GIZMO_ATMO_NORMALCUBE=1` — eleme denemeleri
//!   * **Sağ-tık + fare / WASDQE** — kamera (ölçüm için dokunmayın)

use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

fn preset() -> u32 {
    std::env::var("GIZMO_ATMO_PRESET")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0u32)
        .min(2)
}

fn main() {
    let p = preset();

    App::<SimpleSceneState>::new("Gizmo Engine - Atmosphere", 1280, 720)
        .with_simple_scene(move |scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;

            // Gökyüzü: içe bakan bir küp + `with_skybox`. Bu, `sky.wgsl` yolunu açıyor.
            // `GIZMO_ATMO_NOSKY=1` ile hiç doğurulmuyor — kontrol koşusu.
            if std::env::var("GIZMO_ATMO_NOSKY").is_err() {
            let probe = AssetManager::create_inverted_cube(device);
            gizmo::gizmo_log!(
                Info,
                "ters küp: {} köşe · sınırlar min {:?} maks {:?} · merkez kayması {:?}",
                probe.vertex_count,
                (probe.bounds.min.x, probe.bounds.min.y, probe.bounds.min.z),
                (probe.bounds.max.x, probe.bounds.max.y, probe.bounds.max.z),
                (probe.center_offset.x, probe.center_offset.y, probe.center_offset.z)
            );
            scene.world.spawn_bundle((
                Transform::new(Vec3::ZERO).with_scale(Vec3::splat(
                    std::env::var("GIZMO_ATMO_SKYSCALE")
                        .ok()
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(400.0f32),
                )),
                // `GIZMO_ATMO_NORMALCUBE=1`: ters küp yerine sıradan küp — ayırıcı deney.
                if std::env::var("GIZMO_ATMO_NORMALCUBE").is_ok() {
                    AssetManager::create_cube(device)
                } else {
                    AssetManager::create_inverted_cube(device)
                },
                // `GIZMO_ATMO_UNLIT=1`: aynı küp, ama Unlit malzemeyle. Ayırıcı deney.
                if std::env::var("GIZMO_ATMO_UNLIT").is_ok() {
                    Material::new(white.clone()).with_unlit(Vec4::new(0.20, 0.35, 0.75, 1.0))
                } else {
                    Material::new(white.clone()).with_skybox()
                },
                MeshRenderer::new(),
            ));
            }

            // Yer: ortam ışığının okunacağı yüzey. Gölgede kalan tarafı preset'e tepki vermeli.
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -1.2, 0.0)),
                GlobalTransform::default(),
                AssetManager::create_plane(device, 40.0),
                Material::new(white.clone()).with_pbr(Vec4::new(0.72, 0.72, 0.74, 1.0), 0.85, 0.0),
                MeshRenderer::new(),
            ));
            // Güneşi kesen bir duvar: arkasında kalan yer yalnız ortam ışığı alıyor.
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, 1.2, -1.0)).with_scale(Vec3::new(9.0, 4.6, 0.4)),
                GlobalTransform::default(),
                AssetManager::create_cube(device),
                Material::new(white.clone()).with_pbr(Vec4::new(0.35, 0.36, 0.40, 1.0), 0.8, 0.0),
                MeshRenderer::new(),
            ));

            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.1) * Quat::from_rotation_x(-1.15),
                intensity: 2.6,
                ..Default::default()
            });
            let _ = white;
            scene.spawn_camera(state, Vec3::new(0.0, 1.6, 8.0), Vec3::new(0.0, 0.6, 0.0));
            gizmo::gizmo_log!(Info, "çevre preset: {}", p);
        })
        .set_render(move |world, _state, encoder, view, renderer, _lt| {
            renderer.gpu_physics = None;
            renderer.gpu_fluid = None;
            renderer.gpu_particles = None;
            renderer.ssr = None;
            renderer.ssgi = None;
            // Preset yalnız ertelenmiş aydınlatmaya ulaşıyor; gökyüzü onu görmüyor.
            renderer.environment_preset = p;
            renderer.environment_preset_2 = p;
            renderer.environment_blend_t = 0.0;
            gizmo::systems::default_render_pass(world, encoder, view, renderer);
        })
        .set_ui(move |_world, _state, ctx| {
            gizmo::egui::Area::new("at".into())
                .anchor(gizmo::egui::Align2::RIGHT_TOP, [-12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(420.0);
                        ui.heading("Gökyüzü ve çevre");
                        ui.label(format!("environment_preset: {p}"));
                        ui.separator();
                        ui.label("sky.wgsl preset'i OKUMUYOR — gradyan sabit.");
                        ui.label("deferred_lighting.wgsl üç preset dalı taşıyor.");
                        ui.colored_label(
                            gizmo::egui::Color32::from_rgb(230, 160, 80),
                            "iki gökyüzü birbirinden habersiz",
                        );
                        ui.separator();
                        ui.label("fiziksel atmosfer modeli yok: bulanıklık, ozon,");
                        ui.label("optik derinlik, hava perspektifi — hiçbiri.");
                        ui.label("küp haritası gökyüzü de yok (bağlama kapalı).");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}
