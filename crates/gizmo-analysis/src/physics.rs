//! Fizik metrik toplayıcısı (`physics` özelliği).
//!
//! `PhysicsWorld` resource'undan `PhysicsMetrics`'i (aşama zamanlamaları + sayımlar) okur.

use crate::collector::Collector;
use crate::snapshot::FrameSnapshot;
use gizmo_core::world::World;
use gizmo_physics_rigid::PhysicsWorld;

/// Rigid-body fizik dünyasının metriklerini toplar.
#[derive(Debug, Clone, Default)]
pub struct PhysicsCollector;

impl Collector for PhysicsCollector {
    fn name(&self) -> &'static str {
        "physics"
    }

    fn collect(&mut self, world: &World, out: &mut FrameSnapshot) {
        let Some(pw) = world.get_resource::<PhysicsWorld>() else {
            return;
        };
        let m = &pw.metrics;
        for (name, value) in [
            ("bodies", m.body_count as f64),
            ("sleeping", m.sleeping_count as f64),
            ("contacts", m.contact_count as f64),
            ("islands", m.island_count as f64),
            ("broadphase_ms", m.broadphase_ms as f64),
            ("narrowphase_ms", m.narrowphase_ms as f64),
            ("solver_ms", m.solver_ms as f64),
            ("integration_ms", m.integration_ms as f64),
            ("total_ms", m.total_ms() as f64),
        ] {
            out.push_metric("physics", name, value);
        }
        // Uyku oranı — enerji/yerleşme analizi için pratik türetilmiş metrik.
        if m.body_count > 0 {
            out.push_metric(
                "physics",
                "sleep_ratio",
                m.sleeping_count as f64 / m.body_count as f64,
            );
        }
    }
}
