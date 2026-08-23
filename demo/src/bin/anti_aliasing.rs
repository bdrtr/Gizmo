//! # Kenar yumuşatma
//!
//! Kenar tırtıklarını yumuşatmak. Sahne bilerek en kötü durumu kuruyor: sığ açıyla uzayan bir
//! dama tahtası ve ince direklerden bir çit — ikisi de tırtıklanmanın klasik konuları.
//!
//! ## Envanter: beş tekniğin ikisi var
//!
//! | yetenek | Gizmo |
//! |---------|-------|
//! | MSAA (kapalı / ×2 / ×4 / ×8) | **yok** — motorun her render hedefi `sample_count: 1`, ve ertelenmiş hat zaten MSAA'ya uygun değil |
//! | FXAA (3 duyarlılık kademesi) | **var**, ama açık/kapalı — duyarlılık düğmesi yok |
//! | SMAA (4 hazır ayar) | **yok** |
//! | TAA | **var** (Halton titreşimi + geçmiş tamponu) |
//! | CAS keskinleştirme | **yok** |
//!
//! İkisi de `Renderer`'ın `Option` alanı ve ikisinin de `enabled` bayrağı var; demo dördünü de
//! (kapalı / FXAA / TAA / ikisi) tuşla geziyor.
//!
//! ## Ölçülen: tırtık bir sayıdır
//!
//! "Daha yumuşak görünüyor" bir ölçüm değil. Bu demo kenarı **komşu piksel farkıyla** ölçüyor:
//! tırtıklı bir kenar az sayıda ama çok sert basamaktır, yumuşatılmış kenar ise daha çok ama
//! daha yumuşak basamağa yayılır. İki sayı bunu birlikte yakalıyor — kaç piksel kenara katılıyor,
//! ve en sert basamak ne kadar.
//!
//! Ölçüldü (2026-08-23, kare 240, aynı sabit kamera, `GIZMO_AA=<kip>`, HUD'un satırları hariç).
//! Ölçüt yatay komşu farkı: **kenar pikseli** = farkı 12'yi aşan piksel, **maks basamak** = en
//! sert geçiş.
//!
//! | kip | kenar pikseli | ortalama \|grad\| | maks basamak |
//! |-----|---------------|-------------------|--------------|
//! | kapalı | %4,73 | 3,23 | **191** |
//! | FXAA | %6,04 | 3,17 | 148 |
//! | TAA | %6,45 | 2,89 | 145 |
//! | ikisi | %6,72 | 2,87 | **126** |
//!
//! Tablo tek bir şeyi söylüyor ve tam da kenar yumuşatmanın tanımı bu: kenar **daha çok piksele
//! yayılıyor** (%4,73 → %6,72) ve buna karşılık **en sert basamak kırpılıyor** (191 → 126).
//! İkisi birden en iyisi, ve iki teknik birbirini bozmuyor — FXAA uzamsal, TAA zamansal.
//!
//! "Daha yumuşak görünüyor" bir ölçüm değildir; bu tablo ölçümdür.
//!
//! ## TAA'nın bir şartı var
//!
//! TAA geçmiş kareyi karıştırıyor, yani **duran bir sahnede** yakınsıyor; kamera her kare
//! oynarsa geçmiş her kare geçersizleşir. Demo bu yüzden kamerayı sabit tutuyor, ve ölçüm de
//! 240. karede — geçmişin oturması için yeterince geç.
//!
//! ## Kontroller
//!   * **1** kapalı · **2** FXAA · **3** TAA · **4** ikisi
//!   * **Sağ-tık + fare / WASDQE** — kamera (ölçüm için dokunmayın)

use gizmo::core::input::Input;
use gizmo::core::system::{IntoSystemConfig, Phase, Res, ResMut};
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Hangi kenar yumuşatmanın açık olduğu.
#[derive(Clone, Copy)]
struct AaMode {
    fxaa: bool,
    taa: bool,
}

impl Default for AaMode {
    fn default() -> Self {
        // Başsız bir karede tuşa basacak kimse yok.
        match std::env::var("GIZMO_AA").as_deref() {
            Ok("fxaa") => Self { fxaa: true, taa: false },
            Ok("taa") => Self { fxaa: false, taa: true },
            Ok("both") => Self { fxaa: true, taa: true },
            _ => Self { fxaa: false, taa: false },
        }
    }
}
gizmo::core::impl_component!(AaMode);

/// Çitin direk sayısı ve aralığı — ince ve sık, yani tırtıklanmanın en sevdiği geometri.
const POSTS: i32 = 60;
const POST_GAP: f32 = 0.55;

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Anti-Aliasing", 1280, 720)
        .with_simple_scene(|scene, state| {
            let device = &scene.renderer.device;
            // Dama tahtası motorun kendi dokusu — sığ açıdan bakıldığında en kötü durumu üretir.
            let checker = scene.asset_manager.create_checkerboard_texture(
                device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let white = scene.asset_manager.create_white_texture(
                device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );

            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, 0.0, -30.0)),
                GlobalTransform::default(),
                AssetManager::create_plane(device, 120.0),
                Material::new(checker).with_pbr(Vec4::ONE, 0.9, 0.0),
                MeshRenderer::new(),
            ));

            // İnce direklerden bir çit: her biri bir piksel genişliğine kadar incelen dikey kenar.
            let cube = AssetManager::create_cube(device);
            for i in -POSTS / 2..POSTS / 2 {
                let x = i as f32 * POST_GAP;
                scene.world.spawn_bundle((
                    Transform::new(Vec3::new(x, 1.0, -6.0))
                        .with_scale(Vec3::new(0.03, 1.0, 0.03)),
                    GlobalTransform::default(),
                    cube.clone(),
                    Material::new(white.clone()).with_pbr(Vec4::new(0.9, 0.9, 0.9, 1.0), 0.4, 0.0),
                    MeshRenderer::new(),
                ));
            }

            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.4) * Quat::from_rotation_x(-0.7),
                intensity: 3.0,
                ..Default::default()
            });

            scene.world.insert_resource(AaMode::default());
            // Sığ açı: kamera zemine yakın ve uzağa bakıyor.
            scene.spawn_camera(state, Vec3::new(0.0, 0.9, 4.0), Vec3::new(0.0, 0.7, -40.0));
        })
        .add_update_system(aa_input.in_phase(Phase::Update))
        // İki pas da `Renderer`'ın alanı; sistemler oraya erişemiyor.
        .set_render(|world, _state, encoder, view, renderer, _light_time| {
            renderer.gpu_physics = None;
            renderer.gpu_fluid = None;
            renderer.gpu_particles = None;
            renderer.ssr = None;
            renderer.ssgi = None;

            if let Some(mode) = world.get_resource::<AaMode>().map(|m| *m) {
                if let Some(taa) = renderer.taa.as_mut() {
                    taa.enabled = mode.taa;
                }
                // FXAA'nın bayrağı hem Rust'ta hem uniform'da: `set_enabled` ikisini birden
                // yazıyor, ve kuyruğu istemesinin sebebi bu.
                let queue = renderer.queue.clone();
                if let Some(fxaa) = renderer.fxaa.as_mut() {
                    fxaa.set_enabled(&queue, mode.fxaa);
                }
            }

            gizmo::systems::default_render_pass(world, encoder, view, renderer);
        })
        .set_ui(|world, _state, ctx| {
            let Some(mode) = world.get_resource::<AaMode>().map(|m| *m) else {
                return;
            };
            gizmo::egui::Area::new("aa".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(400.0);
                        ui.heading("Kenar yumuşatma");
                        ui.label(format!(
                            "FXAA: {} · TAA: {}",
                            if mode.fxaa { "açık" } else { "kapalı" },
                            if mode.taa { "açık" } else { "kapalı" }
                        ));
                        ui.separator();
                        ui.label("motorda MSAA YOK — her hedef sample_count: 1");
                        ui.label("SMAA ve CAS keskinleştirme de yok");
                        ui.label("FXAA açık/kapalı; duyarlılık kademeleri yok");
                        ui.separator();
                        ui.label("TAA duran sahnede yakınsar: kamerayı oynatmayın");
                        ui.separator();
                        ui.label("1 kapalı · 2 FXAA · 3 TAA · 4 ikisi");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Dört kipi gezen tuşlar. Rakamlar bilerek: WASDQE kameranın.
fn aa_input(mut mode: ResMut<AaMode>, input: Res<Input>) {
    use gizmo::winit::keyboard::KeyCode;
    for (key, fxaa, taa) in [
        (KeyCode::Digit1, false, false),
        (KeyCode::Digit2, true, false),
        (KeyCode::Digit3, false, true),
        (KeyCode::Digit4, true, true),
    ] {
        if input.is_key_just_pressed(key as u32) {
            mode.fxaa = fxaa;
            mode.taa = taa;
        }
    }
}
