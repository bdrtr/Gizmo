//! # Bevy'nin `diagnostics/custom_diagnostic` örneğinin Gizmo Engine portu
//!
//! Kendi ölçümünü kaydetmek: bir sistem her karede bir sayı üretiyor, motor onu biriktiriyor,
//! ve sonuç istatistikleriyle birlikte okunabiliyor. Bevy'de bu `Diagnostic` +
//! `LogDiagnosticsPlugin`; Gizmo'da `gizmo-analysis` crate'i.
//!
//! **Bu, o crate'i gösteren ilk demo** — bugüne kadar hiçbir demo ona dokunmuyordu, ve
//! `demo`'nun özellik listesinde bile yoktu (bu port `analysis` özelliğini açıyor).
//!
//! ## Motorun verdiği, Bevy'nin verdiğinden geniş
//!
//! Bevy'nin `Diagnostic`'i bir ölçüm dizisi tutar ve ortalamasını verir. Gizmo'nun
//! [`Analyzer`]'ı üç ayrı seri türü tanıyor — **sayaç** (`counter_add`), **gösterge** (`gauge`)
//! ve **örnek** (`sample`) — ve her biri için min/maks/ortalama/std/p50/p95/p99 hesaplıyor. Demo
//! üçünü de kullanıyor:
//!
//!   * `demo.system_iterations` — sayaç: ölçüm sistemi kaç kez koştu (Bevy'nin örneğinin
//!     ölçtüğü şeyin ta kendisi).
//!   * `demo.alive` — gösterge: şu anda kaç küp var.
//!   * `demo.frame_ms` — örnek: karenin süresi (istatistikler bunun üstünde okunuyor).
//!
//! Motorun `profile_scope!` makrosu bir `&World` istiyor; ECS sistemleri dünyayı doğrudan
//! almadığı için bu demo kendi süresini elle ölçüp `span.demo_measure` serisine yazıyor — aynı
//! deftere düşen, elle tutulmuş bir span. (`Analyzer::to_chrome_trace()` gerçek span'leri
//! Perfetto'ya açar; onun için `profile_scope!`'un dünyayı gördüğü bir yer gerekir, örneğin
//! `set_render` kapanışı.)
//!
//! Bevy'nin konsola basan eklentisinin karşılığı burada iki yerde: HUD (canlı istatistikler) ve
//! **F3** — `Analyzer::report_text()`'i günlüğe basar.
//!
//! ## Kontroller
//!   * **F3** — analiz raporunu günlüğe yaz · **Sağ-tık + fare / WASDQE** — kamera

use gizmo::analysis::{AnalysisPlugin, Analyzer};
use gizmo::core::input::Input;
use gizmo::core::query::{Query, With};
use gizmo::core::system::{IntoSystemConfig, Phase, Res, ResMut};
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Ölçülen sahnedeki küplerin işareti.
#[derive(Clone, Copy)]
struct Spinner;
gizmo::core::impl_component!(Spinner);

/// Demonun kendi defteri — sahnede kaç küp var.
#[derive(Default, Clone, Copy)]
struct Census {
    alive: usize,
}
gizmo::core::impl_component!(Census);

/// Ölçümlerin adları — Bevy'nin `DiagnosticPath`'inin karşılığı, ve aynı gerekçeyle sabit:
/// seriyi yazan ile okuyan aynı dizeyi kullanmalı.
const METRIC_ITERATIONS: &str = "demo.system_iterations";
const METRIC_ALIVE: &str = "demo.alive";
const METRIC_FRAME_MS: &str = "demo.frame_ms";

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Bevy Custom Diagnostic", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let cube = AssetManager::create_cube(&scene.renderer.device);

            for i in 0..12 {
                let angle = i as f32 / 12.0 * std::f32::consts::TAU;
                scene.world.spawn_bundle((
                    Transform::new(Vec3::new(angle.cos() * 2.5, 0.6, angle.sin() * 2.5))
                        .with_scale(Vec3::splat(0.3)),
                    GlobalTransform::default(),
                    cube.clone(),
                    Material::new(white.clone()).with_pbr(
                        Vec4::new(0.35, 0.55, 0.9, 1.0),
                        0.5,
                        0.0,
                    ),
                    MeshRenderer::new(),
                    Spinner,
                    Spin::new(Vec3::Y, 1.2),
                ));
            }

            scene.world.spawn_bundle((
                Transform::new(Vec3::ZERO),
                GlobalTransform::default(),
                AssetManager::create_plane(&scene.renderer.device, 12.0),
                Material::new(white).with_pbr(Vec4::new(0.1, 0.11, 0.13, 1.0), 1.0, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.8) * Quat::from_rotation_x(-0.6),
                intensity: 2.5,
                ..Default::default()
            });

            scene.world.insert_resource(Census { alive: 12 });
            scene.spawn_camera(state, Vec3::new(0.0, 3.5, 7.0), Vec3::new(0.0, 0.5, 0.0));
        })
        // Analiz altyapısı: FrameProfiler + Analyzer + her kare toplama.
        .add_plugin(AnalysisPlugin::new())
        .add_plugin(SpinPlugin)
        .add_update_system(measure.in_phase(Phase::Update))
        .add_update_system(report_on_key.in_phase(Phase::Update))
        .set_ui(|world, _state, ctx| {
            let Some(analyzer) = world.get_resource::<Analyzer>() else {
                return;
            };
            let frame_stats = analyzer.stats(METRIC_FRAME_MS);
            let alive = analyzer.stats(METRIC_ALIVE);
            // Sayaçta `stats().last` KARE BAŞINA artıştır; kümülatif toplam serinin
            // kendisinde durur (`MetricSeries::total`). İkisini de göstermek, sayaç ile
            // gösterge/örnek arasındaki farkı görünür kılıyor.
            let iterations = analyzer.metrics().get(METRIC_ITERATIONS);
            let span = analyzer.stats("span.demo_measure");
            gizmo::egui::Area::new("diag".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    ui.heading("Kendi ölçümlerin");
                    ui.label(format!("analiz edilen kare: {}", analyzer.frame()));
                    ui.separator();
                    if let Some(s) = frame_stats {
                        ui.label(format!("{METRIC_FRAME_MS} (örnek serisi)"));
                        ui.label(format!(
                            "  ort {:.2} ms · p50 {:.2} · p95 {:.2} · p99 {:.2}",
                            s.mean, s.p50, s.p95, s.p99
                        ));
                        ui.label(format!(
                            "  min {:.2} · maks {:.2} · std {:.2} · adet {}",
                            s.min, s.max, s.stddev, s.count
                        ));
                    }
                    if let Some(s) = alive {
                        ui.label(format!("{METRIC_ALIVE} (gösterge): son {:.0}", s.last));
                    }
                    if let Some(series) = iterations {
                        let stats = series.stats();
                        ui.label(format!(
                            "{METRIC_ITERATIONS} (sayaç): toplam {:.0} · kare başına {:.0}",
                            series.total(),
                            stats.last
                        ));
                    }
                    if let Some(s) = span {
                        ui.label(format!(
                            "span.demo_measure: ort {:.3} ms · maks {:.3}",
                            s.mean, s.max
                        ));
                    }
                    ui.separator();
                    ui.label("F3 — report_text()'i günlüğe yaz");
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Bevy'nin `my_system`'i: her karede ölçümleri kaydeder.
///
/// Üç seri türü de burada: sayaç birikir, gösterge o anki değeri tutar, örnek dağılımı taşır.
fn measure(
    spinners: Query<(&Transform, With<Spinner>)>,
    mut census: ResMut<Census>,
    mut analyzer: ResMut<Analyzer>,
    time: Res<Time>,
) {
    // Sistemin kendi süresi: `profile_scope!` bir `&World` ister, sistem onu almaz — bu yüzden
    // elle ölçülüp aşağıda aynı deftere yazılıyor.
    let started = std::time::Instant::now();

    census.alive = spinners.iter().count();

    // Bevy'nin örneği tam olarak bunu ölçüyor: sistemin kaç kez koştuğu.
    analyzer.counter_add(METRIC_ITERATIONS, 1.0);
    analyzer.gauge(METRIC_ALIVE, census.alive as f64);
    analyzer.sample(METRIC_FRAME_MS, (time.dt() * 1000.0) as f64);
    analyzer.sample(
        "span.demo_measure",
        started.elapsed().as_secs_f64() * 1000.0,
    );
}

/// F3: raporu günlüğe basar — Bevy'nin `LogDiagnosticsPlugin`'inin karşılığı.
fn report_on_key(analyzer: Res<Analyzer>, input: Res<Input>) {
    if !input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::F3 as u32) {
        return;
    }
    gizmo::gizmo_log!(Info, "{}", analyzer.report_text());
}
