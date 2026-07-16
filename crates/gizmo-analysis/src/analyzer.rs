//! `Analyzer` — analiz merkezi. Bir World resource'u olarak eklenir.
//!
//! Her frame bir `FrameSnapshot` üretir (ECS durumu + span'ler + collector metrikleri),
//! bunu ring-buffer geçmişine yazar ve tüm sayısal değerleri `MetricStore`'a besler; böylece
//! herhangi bir metriğin geçmişi üzerinden istatistik/trend alınabilir.

use crate::collector::{Collector, EcsCollector};
use crate::metrics::{MetricStore, Stats};
use crate::snapshot::{FrameSnapshot, SpanSample};
use gizmo_core::world::World;
use gizmo_core::FrameProfiler;
use std::collections::VecDeque;
use std::fmt::Write as _;
use std::time::Instant;

/// Analyzer davranış ayarları.
#[derive(Debug, Clone)]
pub struct AnalysisConfig {
    /// Kapalıysa `collect` hiçbir şey yapmaz (sıfır maliyet).
    pub enabled: bool,
    /// Geçmişte tutulacak frame snapshot sayısı (ring-buffer).
    pub history_frames: usize,
    /// Metrik serilerinde tutulacak değer sayısı (ring-buffer).
    pub metric_history: usize,
    /// Ayrıntılı archetype tablosunu topla.
    pub detailed_archetypes: bool,
}

impl Default for AnalysisConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            history_frames: 300,   // ~5 sn @ 60fps
            metric_history: 1024,  // metrik başına ~17 sn @ 60fps
            detailed_archetypes: true,
        }
    }
}

/// Merkezi analiz durumu.
pub struct Analyzer {
    pub config: AnalysisConfig,
    frame: u64,
    metrics: MetricStore,
    collectors: Vec<Box<dyn Collector>>,
    history: VecDeque<FrameSnapshot>,
    epoch: Instant,
    last_collect: Instant,
}

impl Analyzer {
    /// Varsayılan ayarlarla + yerleşik `EcsCollector` ile.
    pub fn new() -> Self {
        Self::with_config(AnalysisConfig::default())
    }

    /// Verilen ayarlarla; yerleşik `EcsCollector` otomatik kayıtlı.
    pub fn with_config(config: AnalysisConfig) -> Self {
        let now = Instant::now();
        let mut analyzer = Self {
            metrics: MetricStore::new(config.metric_history),
            collectors: Vec::new(),
            history: VecDeque::with_capacity(config.history_frames.min(4096)),
            frame: 0,
            epoch: now,
            last_collect: now,
            config,
        };
        analyzer.register_collector(Box::new(EcsCollector {
            detailed_archetypes: analyzer.config.detailed_archetypes,
        }));
        analyzer
    }

    /// Ek bir collector kaydet (ör. `PhysicsCollector`).
    pub fn register_collector(&mut self, collector: Box<dyn Collector>) {
        self.collectors.push(collector);
    }

    /// Kayıtlı collector adları.
    pub fn collector_names(&self) -> Vec<&'static str> {
        self.collectors.iter().map(|c| c.name()).collect()
    }

    // ── Elle enstrümantasyon (sistem içinden `get_resource_mut` ile) ─────────
    /// Monoton sayaç — bu frame'in deltasına birikir.
    pub fn counter_add(&mut self, name: &str, delta: f64) {
        self.metrics.counter_add(name, delta);
    }
    /// Anlık gösterge değeri.
    pub fn gauge(&mut self, name: &str, value: f64) {
        self.metrics.gauge(name, value);
    }
    /// Ölçüm örneği (ms/iterasyon…).
    pub fn sample(&mut self, name: &str, value: f64) {
        self.metrics.sample(name, value);
    }

    // ── Sorgu API'si ─────────────────────────────────────────────────────────
    /// İşlenmiş frame sayısı.
    pub fn frame(&self) -> u64 {
        self.frame
    }
    /// Son frame snapshot'ı (geçmişin son elemanı).
    pub fn last(&self) -> Option<&FrameSnapshot> {
        self.history.back()
    }
    /// Snapshot geçmişi (eskiden yeniye).
    pub fn history(&self) -> impl Iterator<Item = &FrameSnapshot> {
        self.history.iter()
    }
    /// Metrik deposu (dışa aktarım/özel sorgu için).
    pub fn metrics(&self) -> &MetricStore {
        &self.metrics
    }
    /// Bir metriğin geçmişi üzerinden istatistik.
    pub fn stats(&self, metric: &str) -> Option<Stats> {
        self.metrics.stats(metric)
    }
    /// Tahmini FPS (frame_ms örnek geçmişinden).
    pub fn estimated_fps(&self) -> f64 {
        match self.metrics.stats("frame_ms") {
            Some(s) if s.mean > 0.0 => 1000.0 / s.mean,
            _ => 0.0,
        }
    }

    /// Frame başına bir kez çağrılır: snapshot üret, geçmişe yaz, collector'ları çalıştır.
    pub fn collect(&mut self, world: &World) {
        if !self.config.enabled {
            return;
        }

        let now = Instant::now();
        let timestamp_ns = self.epoch.elapsed().as_nanos() as u64;

        // Frame süresi: FrameProfiler varsa oradan (daha doğru), yoksa duvar-saati.
        let mut frame_ms = now.duration_since(self.last_collect).as_secs_f64() * 1000.0;
        let mut spans: Vec<SpanSample> = Vec::new();
        if let Some(profiler) = world.get_resource::<FrameProfiler>() {
            if let Some(fp) = profiler.last_frame() {
                if fp.total_ms > 0.0 {
                    frame_ms = fp.total_ms;
                }
                spans = fp
                    .scopes
                    .iter()
                    .map(|s| SpanSample {
                        name: s.name,
                        ms: s.duration_ms(),
                        depth: s.depth,
                        start_ns: s.start_ns,
                        end_ns: s.end_ns,
                    })
                    .collect();
            }
        }
        self.last_collect = now;

        let mut snap = FrameSnapshot {
            frame: self.frame,
            frame_ms,
            timestamp_ns,
            spans,
            ..Default::default()
        };

        // Collector'ları çalıştır (kendi Vec'lerini geçici olarak dışarı al ki `self`'i
        // hem collector'a hem snapshot'a aynı anda ödünç verme sorunu olmasın).
        let mut collectors = std::mem::take(&mut self.collectors);
        for c in collectors.iter_mut() {
            c.collect(world, &mut snap);
        }
        self.collectors = collectors;

        // Snapshot metriklerini + span'leri MetricStore'a yansıt (zaman-serisi).
        // Anahtarları yeniden-kullanılan tek bir tampona yazarak her-frame `format!`
        // tahsisinden kaçın.
        self.metrics.sample("frame_ms", frame_ms);
        let mut key = String::new();
        for (group, entries) in &snap.groups {
            for (name, value) in entries {
                key.clear();
                let _ = write!(key, "{group}.{name}");
                self.metrics.gauge(&key, *value);
            }
        }
        for s in &snap.spans {
            key.clear();
            let _ = write!(key, "span.{}", s.name);
            self.metrics.sample(&key, s.ms);
        }

        // Frame'i kapat (Counter deltaları ring'e işlenir), geçmişe yaz (klonsuz —
        // `last()` artık geçmişin son elemanı).
        self.metrics.end_frame();

        if self.history.len() == self.config.history_frames.max(1) {
            self.history.pop_front();
        }
        self.history.push_back(snap);
        self.frame += 1;
    }
}

impl Default for Analyzer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_core::world::World;
    use gizmo_core::FrameProfiler;

    #[derive(Clone)]
    struct Pos(#[allow(dead_code)] f32);
    impl gizmo_core::Component for Pos {}

    /// A minimal headless world with a FrameProfiler and `n` entities carrying `Pos`.
    fn world_with(n: usize) -> World {
        let mut w = World::new();
        w.insert_resource(FrameProfiler::new());
        for i in 0..n {
            let e = w.spawn();
            w.add_component(e, Pos(i as f32));
        }
        w
    }

    #[test]
    fn new_auto_registers_ecs_collector() {
        assert_eq!(Analyzer::new().collector_names(), vec!["ecs"]);
    }

    #[test]
    fn collect_increments_frame_and_records_history() {
        let w = world_with(2);
        let mut a = Analyzer::new();
        for _ in 0..3 {
            a.collect(&w);
        }
        assert_eq!(a.frame(), 3);
        assert_eq!(a.history().count(), 3);
        assert_eq!(a.last().unwrap().frame, 2, "last snapshot carries the newest frame index");
    }

    #[test]
    fn disabled_config_makes_collect_a_noop() {
        let w = world_with(1);
        let mut a = Analyzer::with_config(AnalysisConfig { enabled: false, ..Default::default() });
        a.collect(&w);
        a.collect(&w);
        assert_eq!(a.frame(), 0);
        assert!(a.last().is_none());
        assert!(a.metrics().is_empty());
    }

    #[test]
    fn history_ring_is_bounded_and_keeps_newest() {
        let w = world_with(1);
        let mut a = Analyzer::with_config(AnalysisConfig { history_frames: 2, ..Default::default() });
        for _ in 0..5 {
            a.collect(&w);
        }
        // Five frames processed, but only the two most recent snapshots retained.
        assert_eq!(a.frame(), 5);
        let frames: Vec<u64> = a.history().map(|f| f.frame).collect();
        assert_eq!(frames, vec![3, 4]);
    }

    #[test]
    fn estimated_fps_is_reciprocal_of_mean_frame_ms() {
        let mut a = Analyzer::new();
        // No frame_ms data yet → 0, not a divide-by-zero / NaN.
        assert_eq!(a.estimated_fps(), 0.0);
        // Inject deterministic frame durations (sample() writes straight to the ring).
        a.sample("frame_ms", 20.0);
        a.sample("frame_ms", 20.0);
        assert!((a.estimated_fps() - 50.0).abs() < 1e-9, "1000/20 = 50 fps");
    }

    #[test]
    fn instrumentation_delegates_to_metric_store() {
        let w = world_with(0);
        let mut a = Analyzer::new();
        a.counter_add("hits", 2.0);
        a.counter_add("hits", 3.0);
        a.gauge("temp", 7.0);
        // collect() flushes the counter frame (end_frame) into the ring.
        a.collect(&w);
        let hits = a.metrics().get("hits").unwrap();
        assert_eq!(hits.total(), 5.0);
        assert_eq!(hits.last(), 5.0, "the frame delta (2+3) lands in the ring");
        assert_eq!(a.stats("temp").unwrap().last, 7.0);
    }

    #[test]
    fn collect_projects_ecs_snapshot_into_metrics() {
        let w = world_with(3);
        let mut a = Analyzer::new();
        a.collect(&w);
        let last = a.last().unwrap();
        assert_eq!(last.ecs.entities, 3);
        // EcsCollector's "ecs" group is mirrored into gauges keyed "<group>.<name>".
        assert_eq!(a.stats("ecs.entities").unwrap().last, 3.0);
        // frame_ms is always sampled once per collect.
        assert_eq!(a.stats("frame_ms").unwrap().count, 1);
    }
}
