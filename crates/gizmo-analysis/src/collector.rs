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

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct A(#[allow(dead_code)] f32);
    #[derive(Clone)]
    struct B(#[allow(dead_code)] u32);
    impl gizmo_core::Component for A {}
    impl gizmo_core::Component for B {}

    fn world_with_two_archetypes() -> World {
        let mut w = World::new();
        // Two entities with just A, one with A+B → two distinct non-empty archetypes.
        for _ in 0..2 {
            let e = w.spawn();
            w.add_component(e, A(1.0));
        }
        let e = w.spawn();
        w.add_component(e, A(1.0));
        w.add_component(e, B(9));
        w
    }

    #[test]
    fn default_collector_is_detailed() {
        assert!(EcsCollector::default().detailed_archetypes);
    }

    #[test]
    fn collector_name_is_stable() {
        assert_eq!(EcsCollector::default().name(), "ecs");
    }

    #[test]
    fn collect_populates_ecs_stats_and_metric_group() {
        let w = world_with_two_archetypes();
        let mut c = EcsCollector { detailed_archetypes: true };
        let mut snap = FrameSnapshot::default();
        c.collect(&w, &mut snap);

        assert_eq!(snap.ecs.entities, 3, "3 live entities across both archetypes");
        assert_eq!(snap.ecs.non_empty_archetypes, 2);

        // The high-level counts are mirrored into the "ecs" metric group as a time series.
        assert_eq!(snap.metric("ecs", "entities"), Some(3.0));
        assert_eq!(snap.metric("ecs", "non_empty_archetypes"), Some(2.0));
        // Exactly the six documented entries are emitted.
        assert_eq!(snap.groups["ecs"].len(), 6);
    }

    #[test]
    fn detailed_flag_toggles_archetype_table_only() {
        let w = world_with_two_archetypes();

        let mut detailed = EcsCollector { detailed_archetypes: true };
        let mut s1 = FrameSnapshot::default();
        detailed.collect(&w, &mut s1);
        assert!(!s1.archetypes.is_empty(), "detailed run fills the archetype table");

        let mut brief = EcsCollector { detailed_archetypes: false };
        let mut s2 = FrameSnapshot::default();
        brief.collect(&w, &mut s2);
        assert!(s2.archetypes.is_empty(), "brief run leaves the archetype table empty");
        // ...but the top-level stats + metric group are present either way.
        assert_eq!(s2.ecs.entities, 3);
        assert_eq!(s2.metric("ecs", "entities"), Some(3.0));
    }
}
