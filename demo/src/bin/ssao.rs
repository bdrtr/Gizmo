//! # Ekran uzayı örtüşme gölgesi
//!
//! Ekran uzayında örtüşme gölgesi: iki yüzeyin birleştiği yerde ışığın azalması. Işık
//! hesabının yakalayamadığı, ama gözün "bu köşe" diye okuduğu karartma.
//!
//! ## Envanter: tek bir düğme
//!
//! | yetenek | Gizmo |
//! |---------|-------|
//! | kalite kademesi (dilim ve dilim başına örnek sayısı) | **yok** |
//! | zamansal gürültü giderme | **yok** |
//! | şiddet | [`SsaoParams::strength`], 0..1 arası **tek** sayı |
//!
//! Yani motorun SSAO'su açılıp kapanabiliyor ve ne kadar karartacağı söylenebiliyor; kaç örnek
//! alacağı, hangi yarıçapta arayacağı ve gürültüsünün nasıl gideceği seçilemiyor. Kalite diye
//! ayarlanabilecek her şey burada tek bir kaydırağa iniyor.
//!
//! ## Ölçülen: karartma bir sayıdır
//!
//! Demo sahneyi bilerek köşelerden kuruyor — bir zemin, üstünde duran bir küre, ve üç duvarı
//! olan açık bir kutu. SSAO'nun işi tam olarak bu birleşme yerlerini karartmak.
//!
//! Ölçüm iki bölgenin ortalama parlaklığını karşılaştırıyor: **birleşim** (duvarın zeminle
//! buluştuğu içbükey şerit) ve **açık zemin** (ortada, hiçbir şeye yakın olmayan). SSAO doğru
//! çalışıyorsa şiddet arttıkça birleşim kararır, açık zemin kararmaz.
//!
//! Ölçüldü (2026-08-23, kare 240, sabit kamera, `GIZMO_SSAO=<şiddet>`, HUD hariç):
//!
//! | şiddet | birleşim | açık zemin | birleşimin kararması |
//! |--------|----------|------------|----------------------|
//! | 0,0 | 139,02 | 191,51 | 0,00 |
//! | 0,4 | 137,63 | 191,51 | 1,39 |
//! | 0,8 | 136,10 | 191,51 | 2,92 |
//! | 1,0 | 135,26 | 191,51 | **3,77** |
//!
//! İki şey birden doğru: karartma şiddette **doğrusal**, ve açık zemin **hiç** değişmiyor —
//! 191,51, dört ölçümde de virgülden sonra iki hane aynı. Bu tam olarak `ssao_apply.wgsl`'in
//! yaptığı şey: `mix(1.0, ao_raw, strength)`.
//!
//! ### Karartmanın haritası
//!
//! Kare altıya altı bölünüp şiddet 0 ile 1 arasındaki ortalama fark yazıldığında SSAO'nun
//! imzası doğrudan okunuyor:
//!
//! ```text
//!  0,35  0,51  0,04  0,03  0,50  0,36
//!  0,47  1,84  0,18  0,19  1,81  0,52
//!  0,48  3,64  0,97  0,94  3,77  0,61   <- duvar/zemin birleşimleri
//!  3,35  1,92  1,17  1,16  2,03  3,62   <- duvarların dibi
//!  2,96  0,00  0,00  0,00  0,00  3,18   <- ortadaki açık zemin: TAM SIFIR
//!  0,93  0,62  0,65  0,65  0,62  1,04
//! ```
//!
//! İçbükey birleşmeler kararıyor, düz açıklık hiç dokunulmadan kalıyor.
//!
//! ### Gürültü tabanı, ve ölçünün büyüklüğü
//!
//! Aynı ayarın iki ayrı koşusu arasındaki fark: ortalama 0,0001, piksellerin yalnız %0,002'si
//! 2'den fazla ayrılıyor. SSAO'nun etkisi ise ortalama 1,14 ve piksellerin %8,4'ü — yani ölçüm
//! gürültünün çok üstünde.
//!
//! Ama büyüklüğü abartmamak gerek: birleşimdeki ortalama karartma 255 üzerinden 3,77, yani
//! **%1,5**. Tek tek piksellerde 49'a kadar çıkıyor. SSAO burada ince bir dokunuş, sahneyi
//! yeniden aydınlatan bir şey değil.
//!
//! ## Kontroller
//!   * **1/2/3/4** — şiddet 0 / 0,4 / 0,8 / 1,0
//!   * **Sağ-tık + fare / WASDQE** — kamera (ölçüm için dokunmayın)

use gizmo::core::input::Input;
use gizmo::core::system::{IntoSystemConfig, Phase, Res, ResMut};
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// SSAO'nun tek düğmesi.
#[derive(Clone, Copy)]
struct Ssao {
    strength: f32,
}

impl Default for Ssao {
    fn default() -> Self {
        Self {
            // Başsız bir karede tuşa basacak kimse yok.
            strength: std::env::var("GIZMO_SSAO")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.8f32)
                .clamp(0.0, 1.0),
        }
    }
}
gizmo::core::impl_component!(Ssao);

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - SSAO", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;
            let cube = AssetManager::create_cube(device);

            // Zemin.
            scene.world.spawn_bundle((
                Transform::new(Vec3::ZERO),
                GlobalTransform::default(),
                AssetManager::create_plane(device, 24.0),
                Material::new(white.clone()).with_pbr(Vec4::new(0.72, 0.72, 0.74, 1.0), 0.9, 0.0),
                MeshRenderer::new(),
            ));

            // Üç duvarlı açık kutu — SSAO'nun konusu olan içbükey köşeler burada.
            let walls: [(Vec3, Vec3); 3] = [
                (Vec3::new(0.0, 1.5, -3.0), Vec3::new(3.0, 1.5, 0.1)),
                (Vec3::new(-3.0, 1.5, 0.0), Vec3::new(0.1, 1.5, 3.0)),
                (Vec3::new(3.0, 1.5, 0.0), Vec3::new(0.1, 1.5, 3.0)),
            ];
            for (position, scale) in walls {
                scene.world.spawn_bundle((
                    Transform::new(position).with_scale(scale),
                    GlobalTransform::default(),
                    cube.clone(),
                    Material::new(white.clone())
                        .with_pbr(Vec4::new(0.75, 0.75, 0.78, 1.0), 0.85, 0.0),
                    MeshRenderer::new(),
                ));
            }

            // Zeminde duran küre: değdiği yerde ince bir temas gölgesi bırakmalı.
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, 1.2, 0.0)),
                GlobalTransform::default(),
                AssetManager::create_sphere(device, 1.2, 24, 40),
                Material::new(white).with_pbr(Vec4::new(0.80, 0.78, 0.74, 1.0), 0.6, 0.0),
                MeshRenderer::new(),
            ));

            // Işık bilerek yumuşak ve tepeden: sert gölge SSAO'nun ölçümünü bozar.
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_x(-1.35),
                intensity: 1.6,
                ..Default::default()
            });

            scene.world.insert_resource(Ssao::default());
            scene.spawn_camera(state, Vec3::new(0.0, 4.2, 8.5), Vec3::new(0.0, 1.0, 0.0));
        })
        .add_update_system(ssao_input.in_phase(Phase::Update))
        // SSAO `Renderer`'ın `Option` alanı; sistemler oraya erişemiyor.
        .set_render(|world, _state, encoder, view, renderer, _light_time| {
            renderer.gpu_physics = None;
            renderer.gpu_fluid = None;
            renderer.gpu_particles = None;
            renderer.ssr = None;
            renderer.ssgi = None;

            if let Some(ssao) = world.get_resource::<Ssao>().map(|s| *s) {
                let queue = renderer.queue.clone();
                if let Some(state) = renderer.ssao.as_ref() {
                    state.set_strength(&queue, ssao.strength);
                }
            }

            gizmo::systems::default_render_pass(world, encoder, view, renderer);
        })
        .set_ui(|world, _state, ctx| {
            let Some(ssao) = world.get_resource::<Ssao>().map(|s| *s) else {
                return;
            };
            gizmo::egui::Area::new("ssao".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(400.0);
                        ui.heading("Ekran uzayı örtüşme gölgesi");
                        ui.label(format!("şiddet: {:.2}", ssao.strength));
                        ui.separator();
                        ui.label("motorun tek düğmesi bu: 0 = kapalı, 1 = ham terim");
                        ui.label("kalite kademesi seçimi yok");
                        ui.label("zamansal gürültü giderme de yok");
                        ui.separator();
                        ui.label("1/2/3/4 — 0 / 0,4 / 0,8 / 1,0");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Şiddeti seçen tuşlar. Rakamlar bilerek: WASDQE kameranın.
fn ssao_input(mut ssao: ResMut<Ssao>, input: Res<Input>) {
    use gizmo::winit::keyboard::KeyCode;
    for (key, strength) in [
        (KeyCode::Digit1, 0.0),
        (KeyCode::Digit2, 0.4),
        (KeyCode::Digit3, 0.8),
        (KeyCode::Digit4, 1.0),
    ] {
        if input.is_key_just_pressed(key as u32) {
            ssao.strength = strength;
        }
    }
}
