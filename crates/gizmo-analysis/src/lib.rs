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
//! ```ignore
//! use gizmo_analysis::Analyzer;
//! let mut analyzer = Analyzer::new();          // FrameProfiler + EcsCollector yerleşik
//! world.insert_resource(gizmo_core::FrameProfiler::new());
//! loop {
//!     schedule.run(&mut world, dt);            // motor bir frame ilerler
//!     analyzer.collect(&world);                // o frame'i analiz et
//! }
//! println!("{}", analyzer.report_text());
//! std::fs::write("trace.json", analyzer.to_chrome_trace()).unwrap();
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
