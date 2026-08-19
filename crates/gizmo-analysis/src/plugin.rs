//! App/Plugin integration (the `app` feature).
//!
//! [`AnalysisPlugin`] adds the `FrameProfiler` and `Analyzer` resources and puts a system into
//! the schedule that calls `Analyzer::collect` every frame.

use crate::analyzer::{AnalysisConfig, Analyzer};
use gizmo_app::Plugin;
use gizmo_core::system::{AccessInfo, System};
use gizmo_core::world::World;
use gizmo_core::FrameProfiler;

/// The plugin that wires the analysis machinery into an App.
#[derive(Debug, Clone, Default)]
pub struct AnalysisPlugin {
    /// The configuration the installed [`Analyzer`](crate::Analyzer) starts with.
    pub config: AnalysisConfig,
}

impl AnalysisPlugin {
    /// The plugin with default settings — collection on, the default history depth.
    pub fn new() -> Self {
        Self::default()
    }
    /// The plugin with a configuration of your own.
    pub fn with_config(config: AnalysisConfig) -> Self {
        Self { config }
    }
}

impl Plugin for AnalysisPlugin {
    fn build(&self, app: &mut dyn gizmo_app::AppLike) {
        let app = app.parts_mut();
        // Frame profiler yoksa ekle — Analyzer span'leri buradan okuyor. Kareyi BİTİREN şey
        // çalışma zamanının döngüsü (pencereli/headless/sunucu); `Schedule::run` değil, çünkü bir
        // schedule koşusu bir kare değil (pencereli bir kare iki schedule koşuyor, biri 0..N kez).
        if app.world.get_resource::<FrameProfiler>().is_none() {
            app.world.insert_resource(FrameProfiler::new());
        }

        let mut analyzer = Analyzer::with_config(self.config.clone());
        #[cfg(feature = "physics")]
        analyzer.register_collector(Box::new(crate::PhysicsCollector));

        app.world.insert_resource(analyzer);
        // **Per RENDERED frame, not per fixed step.** `AppParts::schedule` is the fixed-timestep
        // schedule, which runs 0..N times per frame — so the analyzer was sampling physics steps
        // and calling them frames: two rows for a frame that stepped twice, none for a frame that
        // stepped none, and every "frame" number it reported was that count rather than the
        // engine's. `update_schedule` runs exactly once per frame, which is what "collect every
        // frame" was always supposed to mean.
        app.update_schedule.add_system(AnalysisCollectSystem);
    }
}

/// The system that calls `Analyzer::collect` every frame.
///
/// Registered on the **update** schedule, so it runs once per rendered frame. It used to be on the
/// fixed-timestep schedule, which runs `0..N` times per frame — the analyzer counted physics steps
/// as frames.
///
/// NOTE: the runtime's loop ends the frame AFTER its schedules have run, so the spans this system
/// sees belong to the PREVIOUS frame (the ECS state is current). If you need zero lag on spans,
/// call `analyzer.collect(&world)` by hand after your own `end_frame` instead of using the plugin
/// (see the `headless_analysis` example).
pub struct AnalysisCollectSystem;

impl System for AnalysisCollectSystem {
    fn access_info(&self) -> AccessInfo {
        let mut info = AccessInfo::new();
        // Analyzer tüm dünyayı okuduğundan güvenli taraf: exclusive.
        info.is_exclusive = true;
        info
    }

    fn run(&mut self, world: &World, _dt: f32) {
        if let Some(mut analyzer) = world.get_resource_mut::<Analyzer>() {
            analyzer.collect(world);
        }
    }
}

#[cfg(test)]
mod schedule_choice_tests {
    /// Source with its comments removed: the paragraph explaining the choice names both
    /// schedules, and a positive `contains` over raw source would be satisfied by the prose.
    fn code_only(src: &str) -> String {
        src.lines()
            .map(|line| {
                let bytes = line.as_bytes();
                let mut end = line.len();
                let mut i = 0;
                while i + 1 < bytes.len() {
                    if bytes[i] == b'/' && bytes[i + 1] == b'/' && (i == 0 || bytes[i - 1] != b':') {
                        end = i;
                        break;
                    }
                    i += 1;
                }
                &line[..end]
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// **The collector belongs on the per-frame schedule.**
    ///
    /// `AppParts::schedule` is the fixed-timestep one and runs `0..N` times per rendered frame, so
    /// registering there made the analyzer sample physics steps and call them frames: two rows for
    /// a frame that stepped twice, none for a frame that stepped none. A behavioural test would
    /// need a runtime loop; this pins the registration, comments cut.
    #[test]
    fn the_collector_is_registered_on_the_per_frame_schedule() {
        let code: String = code_only(include_str!("plugin.rs"))
            .split("#[cfg(test)]")
            .next()
            .unwrap_or("")
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();

        assert!(
            code.contains("app.update_schedule.add_system(AnalysisCollectSystem)"),
            "the collector must be on the update schedule — one run per rendered frame"
        );
        assert!(
            !code.contains("app.schedule.add_system(AnalysisCollectSystem)"),
            "it is back on the fixed schedule, which runs 0..N times per frame"
        );
    }
}
