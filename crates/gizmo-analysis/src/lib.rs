#![deny(clippy::undocumented_unsafe_blocks)]
//! (`undocumented_unsafe_blocks` is a RATCHET: this crate carries no `unsafe` block without a
//! `// SAFETY:` line stating why it is sound, and the lint keeps it that way. Every crate in the
//! workspace except `gizmo-core` is at zero and denies it; `gizmo-core`'s ECS internals are the
//! measured remainder — see docs/ENGINE.md.)
//! # gizmo-analysis
//!
//! **Deep observability and analysis** for the Gizmo engine. The goal: with this module running,
//! **even the smallest detail** of an engine — every frame's duration and its sub-spans, the
//! ECS's archetype/component/memory breakdown, physics metrics, any named
//! counter/gauge/measurement — can be recorded, have statistics taken over it, and be exported.
//!
//! ## The pieces
//! - [`Analyzer`] — the central resource. Produces a [`FrameSnapshot`] every frame, keeps a
//!   ring-buffer history, and feeds every numeric value into the [`MetricStore`].
//! - [`MetricStore`] / [`Stats`] — counter/gauge/sample series plus
//!   min/max/mean/std/p50/p95/p99.
//! - [`Collector`] — the extension point. To make a new subsystem analysable, write a collector
//!   and register it. Built in: [`EcsCollector`] (plus `PhysicsCollector` with the `physics`
//!   feature).
//! - Export: [`Analyzer::report_text`], [`Analyzer::to_json`], [`Analyzer::to_csv`],
//!   [`Analyzer::to_chrome_trace`] (Perfetto / `chrome://tracing`).
//!
//! ## Quick start (headless)
//! ```
//! use gizmo_analysis::{profile_scope, Analyzer, FrameProfiler};
//! use gizmo_core::{Schedule, World};
//!
//! let mut world = World::new();
//! world.insert_resource(FrameProfiler::new());
//! let mut schedule = Schedule::new();          // full of systems in a real engine
//! let mut analyzer = Analyzer::new();          // EcsCollector is built in; spans are
//!                                              // read from the world's FrameProfiler
//!
//! for _ in 0..3 {
//!     {
//!         profile_scope!(&world, "simulate");  // the spans the systems measure
//!     }
//!     schedule.run(&mut world, 1.0 / 60.0);    // the engine advances one frame (+ end_frame)
//!     analyzer.collect(&world);                // analyse that frame
//! }
//!
//! // Every `collect` records one frame; spans also land in the `span.<name>` metric series.
//! assert_eq!(analyzer.frame(), 3);
//! assert_eq!(analyzer.stats("span.simulate").unwrap().count, 3);
//! assert!(analyzer.report_text().contains("Gizmo Analysis"));
//!
//! // Flame chart: `std::fs::write("trace.json", trace)` → chrome://tracing / Perfetto.
//! let trace = analyzer.to_chrome_trace();
//! assert!(trace.contains("\"name\":\"simulate\""));
//! ```
//!
//! With the `app` feature, [`AnalysisPlugin`] wires all of this into the App/Plugin schedule
//! automatically.

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

/// Starts a profiling span (if there is a FrameProfiler resource). Normally you want the
/// [`profile_scope!`] macro instead.
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

/// An RAII span timer — the span closes when it is dropped. It locks the FrameProfiler resource
/// only briefly, at the start and at the end (never holds it, so it is safe under parallel
/// systems).
pub struct ScopeTimer<'w> {
    world: &'w World,
    name: &'static str,
}

impl Drop for ScopeTimer<'_> {
    fn drop(&mut self) {
        end_scope(self.world, self.name);
    }
}

/// Opens a span and returns the RAII timer.
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
