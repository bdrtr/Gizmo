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
        // Frame profiler yoksa ekle (Analyzer span'leri buradan okur; schedule.run
        // her frame end_frame çağırır).
        if app.world.get_resource::<FrameProfiler>().is_none() {
            app.world.insert_resource(FrameProfiler::new());
        }

        let mut analyzer = Analyzer::with_config(self.config.clone());
        #[cfg(feature = "physics")]
        analyzer.register_collector(Box::new(crate::PhysicsCollector));

        app.world.insert_resource(analyzer);
        app.schedule.add_system(AnalysisCollectSystem);
    }
}

/// The system that calls `Analyzer::collect` every frame.
///
/// NOTE: the schedule calls `FrameProfiler::end_frame` AFTER all systems, so the spans this
/// system sees belong to the PREVIOUS frame (the ECS state is current). If you need zero lag on
/// spans, call `analyzer.collect(&world)` by hand after `schedule.run` instead of using the
/// plugin (see the `headless_analysis` example).
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
