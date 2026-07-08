//! Collector'lar — her frame dünyayı okuyup snapshot'a metrik ekleyen eklentiler.
//!
//! Yeni bir alt-sistemi analiz edilebilir yapmak için tek yapılması gereken bir
//! `Collector` yazıp `Analyzer`'a kaydetmektir. Böylece "en ufak ayrıntı" sınırsızca
//! genişletilebilir kalır.

use crate::snapshot::FrameSnapshot;
use gizmo_core::world::World;

/// Her frame çağrılan analiz toplayıcısı.
pub trait Collector: Send + Sync {
    /// Grup adı (snapshot metrik grubu olarak da kullanılır).
    fn name(&self) -> &'static str;
    /// Dünyayı oku, `out`'a metrik/ayrıntı ekle. Dünyayı DEĞİŞTİRME.
    fn collect(&mut self, world: &World, out: &mut FrameSnapshot);
}

/// Yerleşik ECS toplayıcısı — entity/archetype/component/bellek durumunu çıkarır.
#[derive(Debug, Clone)]
pub struct EcsCollector {
    /// Ayrıntılı per-archetype tabloyu topla (biraz daha ağır). Kapalıysa yalnız üst-düzey.
    pub detailed_archetypes: bool,
}

impl Default for EcsCollector {
    fn default() -> Self {
        Self {
            detailed_archetypes: true,
        }
    }
}

impl Collector for EcsCollector {
    fn name(&self) -> &'static str {
        "ecs"
    }

    fn collect(&mut self, world: &World, out: &mut FrameSnapshot) {
        out.ecs = world.world_stats();
        if self.detailed_archetypes {
            out.archetypes = world.archetype_summaries();
        }

        // Üst-düzey sayıları zaman-serisi için metrik grubuna da yansıt. Dizi önce
        // kurulur (out.ecs okuması orada biter), sonra push'lanır → klona gerek yok.
        for (name, value) in [
            ("entities", out.ecs.entities as f64),
            ("archetypes", out.ecs.archetypes as f64),
            ("non_empty_archetypes", out.ecs.non_empty_archetypes as f64),
            ("registered_components", out.ecs.registered_components as f64),
            ("resources", out.ecs.resources as f64),
            ("component_bytes", out.ecs.component_bytes as f64),
        ] {
            out.push_metric("ecs", name, value);
        }
    }
}
