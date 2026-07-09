//! Metrik deposu — sayaç (counter), gösterge (gauge) ve örnek (sample) serileri.
//!
//! Her metrik son `capacity` değeri bir ring-buffer'da tutar; üzerinden tam istatistik
//! (min/max/ortalama/std-sapma/p50/p95/p99) hesaplanabilir. Bu, "en ufak ayrıntıyı"
//! zaman-serisi olarak analiz etmenin temelidir: herhangi bir ölçümü isimlendirip
//! kaydet, geçmişi üzerinden trend/tepe (spike) çıkar.

use std::collections::BTreeMap;
use std::collections::HashSet;
use std::collections::VecDeque;

/// Bir metriğin türü — nasıl yorumlanacağını belirler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricKind {
    /// Monoton artan toplam; her frame'in *deltası* örneklenir.
    Counter,
    /// Anlık değer (entity sayısı, bellek, sıcaklık…).
    Gauge,
    /// Ölçüm/zamanlama örneği (ms, iterasyon…) — istatistik için.
    Sample,
}

impl MetricKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            MetricKind::Counter => "counter",
            MetricKind::Gauge => "gauge",
            MetricKind::Sample => "sample",
        }
    }
}

/// Bir metriğin geçmişi üzerinden hesaplanan özet istatistik.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stats {
    pub count: usize,
    pub last: f64,
    pub min: f64,
    pub max: f64,
    pub mean: f64,
    pub stddev: f64,
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
}

impl Stats {
    /// Boş seri için sıfır istatistik.
    pub fn empty() -> Self {
        Stats {
            count: 0,
            last: 0.0,
            min: 0.0,
            max: 0.0,
            mean: 0.0,
            stddev: 0.0,
            p50: 0.0,
            p95: 0.0,
            p99: 0.0,
        }
    }
}

/// Tek bir isimlendirilmiş metrik serisi (ring-buffer).
#[derive(Debug, Clone)]
pub struct MetricSeries {
    pub kind: MetricKind,
    ring: VecDeque<f64>,
    capacity: usize,
    /// Counter için kümülatif toplam (delta hesabında kullanılır).
    total: f64,
    /// Counter için son frame'de eklenen ham toplam (delta ölçümü için).
    accum_this_frame: f64,
}

impl MetricSeries {
    fn new(kind: MetricKind, capacity: usize) -> Self {
        MetricSeries {
            kind,
            ring: VecDeque::with_capacity(capacity.min(1024)),
            capacity: capacity.max(1),
            total: 0.0,
            accum_this_frame: 0.0,
        }
    }

    fn push_ring(&mut self, v: f64) {
        if self.ring.len() == self.capacity {
            self.ring.pop_front();
        }
        self.ring.push_back(v);
    }

    /// Bu frame boyunca biriktir (Counter). `end_frame`'de ring'e delta olarak işlenir.
    fn add(&mut self, delta: f64) {
        self.accum_this_frame += delta;
        self.total += delta;
    }

    /// Anlık değer yaz (Gauge/Sample). Doğrudan ring'e girer.
    fn set(&mut self, v: f64) {
        self.push_ring(v);
    }

    /// Frame sonu: Counter'ın bu frame'deki deltasını ring'e yaz ve biriktiriciyi sıfırla.
    fn end_frame(&mut self) {
        if self.kind == MetricKind::Counter {
            self.push_ring(self.accum_this_frame);
            self.accum_this_frame = 0.0;
        }
    }

    /// Kümülatif toplam (yalnız Counter için anlamlı).
    pub fn total(&self) -> f64 {
        self.total
    }

    /// Son kaydedilen değer (ring'in son elemanı).
    pub fn last(&self) -> f64 {
        self.ring.back().copied().unwrap_or(0.0)
    }

    /// Ring'teki ham değerler (eskiden yeniye).
    pub fn values(&self) -> impl Iterator<Item = f64> + '_ {
        self.ring.iter().copied()
    }

    pub fn len(&self) -> usize {
        self.ring.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ring.is_empty()
    }

    /// Ring üzerinden tam istatistik.
    pub fn stats(&self) -> Stats {
        let n = self.ring.len();
        if n == 0 {
            return Stats::empty();
        }
        let mut sorted: Vec<f64> = self.ring.iter().copied().collect();
        // NaN'ları sona iterek toplam sıralama; NaN üretmeyiz ama savunmacı olalım.
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        let sum: f64 = sorted.iter().sum();
        let mean = sum / n as f64;
        let var = sorted.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / n as f64;

        Stats {
            count: n,
            last: self.last(),
            min: sorted[0],
            max: sorted[n - 1],
            mean,
            stddev: var.sqrt(),
            p50: percentile(&sorted, 0.50),
            p95: percentile(&sorted, 0.95),
            p99: percentile(&sorted, 0.99),
        }
    }
}

/// Sıralı bir dilim üzerinde doğrusal-interpolasyonlu yüzdelik. `q ∈ [0,1]`.
fn percentile(sorted: &[f64], q: f64) -> f64 {
    let n = sorted.len();
    if n == 0 {
        return 0.0;
    }
    if n == 1 {
        return sorted[0];
    }
    let rank = q.clamp(0.0, 1.0) * (n - 1) as f64;
    let lo = rank.floor() as usize;
    let hi = rank.ceil() as usize;
    if lo == hi {
        sorted[lo]
    } else {
        let frac = rank - lo as f64;
        sorted[lo] * (1.0 - frac) + sorted[hi] * frac
    }
}

/// İsimlendirilmiş metriklerin merkezi deposu.
#[derive(Debug, Clone)]
pub struct MetricStore {
    series: BTreeMap<String, MetricSeries>,
    capacity: usize,
    /// Names that already emitted a kind-collision warning (warn once, not every frame).
    warned: HashSet<String>,
}

impl MetricStore {
    pub fn new(capacity: usize) -> Self {
        MetricStore {
            series: BTreeMap::new(),
            capacity: capacity.max(1),
            warned: HashSet::new(),
        }
    }

    /// Return the mutable series for `name`, creating it with `kind` on first use.
    ///
    /// The **first** kind registered for a name wins. Instrumenting the same name with a
    /// different kind (e.g. `gauge("x")` then `counter_add("x")`) previously reused the
    /// existing series while ignoring the requested kind, silently corrupting it (a
    /// counter delta pushed onto a gauge ring, or a gauge write that `end_frame` never
    /// flushes). We now keep the original kind, drop the mismatched write, and warn once.
    fn entry(&mut self, name: &str, kind: MetricKind) -> Option<&mut MetricSeries> {
        // `kind` is Copy, so this read-borrow ends immediately — no alloc on the hot path
        // where the name already exists with the matching kind (steady state).
        match self.series.get(name).map(|s| s.kind) {
            Some(existing) if existing == kind => self.series.get_mut(name),
            Some(existing) => {
                if self.warned.insert(name.to_string()) {
                    Self::warn_kind_collision(name, existing, kind);
                }
                None
            }
            None => {
                let cap = self.capacity;
                Some(
                    self.series
                        .entry(name.to_string())
                        .or_insert_with(|| MetricSeries::new(kind, cap)),
                )
            }
        }
    }

    #[cfg_attr(not(feature = "trace"), allow(unused_variables))]
    fn warn_kind_collision(name: &str, existing: MetricKind, requested: MetricKind) {
        #[cfg(feature = "trace")]
        tracing::warn!(
            metric = name,
            existing = existing.as_str(),
            requested = requested.as_str(),
            "metrik kind çakışması: ilk kayıtlı kind korunuyor, uyumsuz yazım düşürülüyor"
        );
    }

    /// Counter'a ekle (bu frame'in deltasına birikir).
    pub fn counter_add(&mut self, name: &str, delta: f64) {
        if let Some(s) = self.entry(name, MetricKind::Counter) {
            s.add(delta);
        }
    }

    /// Gauge (anlık değer) yaz.
    pub fn gauge(&mut self, name: &str, value: f64) {
        if let Some(s) = self.entry(name, MetricKind::Gauge) {
            s.set(value);
        }
    }

    /// Sample (ölçüm) kaydet.
    pub fn sample(&mut self, name: &str, value: f64) {
        if let Some(s) = self.entry(name, MetricKind::Sample) {
            s.set(value);
        }
    }

    /// Frame sonu — tüm Counter serilerinin deltasını ring'e işler.
    pub fn end_frame(&mut self) {
        for s in self.series.values_mut() {
            s.end_frame();
        }
    }

    pub fn get(&self, name: &str) -> Option<&MetricSeries> {
        self.series.get(name)
    }

    /// Bir metriğin geçmişi üzerinden istatistik.
    pub fn stats(&self, name: &str) -> Option<Stats> {
        self.series.get(name).map(|s| s.stats())
    }

    /// Tüm metrik adları (deterministik sıralı).
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.series.keys().map(|s| s.as_str())
    }

    /// (isim, seri) çiftleri.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &MetricSeries)> {
        self.series.iter().map(|(k, v)| (k.as_str(), v))
    }

    pub fn len(&self) -> usize {
        self.series.len()
    }

    pub fn is_empty(&self) -> bool {
        self.series.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_and_stats_are_correct() {
        let mut store = MetricStore::new(100);
        for v in [10.0, 20.0, 30.0, 40.0, 50.0] {
            store.sample("t", v);
        }
        let s = store.stats("t").unwrap();
        assert_eq!(s.count, 5);
        assert_eq!(s.min, 10.0);
        assert_eq!(s.max, 50.0);
        assert_eq!(s.mean, 30.0);
        assert_eq!(s.p50, 30.0);
        assert_eq!(s.last, 50.0);
        assert!((s.p95 - 48.0).abs() < 1e-9); // 0.95*(4)=3.8 → 40 + 0.8*10 = 48
    }

    #[test]
    fn kind_collision_keeps_first_kind_and_drops_mismatched_write() {
        // Reusing a name with a different kind must NOT corrupt the first-registered series.
        let mut store = MetricStore::new(100);
        store.gauge("x", 60.0); // registers Gauge, ring = [60]
        store.counter_add("x", 5.0); // mismatched kind → dropped, series untouched
        store.end_frame();

        let s = store.get("x").unwrap();
        assert_eq!(s.kind, MetricKind::Gauge, "first-registered kind must win");
        assert_eq!(s.values().collect::<Vec<_>>(), vec![60.0], "gauge ring must be intact");
        assert_eq!(s.total(), 0.0, "counter delta must not leak into the gauge series");
    }

    #[test]
    fn kind_collision_symmetric_counter_then_gauge() {
        let mut store = MetricStore::new(100);
        store.counter_add("y", 3.0);
        store.gauge("y", 999.0); // mismatched → dropped
        store.end_frame();

        let s = store.get("y").unwrap();
        assert_eq!(s.kind, MetricKind::Counter);
        assert_eq!(s.total(), 3.0, "counter total preserved");
        // The dropped gauge write (999.0) must never appear in the ring.
        assert_eq!(s.values().collect::<Vec<_>>(), vec![3.0]);
    }

    #[test]
    fn counter_accumulates_per_frame_delta() {
        let mut store = MetricStore::new(100);
        // Frame 1: iki artış → delta 3
        store.counter_add("spawns", 1.0);
        store.counter_add("spawns", 2.0);
        store.end_frame();
        // Frame 2: bir artış → delta 5
        store.counter_add("spawns", 5.0);
        store.end_frame();

        let s = store.get("spawns").unwrap();
        assert_eq!(s.total(), 8.0);
        let vals: Vec<f64> = s.values().collect();
        assert_eq!(vals, vec![3.0, 5.0]);
    }

    #[test]
    fn ring_buffer_bounds_history() {
        let mut store = MetricStore::new(3);
        for v in 0..10 {
            store.gauge("g", v as f64);
        }
        let s = store.get("g").unwrap();
        assert_eq!(s.len(), 3);
        let vals: Vec<f64> = s.values().collect();
        assert_eq!(vals, vec![7.0, 8.0, 9.0]);
    }

    #[test]
    fn empty_series_returns_empty_stats() {
        let store = MetricStore::new(10);
        assert!(store.stats("missing").is_none());
        assert_eq!(Stats::empty().count, 0);
    }
}
