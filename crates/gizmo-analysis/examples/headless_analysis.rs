//! `gizmo-analysis` headless demo.
//!
//! Bir ECS dünyasını elle birkaç yüz frame ilerletir, her frame'i profiling scope'larıyla
//! ölçer ve `Analyzer` ile analiz eder. Sonunda metin raporu basar; JSON/CSV/Chrome-trace
//! dosyalarını geçici dizine yazar.
//!
//! Çalıştır:  `cargo run -p gizmo-analysis --example headless_analysis`

use gizmo_analysis::{profile_scope, AnalysisConfig, Analyzer};
use gizmo_core::world::World;
use gizmo_core::FrameProfiler;
use std::time::Duration;

#[derive(Clone)]
struct Position {
    _x: f32,
    _y: f32,
    _z: f32,
}
#[derive(Clone)]
struct Velocity {
    _v: [f32; 3],
}
#[derive(Clone)]
struct Health {
    _hp: f32,
}
#[derive(Clone)]
struct Name(#[allow(dead_code)] String);

impl gizmo_core::Component for Position {}
impl gizmo_core::Component for Velocity {}
impl gizmo_core::Component for Health {}
impl gizmo_core::Component for Name {}

fn main() {
    let mut world = World::new();
    world.insert_resource(FrameProfiler::new());

    let mut analyzer = Analyzer::with_config(AnalysisConfig {
        history_frames: 240,
        metric_history: 2048,
        detailed_archetypes: true,
        ..Default::default()
    });

    let frames = 180u64;
    println!("▶ {frames} frame simüle ediliyor...\n");

    for frame in 0..frames {
        // ── Spawn aşaması ────────────────────────────────────────────────────
        // Bu blok dünyayı `&mut` ile değiştirdiğinden RAII `profile_scope!` (dünyayı
        // `&World` olarak tutar) yerine açık begin/end kullanıyoruz. Gerçek motor
        // sistemleri dünyayı `&World` alıp iç-değişebilirlikle değiştirdiğinden orada
        // doğrudan `profile_scope!` kullanılabilir.
        gizmo_analysis::begin_scope(&world, "spawn");
        {
            let batch = 3 + (frame % 5) as usize;
            for i in 0..batch {
                let e = world.spawn();
                world.add_component(e, Position { _x: i as f32, _y: 0.0, _z: frame as f32 });
                // Değişken archetype'lar → ilginç bir tablo.
                if i % 2 == 0 {
                    world.add_component(e, Velocity { _v: [1.0, 0.0, 0.0] });
                }
                if i % 3 == 0 {
                    world.add_component(e, Health { _hp: 100.0 });
                }
                if i % 4 == 0 {
                    world.add_component(e, Name(format!("ent-{frame}-{i}")));
                }
            }
            analyzer.counter_add("entities_spawned", batch as f64);
            std::thread::sleep(Duration::from_micros(120));
        }
        gizmo_analysis::end_scope(&world, "spawn");

        // ── Simülasyon aşaması (iç içe span) ─────────────────────────────────
        {
            profile_scope!(&world, "simulate");
            {
                profile_scope!(&world, "simulate.integrate");
                std::thread::sleep(Duration::from_micros(300));
            }
            {
                profile_scope!(&world, "simulate.ai");
                std::thread::sleep(Duration::from_micros(150));
            }
            // Örnek bir ölçüm metriği (ör. çözücü iterasyonları).
            let iters = 8.0 + (frame % 4) as f64;
            analyzer.sample("solver_iterations", iters);
        }

        // ── Anlık gösterge ───────────────────────────────────────────────────
        analyzer.gauge("live_entities", world.entity_count() as f64);

        // Frame'i kapat (FrameProfiler span'leri kaydeder) ve analiz et.
        if let Some(mut p) = world.get_resource_mut::<FrameProfiler>() {
            p.end_frame();
        }
        analyzer.collect(&world);
    }

    // ── Sonuç ────────────────────────────────────────────────────────────────
    println!("{}", analyzer.report_text());

    if let Some(s) = analyzer.stats("solver_iterations") {
        println!(
            "solver_iterations  → mean {:.2} | p95 {:.2} | max {:.2} (n={})",
            s.mean, s.p95, s.max, s.count
        );
    }
    if let Some(s) = analyzer.stats("span.simulate") {
        println!(
            "span.simulate      → mean {:.3} ms | p99 {:.3} ms",
            s.mean, s.p99
        );
    }
    if let Some(series) = analyzer.metrics().get("entities_spawned") {
        println!(
            "entities_spawned   → total {} across {} frames",
            series.total() as u64,
            series.len()
        );
    }

    // Dışa aktarım dosyaları.
    let dir = std::env::temp_dir();
    let json = dir.join("gizmo_analysis_report.json");
    let csv = dir.join("gizmo_analysis_timeseries.csv");
    let trace = dir.join("gizmo_analysis_trace.json");
    let _ = std::fs::write(&json, analyzer.to_json());
    let _ = std::fs::write(&csv, analyzer.to_csv());
    let _ = std::fs::write(&trace, analyzer.to_chrome_trace());

    println!("\n📤 Dışa aktarıldı:");
    println!("   JSON  : {}", json.display());
    println!("   CSV   : {}", csv.display());
    println!(
        "   Trace : {}  (chrome://tracing veya ui.perfetto.dev ile aç)",
        trace.display()
    );
}
