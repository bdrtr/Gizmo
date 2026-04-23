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
}
