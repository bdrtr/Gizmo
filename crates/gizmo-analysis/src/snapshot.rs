//! Tek bir frame'in yapısal anlık görüntüsü (snapshot).
//!
//! Bir frame'de motorun gözlemlenebilir durumunun tamamı: ECS istatistikleri,
//! ayrıntılı archetype tablosu, zaman-damgalı span'ler (FrameProfiler'dan) ve
//! collector'ların eklediği serbest metrik grupları. `groups` alanı sayesinde herhangi
//! bir alt-sistem, snapshot tipini değiştirmeden kendi ayrıntısını ekleyebilir.

use gizmo_core::world::{ArchetypeSummary, WorldStats};
use std::collections::BTreeMap;

/// FrameProfiler'ın bir scope'unun analiz-tarafı kopyası (Chrome-trace için ns'ler dahil).
#[derive(Debug, Clone)]
pub struct SpanSample {
    pub name: &'static str,
    pub ms: f64,
    pub depth: u32,
    pub start_ns: u64,
    pub end_ns: u64,
}

/// Bir frame'in tam anlık görüntüsü.
#[derive(Debug, Clone, Default)]
pub struct FrameSnapshot {
    /// Sıfırdan başlayan frame numarası.
    pub frame: u64,
    /// Bu frame'in toplam süresi (ms) — mümkünse FrameProfiler'dan, yoksa duvar-saati.
    pub frame_ms: f64,
    /// Analyzer epoch'undan bu yana geçen zaman (ns) — zaman ekseni için.
    pub timestamp_ns: u64,
    /// ECS üst-düzey istatistikleri.
    pub ecs: WorldStats,
    /// Ayrıntılı archetype tablosu (config'e göre boş olabilir — ağır).
    pub archetypes: Vec<ArchetypeSummary>,
    /// Bu frame'de tamamlanan profiling span'leri (iç içe olabilir).
    pub spans: Vec<SpanSample>,
    /// Collector'ların eklediği serbest metrik grupları: grup → [(metrik, değer)].
    /// Örn. "physics" → [("bodies", 1281.0), ("solver_ms", 4.1), …].
    pub groups: BTreeMap<String, Vec<(String, f64)>>,
}

impl FrameSnapshot {
    /// Bir metrik grubuna (yoksa oluşturarak) değer ekler. Collector'lar bunu kullanır.
    pub fn push_metric(&mut self, group: &str, name: &str, value: f64) {
        self.groups
            .entry(group.to_string())
            .or_default()
            .push((name.to_string(), value));
    }

    /// Bir grup+metrik değerini okur (varsa).
    pub fn metric(&self, group: &str, name: &str) -> Option<f64> {
        self.groups
            .get(group)
            .and_then(|g| g.iter().find(|(n, _)| n == name).map(|(_, v)| *v))
    }

    /// En pahalı span (ms) — hızlı darboğaz göstergesi.
    pub fn hottest_span(&self) -> Option<&SpanSample> {
        self.spans
            .iter()
            .max_by(|a, b| a.ms.partial_cmp(&b.ms).unwrap_or(std::cmp::Ordering::Equal))
    }
}
