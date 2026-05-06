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
//!     println!("{}: {:.2}ms", scope.name, scope.duration_ms());
//! }
//! ```

use std::time::Instant;

/// Tek bir profiling kapsamı (scope) — başlangıç ve bitiş zamanı.
#[derive(Debug, Clone)]
pub struct ProfileScope {
    pub name: &'static str,
    pub start_ns: u64,
    pub end_ns: u64,
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
    pub scopes: Vec<ProfileScope>,
    pub frame_number: u64,
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
        if !self.enabled { return; }
        let now_ns = self.epoch.elapsed().as_nanos() as u64;
        self.active_scopes.push((name, now_ns, self.current_depth));
        self.current_depth += 1;
    }

    /// Aktif scope'u kapatır ve zamanlama verisini kaydeder.
    #[inline]
    pub fn end_scope(&mut self, name: &'static str) {
        if !self.enabled { return; }
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
        if !self.enabled { return; }

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

    /// Son N frame'in toplam süre ortalaması (ms).
    pub fn avg_frame_ms(&self, n: usize) -> f64 {
        let count = n.min(self.history.len());
        if count == 0 { return 0.0; }
        let sum: f64 = self.history.iter()
            .rev()
            .take(count)
            .map(|p| p.total_ms)
            .sum();
        sum / count as f64
    }

    /// Son N frame boyunca belirli bir scope'un ortalama süresi (ms).
    pub fn avg_scope_ms(&self, name: &str, n: usize) -> f64 {
        let count = n.min(self.history.len());
        if count == 0 { return 0.0; }
        let mut total = 0.0;
        let mut found = 0;
        for profile in self.history.iter().rev().take(count) {
            for scope in &profile.scopes {
                if scope.name == name {
                    total += scope.duration_ms();
                    found += 1;
                }
            }
        }
        if found == 0 { 0.0 } else { total / found as f64 }
    }

    /// Tüm history'deki frame profilleri (ring buffer sırasıyla).
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
        if avg > 0.0 { 1000.0 / avg } else { 0.0 }
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
        ProfileGuard { profiler: self, name }
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
        
        for i in 0..350 {
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
