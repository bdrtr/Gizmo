//! Hafif in-game profiler — Frame bazlı zamanlama verileri toplar.
//!
//! # Kullanım
//! ```ignore
//! // Kayıt
//! let mut profiler = world.get_resource_mut::<FrameProfiler>().unwrap();
//! profiler.begin_scope("physics");
//! // ... fizik hesabı ...
//! profiler.end_scope("physics");
//! profiler.end_frame();
//!
//! // Okuma (UI panelinde)
//! let profiler = world.get_resource::<FrameProfiler>().unwrap();
//! for scope in profiler.current_frame_scopes() {
//!     tracing::info!("{}: {:.2}ms", scope.name, scope.duration_ms());
//! }
//! ```

#[cfg(target_arch = "wasm32")]
use web_time::Instant;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

/// Tek bir profiling kapsamı (scope) — başlangıç ve bitiş zamanı.
#[derive(Debug, Clone)]
pub struct ProfileScope {
    /// The literal passed to `begin_scope`/`end_scope`. `&'static str` keeps recording
    /// allocation-free, but names are neither interned nor unique: one frame may hold several
    /// scopes with the same name, and matching a close to an open is done by name, innermost
    /// (most recently opened) first.
    pub name: &'static str,
    /// Open time in nanoseconds since the owning [`FrameProfiler`]'s epoch — the instant that
    /// profiler was constructed. Not a UNIX timestamp, and not comparable against timings
    /// taken from a different `FrameProfiler`.
    pub start_ns: u64,
    /// Close time on the same monotonic clock and epoch as `start_ns`. Always `>= start_ns`,
    /// because it is sampled strictly later from that clock — the subtraction in
    /// [`ProfileScope::duration_ms`] relies on this and would underflow otherwise.
    pub end_ns: u64,
    /// Nesting depth recorded when the scope was *opened*: 0 for a top-level scope, 1 for one
    /// opened while a scope was already active, and so on. It is a display hint, not a tree
    /// link — nothing here names the parent scope. Depth restarts at 0 every frame.
    pub depth: u32,
}

impl ProfileScope {
    /// Kapsamın süresi (milisaniye).
    #[inline]
    pub fn duration_ms(&self) -> f64 {
        (self.end_ns - self.start_ns) as f64 / 1_000_000.0
    }

    /// Kapsamın süresi (mikrosaniye).
    #[inline]
    pub fn duration_us(&self) -> f64 {
        (self.end_ns - self.start_ns) as f64 / 1_000.0
    }
}

/// Tek bir frame'in zamanlama verileri.
#[derive(Debug, Clone, Default)]
pub struct FrameProfile {
    /// Scopes that were *closed* during the frame, in closing order — not sorted by start
    /// time, and not a tree. With strictly nested begin/end pairs that puts an inner scope
    /// before the outer one containing it, but nothing enforces that nesting, so treat the
    /// order as closing order and nothing more. Scopes still open when `end_frame` ran are
    /// dropped entirely: they are neither reported here nor carried into the next frame.
    pub scopes: Vec<ProfileScope>,
    /// Zero-based frame index: the profiler's frame counter as it stood *before* the
    /// `end_frame` call that produced this profile. The first profile is number 0.
    pub frame_number: u64,
    /// Wall-clock milliseconds from the previous `end_frame` (or from profiler construction,
    /// for the first frame) to this one — the whole frame, including everything no scope
    /// covered. Expect it to exceed the sum of the depth-0 scopes; that gap is the
    /// un-instrumented remainder.
    pub total_ms: f64,
}

/// Ring-buffer tabanlı frame profiler.
/// Son `HISTORY_SIZE` frame'in verilerini saklar.
pub struct FrameProfiler {
    /// Tamamlanan frame profilleri (ring buffer).
    history: Vec<FrameProfile>,
    /// Ring buffer yazma indeksi.
    write_idx: usize,
    /// Toplam frame sayısı.
    frame_count: u64,
    /// Şu anki frame'in açık scope'ları.
    active_scopes: Vec<(&'static str, u64, u32)>, // (name, start_ns, depth)
    /// Şu anki frame'in tamamlanan scope'ları.
    current_scopes: Vec<ProfileScope>,
    /// Mevcut derinlik (iç içe scope'lar için).
    current_depth: u32,
    /// Frame başlangıç anı.
    frame_start: Instant,
    /// Profiler referans zamanı (monotonic clock başlangıcı).
    epoch: Instant,
    /// Profiler aktif mi? (false ise hiçbir şey kaydetmez)
    pub enabled: bool,
}

const HISTORY_SIZE: usize = 300; // Son 5 saniye @ 60fps

impl FrameProfiler {
    /// Creates an **enabled** profiler with an empty history, starting both the frame timer
    /// and the epoch that every `start_ns`/`end_ns` is measured from at this instant.
    ///
    /// History is capped at 300 frames (5 seconds at 60 FPS); once full it recycles slots in
    /// place. Because the frame timer starts here, the first frame's `total_ms` measures
    /// construction-to-first-`end_frame`, which is usually not a representative frame.
    ///
    /// Set `enabled = false` to turn `begin_scope`, `end_scope` and `end_frame` into no-ops —
    /// while disabled the profiler records nothing at all, so the history simply stops
    /// growing rather than filling with empty frames.
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            history: Vec::with_capacity(HISTORY_SIZE),
            write_idx: 0,
            frame_count: 0,
            active_scopes: Vec::with_capacity(16),
            current_scopes: Vec::with_capacity(32),
            current_depth: 0,
            frame_start: now,
            epoch: now,
            enabled: true,
        }
    }

    /// Yeni bir profiling scope başlatır.
    #[inline]
    pub fn begin_scope(&mut self, name: &'static str) {
        if !self.enabled {
            return;
        }
        let now_ns = self.epoch.elapsed().as_nanos() as u64;
        self.active_scopes.push((name, now_ns, self.current_depth));
        self.current_depth += 1;
    }

    /// Aktif scope'u kapatır ve zamanlama verisini kaydeder.
    #[inline]
    pub fn end_scope(&mut self, name: &'static str) {
        if !self.enabled {
            return;
        }
        let now_ns = self.epoch.elapsed().as_nanos() as u64;

        // Son eşleşen scope'u bul (iç içe olabilir)
        if let Some(pos) = self.active_scopes.iter().rposition(|(n, _, _)| *n == name) {
            let (_, start_ns, depth) = self.active_scopes.remove(pos);
            self.current_depth = self.current_depth.saturating_sub(1);
            self.current_scopes.push(ProfileScope {
                name,
                start_ns,
                end_ns: now_ns,
                depth,
            });
        }
    }

    /// Frame'i bitirir ve mevcut verileri history ring buffer'a yazar.
    pub fn end_frame(&mut self) {
        if !self.enabled {
            return;
        }

        let total_ms = self.frame_start.elapsed().as_secs_f64() * 1000.0;

        let profile = FrameProfile {
            scopes: std::mem::take(&mut self.current_scopes),
            frame_number: self.frame_count,
            total_ms,
        };

        if self.history.len() < HISTORY_SIZE {
            self.history.push(profile);
        } else {
            self.history[self.write_idx] = profile;
        }
        self.write_idx = (self.write_idx + 1) % HISTORY_SIZE;
        self.frame_count += 1;
        self.frame_start = Instant::now();
        self.active_scopes.clear();
        self.current_depth = 0;
    }

    /// Son tamamlanan frame'in scope verilerini döndürür.
    pub fn last_frame(&self) -> Option<&FrameProfile> {
        if self.history.is_empty() {
            return None;
        }
        let idx = if self.write_idx == 0 {
            self.history.len() - 1
        } else {
            self.write_idx - 1
        };
        self.history.get(idx)
    }

    /// The `count` most recent frames, newest first.
    ///
    /// `history.iter().rev()` is NOT this. `history` is a ring buffer: once it holds
    /// `HISTORY_SIZE` frames the physical `Vec` order stops being chronological — the slot at
    /// `write_idx` is the OLDEST frame, and everything before it is newer. Plain `rev()`
    /// therefore walks backwards from a frame that may be ~`HISTORY_SIZE` frames stale and
    /// wraps into the newest ones only at the far end. [`Self::last_frame`] always did this
    /// modular step by hand; the averaging functions did not, which is the bug this helper
    /// exists to fix in one place.
    ///
    /// While the ring is still filling, `write_idx == history.len()`, so the index reduces to
    /// `len - 1 - i` — exactly what `rev()` gave. The wrap is the only regime that changed.
    fn recent(&self, count: usize) -> impl Iterator<Item = &FrameProfile> + '_ {
        let len = self.history.len();
        let write_idx = self.write_idx;
        // `count.min(len)` keeps `write_idx + len - 1 - i` from underflowing, and guards the
        // `% len` against len == 0 (the range is then empty and the closure never runs).
        (0..count.min(len)).map(move |i| &self.history[(write_idx + len - 1 - i) % len])
    }

    /// Son N frame'in toplam süre ortalaması (ms).
    ///
    /// "Son N" ring buffer'ın KRONOLOJİK son N'i — fiziksel `Vec` sırası değil (bkz.
    /// `recent`). [`Self::estimated_fps`] bunu miras alır.
    pub fn avg_frame_ms(&self, n: usize) -> f64 {
        let count = n.min(self.history.len());
        if count == 0 {
            return 0.0;
        }
        let sum: f64 = self.recent(count).map(|p| p.total_ms).sum();
        sum / count as f64
    }

    /// Son N frame boyunca belirli bir scope'un ortalama süresi (ms).
    ///
    /// Aynı kronolojik-sıra düzeltmesi burada da geçerli (bkz. `recent`): bu fonksiyon
    /// denetim maddesinde adı geçmiyordu ama birebir aynı `iter().rev()` kusurunu taşıyordu.
    pub fn avg_scope_ms(&self, name: &str, n: usize) -> f64 {
        let count = n.min(self.history.len());
        if count == 0 {
            return 0.0;
        }
        let mut total = 0.0;
        let mut found = 0;
        for profile in self.recent(count) {
            for scope in &profile.scopes {
                if scope.name == name {
                    total += scope.duration_ms();
                    found += 1;
                }
            }
        }
        if found == 0 {
            0.0
        } else {
            total / found as f64
        }
    }

    /// Tüm history'deki frame profilleri (ring buffer sırasıyla).
    ///
    /// FİZİKSEL sıra — ring sarmaladıktan sonra kronolojik DEĞİL: `write_idx`'teki slot en
    /// eski frame'dir. Kronolojik gezinmek isteyen `frame_number` alanına bakmalı.
    pub fn history(&self) -> &[FrameProfile] {
        &self.history
    }

    /// Toplam frame sayısı.
    pub fn frame_count(&self) -> u64 {
        self.frame_count
    }

    /// Tahmini FPS (son 60 frame ortalaması).
    pub fn estimated_fps(&self) -> f64 {
        let avg = self.avg_frame_ms(60);
        if avg > 0.0 {
            1000.0 / avg
        } else {
            0.0
        }
    }
}

impl Default for FrameProfiler {
    fn default() -> Self {
        Self::new()
    }
}

/// RAII scope guard — drop edilince otomatik olarak scope'u kapatır.
/// Kullanım: `let _guard = profiler.scope_guard("physics");`
pub struct ProfileGuard<'a> {
    profiler: &'a mut FrameProfiler,
    name: &'static str,
}

impl<'a> Drop for ProfileGuard<'a> {
    fn drop(&mut self) {
        self.profiler.end_scope(self.name);
    }
}

impl FrameProfiler {
    /// RAII scope guard oluşturur — drop edilince scope otomatik kapanır.
    pub fn scope_guard(&mut self, name: &'static str) -> ProfileGuard<'_> {
        self.begin_scope(name);
        ProfileGuard {
            profiler: self,
            name,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_profiling() {
        let mut profiler = FrameProfiler::new();

        profiler.begin_scope("test_scope");
        std::thread::sleep(std::time::Duration::from_millis(1));
        profiler.end_scope("test_scope");
        profiler.end_frame();

        let last = profiler.last_frame().unwrap();
        assert_eq!(last.scopes.len(), 1);
        assert_eq!(last.scopes[0].name, "test_scope");
        assert!(last.scopes[0].duration_ms() > 0.5);
        assert_eq!(last.scopes[0].depth, 0);
    }

    #[test]
    fn test_nested_scopes() {
        let mut profiler = FrameProfiler::new();

        profiler.begin_scope("outer");
        profiler.begin_scope("inner");
        profiler.end_scope("inner");
        profiler.end_scope("outer");
        profiler.end_frame();

        let last = profiler.last_frame().unwrap();
        assert_eq!(last.scopes.len(), 2);
        // Inner finishes first
        assert_eq!(last.scopes[0].name, "inner");
        assert_eq!(last.scopes[0].depth, 1);
        assert_eq!(last.scopes[1].name, "outer");
        assert_eq!(last.scopes[1].depth, 0);
    }

    #[test]
    fn test_ring_buffer() {
        let mut profiler = FrameProfiler::new();

        for _i in 0..350 {
            profiler.begin_scope("frame_scope");
            profiler.end_scope("frame_scope");
            profiler.end_frame();
        }

        // Ring buffer should keep only HISTORY_SIZE frames
        assert_eq!(profiler.history().len(), HISTORY_SIZE);
        assert_eq!(profiler.frame_count(), 350);
    }

    #[test]
    fn test_avg_fps() {
        let mut profiler = FrameProfiler::new();

        // Simulate 10 frames
        for _ in 0..10 {
            profiler.end_frame();
        }

        // FPS should be very high (frames are almost instant)
        assert!(profiler.estimated_fps() > 100.0);
    }

    /// After the ring wraps, physical `Vec` order is no longer chronological — the slot at
    /// `write_idx` holds the OLDEST frame — so `history.iter().rev().take(n)` averaged the
    /// wrong frames. With 350 frames recorded (`write_idx == 50`) it averaged frames
    /// 240..=299 instead of the newest 60, i.e. data ~50 frames stale: a spike shows up in
    /// the HUD roughly a second late, and `estimated_fps` inherits it.
    ///
    /// The timings are stamped by hand after recording: `end_frame` samples a real clock, so
    /// the only deterministic way to assert WHICH frames were averaged is to give every
    /// frame a `total_ms` (and scope duration) equal to its own `frame_number`. The test
    /// module lives in `profiler.rs`, so the private ring is reachable.
    #[test]
    fn avg_over_ring_uses_newest_frames_after_wrap() {
        let mut profiler = FrameProfiler::new();
        for _ in 0..350 {
            profiler.begin_scope("s");
            profiler.end_scope("s");
            profiler.end_frame();
        }
        assert_eq!(profiler.history.len(), HISTORY_SIZE);
        assert_eq!(profiler.write_idx, 350 % HISTORY_SIZE);

        for p in &mut profiler.history {
            let n = p.frame_number;
            p.total_ms = n as f64;
            for s in &mut p.scopes {
                // duration_ms() == frame_number
                s.end_ns = s.start_ns + n * 1_000_000;
            }
        }

        // Newest 60 frames are 290..=349 → mean 319.5.
        // Pre-fix `iter().rev().take(60)` read physical 299..=240, i.e. frames 240..=299 → 269.5.
        assert!(
            (profiler.avg_frame_ms(60) - 319.5).abs() < 1e-9,
            "avg_frame_ms averaged the wrong frames: {}",
            profiler.avg_frame_ms(60)
        );
        assert!(
            (profiler.avg_scope_ms("s", 60) - 319.5).abs() < 1e-9,
            "avg_scope_ms averaged the wrong frames: {}",
            profiler.avg_scope_ms("s", 60)
        );
        // estimated_fps() is defined as avg_frame_ms(60) inverted.
        assert!((profiler.estimated_fps() - 1000.0 / 319.5).abs() < 1e-9);

        // Whole ring: order-independent, so this held before the fix too — it pins the
        // clamp (n > len) rather than the ordering. Frames 50..=349 → mean 199.5.
        assert!((profiler.avg_frame_ms(1000) - 199.5).abs() < 1e-9);
    }

    /// NOT the regression test for the wrap bug — it passes with or without the fix. It
    /// fences the other branch of `recent()`: while the ring is still filling
    /// (`write_idx == len`) the chronological order must stay what plain `rev()` gave.
    #[test]
    fn avg_frame_ms_before_wrap_is_unchanged() {
        let mut profiler = FrameProfiler::new();
        for _ in 0..10 {
            profiler.end_frame();
        }
        for p in &mut profiler.history {
            p.total_ms = p.frame_number as f64;
        }
        // Newest 3 frames are 7, 8, 9 → mean 8.0.
        assert!((profiler.avg_frame_ms(3) - 8.0).abs() < 1e-9);
        // Fewer frames than requested → clamps to all 10 (0..=9 → 4.5).
        assert!((profiler.avg_frame_ms(60) - 4.5).abs() < 1e-9);
    }

    #[test]
    fn test_disabled_profiler() {
        let mut profiler = FrameProfiler::new();
        profiler.enabled = false;

        profiler.begin_scope("disabled_scope");
        profiler.end_scope("disabled_scope");
        profiler.end_frame();

        // Nothing should be recorded
        assert!(profiler.history().is_empty());
    }
}
