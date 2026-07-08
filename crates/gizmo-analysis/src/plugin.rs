//! App/Plugin entegrasyonu (`app` özelliği).
//!
//! [`AnalysisPlugin`] `FrameProfiler` + `Analyzer` kaynaklarını ekler ve her frame
//! `Analyzer::collect` çağıran bir sistemi schedule'a koyar.

use crate::analyzer::{AnalysisConfig, Analyzer};
use gizmo_app::{App, Plugin};
use gizmo_core::system::{AccessInfo, System};
use gizmo_core::world::World;
use gizmo_core::FrameProfiler;

/// Analiz altyapısını App'e bağlayan plugin.
#[derive(Debug, Clone, Default)]
pub struct AnalysisPlugin {
    pub config: AnalysisConfig,
}

impl AnalysisPlugin {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn with_config(config: AnalysisConfig) -> Self {
        Self { config }
    }
}

impl<State: 'static> Plugin<State> for AnalysisPlugin {
    fn build(&self, app: &mut App<State>) {
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

/// Her frame `Analyzer::collect` çağıran sistem.
///
/// NOT: schedule `FrameProfiler::end_frame`'i tüm sistemlerden SONRA çağırdığından,
/// bu sistemin gördüğü span'ler BİR ÖNCEKİ frame'e aittir (ECS durumu günceldir).
/// Span'lerde sıfır-gecikme isteniyorsa, plugin yerine `schedule.run` sonrası elle
/// `analyzer.collect(&world)` çağırın (bkz. `headless_analysis` örneği).
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
