//! # Varlık başına çizim maliyeti
//!
//! Varlık başına çizim maliyetini ölçmek: bir küre kabuğuna dağıtılmış on binlerce küp, sabit
//! adımlarla dönen bir kamera, ve kare süresi. Ayarlar çevre değişkenleriyle veriliyor, çünkü
//! demonun `argh` bağımlılığı yok.
//!
//! ## Ölçülen asıl soru: neyin toplu çizimi bozduğu
//!
//! Motorun toplu çizim anahtarı (`BatchKey`) şunlardan oluşuyor: köşe tamponu, malzemenin
//! **doku bind-group işaretçisi**, iskelet, ve yönlendirme bayrakları (saydam / ışıksız /
//! baked-lit / skybox / backdrop / çift yüzlü / gölge / görünürlük). **Malzemenin rengi, pürüzü
//! ve metalikliği anahtarda yok** — onlar örnek tamponuna (`InstanceRaw`) yazılıyor.
//!
//! Yani örnek başına malzeme verisi değiştirmenin burada bir bedeli olmaması gerekir. Demo bunu
//! ölçüyor, üç kip:
//!
//!   * `shared` — tek mesh, tek malzeme: **tek parti**.
//!   * `vary` — her küpe kendi rengi: parti sayısı değişmemeli, süre de.
//!   * `split` — K ayrı küp mesh'i (aynı geometri, ayrı tampon): **K parti**. Tek değişen şey
//!     parti sayısı, o yüzden aradaki fark doğrudan parti başına maliyettir.
//!
//! ## Ölçüldü (2026-08-23, release, 16 384 küp, kare 800, her ayar üç koşu)
//!
//! | kip | parti | koşular (ms) | ortalama |
//! |-----|-------|--------------|----------|
//! | `shared` | 1 | 8,60 · 8,41 · 8,49 | **8,50** |
//! | `vary` | 1 | 8,05 · 8,03 · 8,00 | **8,03** |
//! | `split` | 32 | 10,14 · 9,69 · 9,76 | **9,86** |
//! | `split` | 128 | 11,64 · 11,39 · 11,70 | **11,58** |
//!
//! **Örnek başına malzeme verisi bedava.** `vary` ile `shared` arasında ölçülebilir bir ceza
//! yok — varyantlı olan bir tık daha hızlı bile çıktı, yani parti kesinlikle bölünmüyor: renk
//! zaten örnek tamponunda.
//!
//! **Parti sayısı ise gerçekten pahalı.** 1'den 32'ye çıkmak +1,36 ms (31 parti başına ~44 µs),
//! 32'den 128'e çıkmak +1,72 ms (96 parti başına ~18 µs). Yani ilk partiler daha pahalı —
//! sabit kurulum — sonra parti başına maliyet ~18 µs'ye oturuyor. Geometri, malzeme ve küp
//! sayısı üç ölçümde de aynı; değişen tek şey partinin kendisi.
//!
//! **Ölçmenin iki kuralı, ikisi de pahalıya öğrenildi:**
//!
//!   * **Kare numarası sabit tutulmalı.** Kamera sabit adımla döndüğü için 800. kare ile 900.
//!     kare farklı sayıda küpü ekranda görüyor; aynı ayar bu iki karede 8,50 ve 7,90 ms verdi.
//!   * **Tek koşu ölçüm değil.** Aynı ayarın üç koşusu arasındaki yayılım 0,05–0,45 ms; daha
//!     gürültülü bir turda 0,73 ms'ye kadar çıktı. Bu bandın altındaki hiçbir fark okunmamalı.
//!
//! ## Motorun oyun yolu çizim istatistiği yayınlamıyor
//!
//! `RenderStats` diye bir tip var (çizim çağrısı, üçgen, örnek sayısı) ama onu **yalnız
//! `gizmo-studio`'nun çizim hattı dolduruyor**; `default_render_pass` hiçbir şey yazmıyor. Yani
//! bir oyun kendi çizim çağrısı sayısını okuyamıyor. Demo bu yüzden parti sayısını *kurgudan*
//! biliyor (K mesh = K parti) ve ölçtüğü şey kare süresi.
//!
//! ## Kameranın sabit adımı, ve istatistiğin kaynağı
//!
//! Kamera **sabit açısal adımla** dönüyor, kare süresiyle değil: iki koşuyu karşılaştırılabilir
//! kılan şey bu. Kare süresi motorun kendi
//! [`Analyzer`]'ına örnek olarak yazılıyor, yani ortalama/p95/p99 hesabı demonun kendi
//! aritmetiği değil.
//!
//! ## Çalıştırma
//!
//! ```text
//! GIZMO_CUBES=16384 GIZMO_CUBES_MODE=vary GIZMO_CUBES_SELFTEST=1 \
//!   cargo run --release -p demo --bin many_cubes
//! ```
//!
//! ## Kontroller
//!   * **Sağ-tık + fare / WASDQE** — kamera (ölçüm için dokunmayın; kamera zaten dönüyor)

use gizmo::analysis::{AnalysisPlugin, Analyzer};
use gizmo::core::query::{Mut, Query, With};
use gizmo::core::system::{IntoSystemConfig, Phase, Res, ResMut};
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};
use std::f32::consts::PI;

/// Kamerayı taşıyan ölçüm defteri.
#[derive(Clone)]
struct Bench {
    cubes: usize,
    mode: &'static str,
    /// `split` kipinde kaç ayrı mesh, yani kaç parti.
    batches: usize,
    /// Kameranın yörünge açısı — kare süresinden bağımsız, sabit adımlı.
    angle: f32,
    frame: u32,
}
gizmo::core::impl_component!(Bench);

/// Küre kabuğunun yarıçapı.
const SHELL_RADIUS: f32 = 60.0;
/// Kameranın kare başına dönüş adımı — sabit, ki iki koşu karşılaştırılabilsin.
const CAMERA_STEP: f32 = 0.0035;
/// Kare süresinin yazıldığı seri.
const METRIC_FRAME_MS: &str = "many_cubes.frame_ms";
/// Kendi kendine ölçümün periyodu.
const PROBE_PERIOD: u32 = 120;

/// Küpün sayısı, kipi ve `split` kipindeki mesh sayısı — hepsi çevreden.
fn config() -> (usize, &'static str, usize) {
    let cubes = std::env::var("GIZMO_CUBES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8192usize)
        .clamp(1, 200_000);
    let mode = match std::env::var("GIZMO_CUBES_MODE").as_deref() {
        Ok("vary") => "vary",
        Ok("split") => "split",
        _ => "shared",
    };
    let batches = std::env::var("GIZMO_CUBES_BATCHES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(32usize)
        .clamp(1, 512);
    (cubes, mode, batches)
}

fn main() {
    let (cubes, mode, batches) = config();

    App::<SimpleSceneState>::new("Gizmo Engine - Many Cubes", 1280, 720)
        .with_simple_scene(move |scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;

            // `split` kipinde K ayrı mesh: aynı geometri, ayrı köşe tamponu. `BatchKey`'in ilk
            // alanı tampon kimliği olduğu için bu tam olarak K parti demek — geometri
            // değişmediğinden aradaki fark yalnız parti maliyeti.
            let meshes: Vec<Mesh> = if mode == "split" {
                (0..batches)
                    .map(|_| AssetManager::create_cube(device))
                    .collect()
            } else {
                vec![AssetManager::create_cube(device)]
            };

            let mut rng = 0x9E37_79B9u32;
            let mut roll = move |low: f32, high: f32| {
                rng ^= rng << 13;
                rng ^= rng >> 17;
                rng ^= rng << 5;
                low + (rng as f32 / u32::MAX as f32) * (high - low)
            };

            for i in 0..cubes {
                let position = shell_point(i, cubes) * SHELL_RADIUS;
                let albedo = if mode == "vary" {
                    // Örnek başına malzeme verisi. `BatchKey`'de olmadığı için partiyi
                    // bölmemesi gerekiyor; ölçtüğümüz şey bu.
                    Vec4::new(roll(0.2, 1.0), roll(0.2, 1.0), roll(0.2, 1.0), 1.0)
                } else {
                    Vec4::new(0.65, 0.68, 0.75, 1.0)
                };
                scene.world.spawn_bundle((
                    Transform::new(position).with_scale(Vec3::splat(0.35)),
                    GlobalTransform::default(),
                    meshes[i % meshes.len()].clone(),
                    Material::new(white.clone()).with_pbr(albedo, 0.55, 0.0),
                    MeshRenderer::new(),
                ));
            }

            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.7) * Quat::from_rotation_x(-0.5),
                intensity: 2.4,
                ..Default::default()
            });

            scene.world.insert_resource(Bench {
                cubes,
                mode,
                batches: if mode == "split" { batches } else { 1 },
                angle: 0.0,
                frame: 0,
            });
            scene.spawn_camera(state, Vec3::new(0.0, 0.0, SHELL_RADIUS * 1.8), Vec3::ZERO);
        })
        .add_plugin(AnalysisPlugin::new())
        .add_update_system(orbit_and_measure.in_phase(Phase::Update))
        .set_ui(|world, _state, ctx| {
            let Some(bench) = world.get_resource::<Bench>().map(|b| b.clone()) else {
                return;
            };
            let stats = world
                .get_resource::<Analyzer>()
                .and_then(|a| a.stats(METRIC_FRAME_MS));
            gizmo::egui::Area::new("cubes".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    // Küpler parlak: çerçevesiz bir panel okunmuyor.
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_min_width(430.0);
                    ui.heading("Varlık başına çizim maliyeti");
                    ui.label(format!(
                        "küp {} · kip {} · parti {}",
                        bench.cubes, bench.mode, bench.batches
                    ));
                    ui.separator();
                    match stats {
                        Some(s) => {
                            ui.label(format!(
                                "kare: ort {:.2} ms · p50 {:.2} · p95 {:.2} · p99 {:.2}",
                                s.mean, s.p50, s.p95, s.p99
                            ));
                            ui.label(format!(
                                "min {:.2} · maks {:.2} · örnek {}",
                                s.min, s.max, s.count
                            ));
                            ui.label(format!("~{:.0} FPS", 1000.0 / s.mean.max(0.001)));
                        }
                        None => {
                            ui.label("ölçüm birikiyor…");
                        }
                    }
                    ui.separator();
                    ui.label("renk/pürüz/metaliklik BatchKey'de değil — partiyi bölmez");
                    ui.label("bölen şey: mesh, doku, yönlendirme bayrakları, gölge, görünürlük");
                    ui.label("oyun yolu çizim çağrısı sayısı yayınlamıyor (RenderStats editörde)");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Kamerayı sabit adımla döndürür ve kare süresini deftere yazar.
///
/// Dönüş `dt` ile değil sabit bir adımla. Kare süresine bağlı bir kamera, ölçümün kendisini
/// ölçüme sokar.
fn orbit_and_measure(
    mut cameras: Query<(Mut<Transform>, With<Camera>)>,
    mut bench: ResMut<Bench>,
    mut analyzer: ResMut<Analyzer>,
    time: Res<Time>,
) {
    bench.angle += CAMERA_STEP;
    bench.frame += 1;
    let angle = bench.angle;

    let eye = Vec3::new(
        angle.cos() * SHELL_RADIUS * 1.8,
        (angle * 0.37).sin() * SHELL_RADIUS * 0.5,
        angle.sin() * SHELL_RADIUS * 1.8,
    );
    for (_entity, (mut transform, _)) in cameras.iter_mut() {
        transform.position = eye;
    }

    analyzer.sample(METRIC_FRAME_MS, (time.dt() * 1000.0) as f64);

    if std::env::var("GIZMO_CUBES_SELFTEST").is_ok() && bench.frame.is_multiple_of(PROBE_PERIOD) {
        if let Some(s) = analyzer.stats(METRIC_FRAME_MS) {
            gizmo::gizmo_log!(
                Info,
                "küp {} · kip {} · parti {} · ort {:.2} ms · p95 {:.2} ms · ~{:.0} FPS",
                bench.cubes,
                bench.mode,
                bench.batches,
                s.mean,
                s.p95,
                1000.0 / s.mean.max(0.001)
            );
        }
    }
}

/// `i`. noktanın küre kabuğundaki yeri — altın açı sarmalı, yani düzgün dağılım.
fn shell_point(i: usize, total: usize) -> Vec3 {
    let golden = PI * (3.0 - 5.0f32.sqrt());
    let y = 1.0 - (i as f32 / (total.max(2) - 1) as f32) * 2.0;
    let radius = (1.0 - y * y).max(0.0).sqrt();
    let theta = golden * i as f32;
    Vec3::new(theta.cos() * radius, y, theta.sin() * radius)
}
