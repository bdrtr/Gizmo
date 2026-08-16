#![deny(clippy::undocumented_unsafe_blocks)]
//! (`undocumented_unsafe_blocks` is a RATCHET: this crate carries no `unsafe` block without a
//! `// SAFETY:` line stating why it is sound, and the lint keeps it that way. Every crate in the
//! workspace except `gizmo-core` is at zero and denies it; `gizmo-core`'s ECS internals are the
//! measured remainder — see docs/ENGINE.md.)
//! # gizmo-analysis
//!
//! Gizmo motoru için **derin gözlemlenebilirlik / analiz modülü**. Amaç: analiz modülüyle
//! çalışan bir motorun **en ufak ayrıntısı bile** — her frame'in süresi ve alt-span'leri,
//! ECS'in archetype/component/bellek dökümü, fizik metrikleri, herhangi bir isimlendirilmiş
//! sayaç/gösterge/ölçüm — kaydedilebilir, üzerinde istatistik alınabilir ve dışa aktarılabilir
//! olsun.
//!
//! ## Bileşenler
//! - [`Analyzer`] — merkezi resource. Her frame bir [`FrameSnapshot`] üretir, ring-buffer
//!   geçmişi tutar, tüm sayısal değerleri [`MetricStore`]'a besler.
//! - [`MetricStore`] / [`Stats`] — sayaç/gösterge/ölçüm serileri + min/max/ortalama/std/p50/p95/p99.
//! - [`Collector`] — genişletme noktası. Yeni bir alt-sistemi analiz edilebilir yapmak için
//!   bir collector yazıp kaydet. Yerleşik: [`EcsCollector`] (+ `physics` özelliğiyle
//!   `PhysicsCollector`).
//! - Dışa aktarım: [`Analyzer::report_text`], [`Analyzer::to_json`], [`Analyzer::to_csv`],
//!   [`Analyzer::to_chrome_trace`] (Perfetto/`chrome://tracing`).
//!
//! ## Hızlı kullanım (headless)
//! ```
//! use gizmo_analysis::{profile_scope, Analyzer, FrameProfiler};
//! use gizmo_core::{Schedule, World};
//!
//! let mut world = World::new();
//! world.insert_resource(FrameProfiler::new());
//! let mut schedule = Schedule::new();          // gerçek motorda sistemlerle dolu
//! let mut analyzer = Analyzer::new();          // EcsCollector yerleşik; span'leri
//!                                              // world'deki FrameProfiler'dan okur
//!
//! for _ in 0..3 {
//!     {
//!         profile_scope!(&world, "simulate");  // sistemlerin ölçtüğü span'ler
//!     }
//!     schedule.run(&mut world, 1.0 / 60.0);    // motor bir frame ilerler (+ end_frame)
//!     analyzer.collect(&world);                // o frame'i analiz et
//! }
//!
//! // Her `collect` bir frame'i kaydeder; span'ler `span.<ad>` metrik serisine de düşer.
//! assert_eq!(analyzer.frame(), 3);
//! assert_eq!(analyzer.stats("span.simulate").unwrap().count, 3);
//! assert!(analyzer.report_text().contains("Gizmo Analysis"));
//!
//! // Alev-grafiği: `std::fs::write("trace.json", trace)` → chrome://tracing / Perfetto.
//! let trace = analyzer.to_chrome_trace();
//! assert!(trace.contains("\"name\":\"simulate\""));
//! ```
//!
//! `app` özelliğiyle [`AnalysisPlugin`] tüm bunları App/Plugin schedule'ına otomatik bağlar.

mod analyzer;
pub mod collector;
pub mod metrics;
mod report;
pub mod snapshot;
mod util;

pub use analyzer::{AnalysisConfig, Analyzer};
pub use collector::{Collector, EcsCollector};
pub use metrics::{MetricKind, MetricSeries, MetricStore, Stats};
pub use snapshot::{FrameSnapshot, SpanSample};

// Çekirdek introspection tiplerini kolaylık için yeniden dışa aktar.
pub use gizmo_core::world::{short_type_name, ArchetypeSummary, ComponentSummary, WorldStats};
pub use gizmo_core::FrameProfiler;

#[cfg(feature = "app")]
mod plugin;
#[cfg(feature = "app")]
pub use plugin::{AnalysisCollectSystem, AnalysisPlugin};

#[cfg(feature = "physics")]
mod physics;
#[cfg(feature = "physics")]
pub use physics::PhysicsCollector;

#[cfg(feature = "trace")]
pub mod trace;

#[cfg(feature = "egui")]
pub mod panel;

use gizmo_core::world::World;

/// Bir profiling span'i başlat (FrameProfiler resource'u varsa). Genelde [`profile_scope!`]
/// makrosunu kullanın.
pub fn begin_scope(world: &World, name: &'static str) {
    if let Some(mut p) = world.get_resource_mut::<FrameProfiler>() {
        p.begin_scope(name);
    }
}

/// Bir profiling span'ini kapat.
pub fn end_scope(world: &World, name: &'static str) {
    if let Some(mut p) = world.get_resource_mut::<FrameProfiler>() {
        p.end_scope(name);
    }
}

/// RAII span zamanlayıcısı — drop edilince span kapanır. FrameProfiler kaynağını yalnız
/// başlangıç ve bitişte kısa süre kilitler (uzun tutmaz → paralel sistemlerde güvenli).
pub struct ScopeTimer<'w> {
    world: &'w World,
    name: &'static str,
}

impl Drop for ScopeTimer<'_> {
    fn drop(&mut self) {
        end_scope(self.world, self.name);
    }
}

/// Bir span aç ve RAII zamanlayıcı döndür.
pub fn scope<'w>(world: &'w World, name: &'static str) -> ScopeTimer<'w> {
    begin_scope(world, name);
    ScopeTimer { world, name }
}

/// `profile_scope!(world, "isim");` — mevcut blok boyunca süreyi ölçer.
#[macro_export]
macro_rules! profile_scope {
    ($world:expr, $name:expr) => {
        let _gizmo_analysis_scope = $crate::scope($world, $name);
    };
}
