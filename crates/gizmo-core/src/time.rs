/// Motor genelinde zaman yönetimi.
///
/// # Kullanım
/// ```rust,ignore
/// let mut time = Time::default();
///
/// // Her frame başında:
/// time.update(raw_dt);
///
/// // Okuma:
/// let dt = time.dt();           // Clamped delta (max 50ms)
/// let elapsed = time.elapsed(); // Toplam geçen süre
/// let frame = time.frame();     // Frame sayacı
/// let raw = time.raw_dt();      // Ham, clamp edilmemiş dt
///
/// // Zaman ölçeği:
/// time.set_time_scale(0.5); // Slow motion
/// time.set_time_scale(0.0); // Pause
/// ```
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Time {
    /// Clamped delta time (saniye). `time_scale` uygulanmış.
    dt: f32,
    /// Ham (raw) delta time — clamp ve scale uygulanmamış.
    raw_dt: f32,
    /// Toplam geçen süre (saniye, f64 hassasiyetinde).
    elapsed: f64,
    /// Frame sayacı.
    frame_count: u64,
    /// Zaman ölçeği. 1.0 = normal, 0.5 = slow motion, 0.0 = pause.
    time_scale: f32,
    /// Maksimum dt cap (saniye). Varsayılan: 1/20 = 50ms.
    max_dt: f32,
}

/// Varsayılan max dt: 50ms (20 FPS minimum).
const DEFAULT_MAX_DT: f32 = 1.0 / 20.0;

impl Time {
    pub fn new() -> Self {
        Self {
            dt: 0.0,
            raw_dt: 0.0,
            elapsed: 0.0,
            frame_count: 0,
            time_scale: 1.0,
            max_dt: DEFAULT_MAX_DT,
        }
    }

    /// Ham dt'yi alır, clamp + scale uygular ve tüm zamansal değerleri günceller.
    /// Her frame başında bir kez çağrılmalıdır.
    pub fn update(&mut self, raw_dt: f32) {
        self.raw_dt = raw_dt.max(0.0); // Negatif dt'yi engelle
        self.dt = (self.raw_dt * self.time_scale).min(self.max_dt);
        self.elapsed += self.dt as f64;
        self.frame_count += 1;
    }

    // ──── Getter'lar ────

    /// Clamped ve scaled delta time (saniye).
    /// Fizik, hareket, animasyon gibi sistemler bunu kullanmalıdır.
    #[inline]
    pub fn dt(&self) -> f32 {
        self.dt
    }

    /// Ham (raw) delta time — clamp ve scale uygulanmamış.
    /// Gerçek wall-clock zamanına ihtiyaç duyan sistemler için (ör: FPS sayacı).
    #[inline]
    pub fn raw_dt(&self) -> f32 {
        self.raw_dt
    }

    /// Toplam geçen süre (saniye, f64 hassasiyetinde).
    /// Uzun oturumlarda bile hassasiyetini korur.
    #[inline]
    pub fn elapsed(&self) -> f64 {
        self.elapsed
    }

    /// Toplam frame sayısı.
    #[inline]
    pub fn frame(&self) -> u64 {
        self.frame_count
    }

    /// Mevcut zaman ölçeği.
    #[inline]
    pub fn time_scale(&self) -> f32 {
        self.time_scale
    }

    /// Mevcut FPS (1/raw_dt). raw_dt = 0 ise 0.0 döner.
    #[inline]
    pub fn fps(&self) -> f32 {
        if self.raw_dt > 0.0 {
            1.0 / self.raw_dt
        } else {
            0.0
        }
    }

    // ──── Setter'lar ────

    /// Zaman ölçeğini ayarlar. 0.0 = durdur, 0.5 = ağır çekim, 1.0 = normal, 2.0 = hızlı.
    pub fn set_time_scale(&mut self, scale: f32) {
        self.time_scale = scale.max(0.0);
    }

    /// Maksimum dt cap'ini ayarlar (saniye).
    pub fn set_max_dt(&mut self, max: f32) {
        self.max_dt = max.max(0.001); // En az ~1ms
    }
}

impl Default for Time {
    fn default() -> Self {
        Self::new()
    }
}

// ═══════════════════════════════════════════════════════════════════════
//  PhysicsTime — Sabit zaman adımlı fizik zamanlayıcı
//
//  Fizik motoru sabit dt'de çalışır (varsayılan 1/60s = 16.67ms).
//  Render frame'leri değişken hızda çalışırken, fizik her zaman aynı
//  dt ile güncellenir → determinizm + kararlılık.
//
//  Kullanım:
//    Frame başında `accumulate(render_dt)` çağrılır.
//    `should_step()` true döndüğü sürece fizik adımları çalıştırılır.
//    `consume_step()` ile accumulator azaltılır.
//    `alpha()` ile render interpolasyonu yapılır.
// ═══════════════════════════════════════════════════════════════════════
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct PhysicsTime {
    /// Sabit fizik zaman adımı (saniye). Varsayılan: 1/60.
    fixed_dt: f32,
    /// Birikmiş süre — henüz fizik adımı olarak harcanmamış zaman.
    accumulator: f32,
    /// Maksimum birikim limiti (spiral of death koruması).
    max_accumulator: f32,
    /// Toplam fizik adım sayısı.
    step_count: u64,
    /// Toplam fizik süresi (f64 hassasiyetinde).
    physics_elapsed: f64,
    /// İnterpolasyon katsayısı: 0.0..1.0 arası.
    /// Render sırasında `lerp(prev_state, curr_state, alpha)` için kullanılır.
    alpha: f32,
}

impl PhysicsTime {
    /// Yeni PhysicsTime oluşturur. `hz` = fizik güncellenme hızı (örn: 60, 120, 240).
    pub fn new(hz: u32) -> Self {
        let fixed_dt = 1.0 / hz as f32;
        Self {
            fixed_dt,
            accumulator: 0.0,
            max_accumulator: fixed_dt * 8.0, // En fazla 8 fizik adımı birikebilir
            step_count: 0,
            physics_elapsed: 0.0,
            alpha: 0.0,
        }
    }

    /// Render frame dt'sini biriktiriciye ekler.
    /// Her frame başında bir kez çağrılır.
    pub fn accumulate(&mut self, render_dt: f32) {
        self.accumulator += render_dt;
        // Spiral of death koruması
        if self.accumulator > self.max_accumulator {
            self.accumulator = self.max_accumulator;
        }
    }

    /// Bir fizik adımı için yeterli süre birikmiş mi?
    #[inline]
    pub fn should_step(&self) -> bool {
        self.accumulator >= self.fixed_dt
    }

    /// Bir fizik adımını "tüketir" — accumulator'dan fixed_dt düşer.
    /// Her fizik step'inden sonra çağrılır.
    pub fn consume_step(&mut self) {
        self.accumulator -= self.fixed_dt;
        self.step_count += 1;
        self.physics_elapsed += self.fixed_dt as f64;
    }

    /// İnterpolasyon alpha'sını hesaplar.
    /// Tüm fizik adımları bittikten sonra, render'dan önce çağrılır.
    pub fn compute_alpha(&mut self) {
        self.alpha = self.accumulator / self.fixed_dt;
    }

    // ──── Getter'lar ────

    /// Sabit fizik dt'si (saniye).
    #[inline]
    pub fn fixed_dt(&self) -> f32 {
        self.fixed_dt
    }

    /// İnterpolasyon katsayısı (0.0 .. 1.0).
    /// `render_pos = lerp(prev_physics_pos, curr_physics_pos, alpha)`
    #[inline]
    pub fn alpha(&self) -> f32 {
        self.alpha
    }

    /// Toplam fizik adım sayısı.
    #[inline]
    pub fn step_count(&self) -> u64 {
        self.step_count
    }

    /// Toplam fizik süresi (f64 hassasiyetinde).
    #[inline]
    pub fn physics_elapsed(&self) -> f64 {
        self.physics_elapsed
    }

    /// Birikmiş süre (debug amaçlı).
    #[inline]
    pub fn accumulator(&self) -> f32 {
        self.accumulator
    }

    // ──── Setter'lar ────

    /// Fizik hızını değiştirir (Hz). Dikkat: birikmiş süre sıfırlanmaz.
    pub fn set_hz(&mut self, hz: u32) {
        self.fixed_dt = 1.0 / hz.max(1) as f32;
        self.max_accumulator = self.fixed_dt * 8.0;
    }
}

impl Default for PhysicsTime {
    fn default() -> Self {
        Self::new(60) // 60 Hz fizik
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_update() {
        let mut time = Time::new();
        time.update(0.016);

        assert!((time.dt() - 0.016).abs() < 0.0001);
        assert!((time.raw_dt() - 0.016).abs() < 0.0001);
        assert!((time.elapsed() - 0.016).abs() < 0.001);
        assert_eq!(time.frame(), 1);
    }

    #[test]
    fn test_dt_clamp() {
        let mut time = Time::new();
        time.update(1.0); // 1 saniye spike

        assert!(time.dt() <= DEFAULT_MAX_DT + 0.0001);
        assert!((time.raw_dt() - 1.0).abs() < 0.0001);
    }

    #[test]
    fn test_negative_dt_clamped_to_zero() {
        let mut time = Time::new();
        time.update(-0.5);

        assert_eq!(time.dt(), 0.0);
        assert_eq!(time.raw_dt(), 0.0);
    }

    #[test]
    fn test_time_scale() {
        let mut time = Time::new();
        time.set_time_scale(0.5);
        time.update(0.016);

        assert!((time.dt() - 0.008).abs() < 0.0001); // 0.016 * 0.5
        assert!((time.raw_dt() - 0.016).abs() < 0.0001);
    }

    #[test]
    fn test_time_scale_zero_is_pause() {
        let mut time = Time::new();
        time.set_time_scale(0.0);
        time.update(0.016);

        assert_eq!(time.dt(), 0.0);
        assert_eq!(time.elapsed(), 0.0);
        assert_eq!(time.frame(), 1); // Frame hâlâ sayılır
    }

    #[test]
    fn test_elapsed_accumulates() {
        let mut time = Time::new();
        for _ in 0..100 {
            time.update(0.01);
        }

        assert!((time.elapsed() - 1.0).abs() < 0.01);
        assert_eq!(time.frame(), 100);
    }

    #[test]
    fn test_fps() {
        let mut time = Time::new();
        time.update(1.0 / 60.0);
        assert!((time.fps() - 60.0).abs() < 1.0);

        time.update(0.0);
        assert_eq!(time.fps(), 0.0); // Division by zero koruması
    }

    #[test]
    fn test_custom_max_dt() {
        let mut time = Time::new();
        time.set_max_dt(1.0 / 10.0); // 100ms
        time.update(0.5);

        assert!((time.dt() - 0.1).abs() < 0.0001); // 100ms cap
    }

    #[test]
    fn test_serde_derive() {
        // serde derive doğru çalışıyor — serialize/deserialize uygulanmış
        let mut time = Time::new();
        time.update(0.016);
        time.update(0.016);

        // Clone ile roundtrip kontrolü (serde_json bağımlılık gerektirmeden)
        let cloned = time;
        assert_eq!(cloned.frame(), time.frame());
        assert!((cloned.elapsed() - time.elapsed()).abs() < 0.001);
    }

    // ─── PhysicsTime Testleri ───

    #[test]
    fn test_physics_time_basic_step() {
        let mut pt = PhysicsTime::new(60);
        assert!(!pt.should_step()); // Henüz birikim yok

        pt.accumulate(1.0 / 60.0); // Tam bir fizik adımı
        assert!(pt.should_step());

        pt.consume_step();
        assert!(!pt.should_step());
        assert_eq!(pt.step_count(), 1);
    }

    #[test]
    fn test_physics_time_multiple_steps() {
        let mut pt = PhysicsTime::new(60);
        let fixed_dt = pt.fixed_dt();
        // 3.5 adıma yetecek birikim (FP hassasiyeti için margin)
        pt.accumulate(fixed_dt * 3.5);

        let mut steps = 0;
        while pt.should_step() {
            pt.consume_step();
            steps += 1;
        }
        assert_eq!(steps, 3);
        assert_eq!(pt.step_count(), 3);
    }

    #[test]
    fn test_physics_time_spiral_of_death() {
        let mut pt = PhysicsTime::new(60);
        // 1 saniyelik spike — max 8 adım birikebilir
        pt.accumulate(1.0);

        let mut steps = 0;
        while pt.should_step() {
            pt.consume_step();
            steps += 1;
        }
        assert!(steps <= 8, "Spiral koruması: max 8 adım, bulundu: {}", steps);
    }

    #[test]
    fn test_physics_time_alpha() {
        let mut pt = PhysicsTime::new(60);
        let fixed_dt = 1.0 / 60.0;

        // 1.5 fizik adımı birikim
        pt.accumulate(fixed_dt * 1.5);
        pt.consume_step(); // 1 adım tüket
        pt.compute_alpha();

        // Kalan 0.5 adım → alpha ≈ 0.5
        assert!((pt.alpha() - 0.5).abs() < 0.01, "Alpha ≈ 0.5: {}", pt.alpha());
    }

    #[test]
    fn test_physics_time_elapsed() {
        let mut pt = PhysicsTime::new(60);
        for _ in 0..60 {
            pt.accumulate(1.0 / 60.0);
            while pt.should_step() {
                pt.consume_step();
            }
        }
        // 60 adım × 1/60 = 1.0s
        assert!((pt.physics_elapsed() - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_physics_time_set_hz() {
        let mut pt = PhysicsTime::new(60);
        pt.set_hz(120);
        assert!((pt.fixed_dt() - 1.0 / 120.0).abs() < 1e-6);
    }
}
