//! # Bevy'nin `3d/ssr` örneğinin Gizmo Engine portu
//!
//! Ekran uzayı yansımaları: parlak bir yüzeyin, karşısındaki nesneleri **ekranda zaten çizilmiş**
//! pikselleri tarayarak yansıtması. Ucuz, ve sınırı da oradan geliyor — ekranda olmayan bir şey
//! yansıyamaz.
//!
//! ## Envanter: altı düğmeye karşı sıfır
//!
//! | Bevy'nin `ScreenSpaceReflections`'ı | Gizmo |
//! |------------------------------------|-------|
//! | `perceptual_roughness_threshold` | **yok** |
//! | `thickness` | **yok** |
//! | `linear_steps` | **yok** |
//! | `linear_march_exponent` | **yok** |
//! | `bisection_steps` | **yok** |
//! | `use_secant` | **yok** |
//!
//! `SsrState`'in ayarlanabilir tek alanı genişlik ve yükseklik — yani yalnız hedef boyutu.
//! Yansımanın ne kadar uzağa bakacağı, hangi pürüzden sonra vazgeçeceği, kaç adımda tarayacağı
//! shader'ın içinde.
//!
//! ## Açıp kapatmak da tek yönlü
//!
//! TAA ve FXAA'nın `enabled` bayrağı var (bkz. `bevy_anti_aliasing`), yani kare kare
//! açılıp kapanabiliyorlar. **SSR'de yoktu:** `Renderer::ssr` bir `Option<SsrState>`, ve bir
//! uygulamanın elindeki tek hareket onu `None` yapmak — yani durumu **yok etmek**. Geri açmak
//! `SsrState::new` ile yeniden kurmayı gerektiriyor (cihaz, format ve boyutla). Bu yüzden demo
//! kipi çalışma anında değil **başlangıçta** seçiyor: `GIZMO_SSR=0` kapalı, `1` açık.
//!
//! Aynı şey SSGI için de geçerli. Serideki bütün demoların `set_render` kapanışında
//! `renderer.ssr = None` yazmasının sebebi de bu: kapatmanın tek yolu bu.
//!
//! ## Ölçülen: yansıma bir sayıdır
//!
//! Sahne, parlak (düşük pürüzlü) bir zeminin üstüne konmuş üç renkli küp. SSR çalışıyorsa
//! zeminin **küplerin tam altındaki** bölgesi küplerin rengini almalı; SSR kapalıyken orası
//! zeminin kendi rengi olmalı.
//!
//! Ölçüt de bu: zeminin o bölgesindeki renk kanallarının küpün rengine kayması.
//!
//! Ölçüldü (2026-08-23, kare 240, `GIZMO_SSR=<0|1>`). Zeminin, küpün tam altındaki bölgesinde
//! ortalama RGB:
//!
//! | bölge | SSR kapalı | SSR açık | kanal farkı |
//! |-------|------------|----------|-------------|
//! | kırmızı küpün altı | (31,3 · 33,4 · 38,0) | (91,0 · 47,5 · 52,1) | (**+59,7** · +14,1 · +14,1) |
//! | yeşil küpün altı | (31,0 · 33,0 · 37,6) | (45,3 · 93,9 · 55,6) | (+14,3 · **+60,9** · +18,0) |
//! | mavi küpün altı | (31,4 · 33,5 · 38,0) | (47,5 · 58,6 · 91,0) | (+16,1 · +25,2 · **+53,0**) |
//!
//! SSR kapalıyken üç bölge de aynı nötr karanlık — (31 · 33 · 38), yani zeminin kendi rengi.
//! Açıldığında her bölge **kendi küpünün** kanalını alıyor, ve baskın kanal ötekilerin üç-dört
//! katı. Yansıma doğru rengi doğru yere taşıyor.
//!
//! ## Kontroller
//!   * **Sağ-tık + fare / WASDQE** — kamera

use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Küplerin renkleri — kanal kanal ayrıldıkları için yansımada hangisinin geldiği okunabiliyor.
const CUBES: [(f32, Vec4); 3] = [
    (-3.2, Vec4::new(0.95, 0.15, 0.15, 1.0)),
    (0.0, Vec4::new(0.15, 0.95, 0.20, 1.0)),
    (3.2, Vec4::new(0.20, 0.35, 0.95, 1.0)),
];

/// SSR açık mı — başlangıçta seçiliyor, çalışma anında değiştirilemiyor.
fn ssr_enabled() -> bool {
    std::env::var("GIZMO_SSR").map(|v| v != "0").unwrap_or(true)
}

fn main() {
    let enabled = ssr_enabled();

    App::<SimpleSceneState>::new("Gizmo Engine - Bevy SSR", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;

            // Ayna gibi zemin: pürüz düşük, metaliklik yüksek. SSR'nin bakacağı yüzey bu.
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, 0.0, 0.0)),
                GlobalTransform::default(),
                AssetManager::create_plane(device, 40.0),
                Material::new(white.clone()).with_pbr(Vec4::new(0.06, 0.06, 0.07, 1.0), 0.08, 0.9),
                MeshRenderer::new(),
            ));

            let cube = AssetManager::create_cube(device);
            for (x, albedo) in CUBES {
                scene.world.spawn_bundle((
                    Transform::new(Vec3::new(x, 1.1, 0.0)).with_scale(Vec3::splat(0.55)),
                    GlobalTransform::default(),
                    cube.clone(),
                    Material::new(white.clone()).with_pbr(albedo, 0.35, 0.0),
                    MeshRenderer::new(),
                ));
            }

            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.4) * Quat::from_rotation_x(-0.8),
                intensity: 2.4,
                ..Default::default()
            });

            scene.spawn_camera(state, Vec3::new(0.0, 2.2, 8.5), Vec3::new(0.0, 0.6, 0.0));
        })
        .set_render(move |world, _state, encoder, view, renderer, _light_time| {
            renderer.gpu_physics = None;
            renderer.gpu_fluid = None;
            renderer.gpu_particles = None;
            renderer.ssgi = None;
            // SSR'yi kapatmanın tek yolu durumu yok etmek: `enabled` bayrağı yok.
            if !enabled {
                renderer.ssr = None;
            }

            gizmo::systems::default_render_pass(world, encoder, view, renderer);
        })
        .set_ui(move |_world, _state, ctx| {
            gizmo::egui::Area::new("ssr".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(430.0);
                        ui.heading("Ekran uzayı yansıması");
                        ui.label(format!("SSR: {}", if enabled { "açık" } else { "kapalı" }));
                        ui.label("zemin: pürüz 0,08 · metaliklik 0,9");
                        ui.separator();
                        ui.label("motorda SSR'nin AYAR DÜĞMESİ YOK — açık ya da kapalı.");
                        ui.label("Bevy'de altı tane: pürüz eşiği, kalınlık, adım sayısı,");
                        ui.label("adım üsteli, ikiye bölme adımı, sekant.");
                        ui.separator();
                        ui.label("enabled bayrağı da yok: kapatmak durumu YOK ETMEK demek,");
                        ui.label("o yüzden kip başlangıçta seçiliyor (GIZMO_SSR=0|1).");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}
