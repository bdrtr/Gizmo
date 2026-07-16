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

#[cfg(test)]
mod tests {
    use super::*;

    fn span(name: &'static str, ms: f64) -> SpanSample {
        SpanSample { name, ms, depth: 0, start_ns: 0, end_ns: 0 }
    }

    #[test]
    fn default_snapshot_is_empty() {
        let s = FrameSnapshot::default();
        assert_eq!(s.frame, 0);
        assert_eq!(s.frame_ms, 0.0);
        assert!(s.groups.is_empty());
        assert!(s.spans.is_empty());
        assert!(s.hottest_span().is_none());
        assert_eq!(s.ecs, WorldStats::default());
    }

    #[test]
    fn push_metric_creates_group_then_appends_in_order() {
        let mut s = FrameSnapshot::default();
        s.push_metric("physics", "bodies", 5.0);
        s.push_metric("physics", "contacts", 2.0);
        let g = &s.groups["physics"];
        // Insertion order is preserved within a group (it's a Vec, not a map).
        assert_eq!(g, &vec![("bodies".to_string(), 5.0), ("contacts".to_string(), 2.0)]);
    }

    #[test]
    fn metric_lookup_hits_misses_and_first_write_wins() {
        let mut s = FrameSnapshot::default();
        s.push_metric("physics", "bodies", 5.0);
        assert_eq!(s.metric("physics", "bodies"), Some(5.0));
        assert_eq!(s.metric("physics", "nope"), None, "missing name → None");
        assert_eq!(s.metric("render", "bodies"), None, "missing group → None");

        // Duplicate name in the same group: `metric` returns the FIRST occurrence.
        s.push_metric("physics", "bodies", 99.0);
        assert_eq!(s.metric("physics", "bodies"), Some(5.0));
    }

    #[test]
    fn groups_iterate_in_sorted_order() {
        // `groups` is a BTreeMap → deterministic, sorted key order regardless of
        // insertion sequence (important for stable JSON/CSV export).
        let mut s = FrameSnapshot::default();
        s.push_metric("zebra", "a", 1.0);
        s.push_metric("alpha", "b", 2.0);
        s.push_metric("mid", "c", 3.0);
        let keys: Vec<&str> = s.groups.keys().map(|k| k.as_str()).collect();
        assert_eq!(keys, vec!["alpha", "mid", "zebra"]);
    }

    #[test]
    fn hottest_span_picks_max_ms() {
        let s = FrameSnapshot {
            spans: vec![span("a", 1.0), span("b", 5.0), span("c", 3.0)],
            ..Default::default()
        };
        assert_eq!(s.hottest_span().unwrap().name, "b");
    }

    #[test]
    fn hottest_span_is_nan_tolerant() {
        // A NaN duration must not panic the comparator; a finite span is still returned.
        let s = FrameSnapshot {
            spans: vec![span("bad", f64::NAN), span("good", 2.0)],
            ..Default::default()
        };
        let h = s.hottest_span().expect("must return a span, not panic");
        // Comparator maps NaN to Equal; the finite maximum "good" is reachable.
        assert!(h.name == "good" || h.name == "bad");
    }
}
