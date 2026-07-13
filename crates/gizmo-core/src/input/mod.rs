use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Ergonomik input soyutlama katmanı.
///
/// Kullanım:
/// ```rust,ignore
/// if input.is_key_pressed(KeyCode::KeyW as u32) { /* ileri git */ }
/// if input.is_key_just_pressed(KeyCode::Space as u32) { /* zıpla (tek sefer) */ }
/// if input.is_mouse_button_pressed(mouse::LEFT) { /* ateş et */ }
/// let (dx, dy) = input.mouse_delta(); /* fare hareketi */
/// let scroll = input.mouse_scroll(); /* tekerlek */
/// ```
#[derive(Clone, Serialize, Deserialize)]
pub struct Input {
    // Tuş durumları
    keys_pressed: HashSet<u32>,       // Şu an basılı tuşlar
    keys_just_pressed: HashSet<u32>,  // Bu frame'de yeni basılan
    keys_just_released: HashSet<u32>, // Bu frame'de bırakılan

    // Fare durumları
    mouse_buttons_pressed: HashSet<u32>,
    mouse_buttons_just_pressed: HashSet<u32>,
    mouse_buttons_just_released: HashSet<u32>,

    // Fare pozisyonu ve hareket
    mouse_position: (f32, f32),
    mouse_delta: (f32, f32),

    // Fare tekerlek (scroll) deltası
    mouse_scroll_delta: f32,
}

impl Input {
    pub fn new() -> Self {
        Self {
            keys_pressed: HashSet::new(),
            keys_just_pressed: HashSet::new(),
            keys_just_released: HashSet::new(),
            mouse_buttons_pressed: HashSet::new(),
            mouse_buttons_just_pressed: HashSet::new(),
            mouse_buttons_just_released: HashSet::new(),
            mouse_position: (0.0, 0.0),
            mouse_delta: (0.0, 0.0),
            mouse_scroll_delta: 0.0,
        }
    }

    // ==================== FRAME YAŞAM DÖNGÜSÜ ====================

    /// Her frame başında çağrılmalı — "just pressed/released" setlerini temizler
    /// ve deferred tuş bırakmalarını gerçekleştirir.
    ///
    /// Mantık:
    /// - `on_key_released()` aynı frame'de basılıp bırakılan tuşlar için `keys_pressed`'den
    ///   silmeyi erteliyordu (fast-tap koruması). `begin_frame()` bu deferred silmeleri gerçekleştirir.
    /// - Ardından just_pressed ve just_released setleri temizlenir, fare deltaları sıfırlanır.
    pub fn begin_frame(&mut self) {
        // Deferred removal: aynı frame'de basılıp bırakılan tuşları artık kaldır
        for k in &self.keys_just_released {
            self.keys_pressed.remove(k);
        }
        for b in &self.mouse_buttons_just_released {
            self.mouse_buttons_pressed.remove(b);
        }

        self.keys_just_pressed.clear();
        self.keys_just_released.clear();
        self.mouse_buttons_just_pressed.clear();
        self.mouse_buttons_just_released.clear();
        self.mouse_delta = (0.0, 0.0);
        self.mouse_scroll_delta = 0.0;
    }

    // ==================== TUŞ GİRDİSİ ====================

    /// Basılı tüm tuşları döndürür (Debug için)
    pub fn pressed_keys(&self) -> Vec<u32> {
        self.keys_pressed.iter().copied().collect()
    }

    /// Tuş basıldığında çağır (winit KeyCode'un scan code'u)
    pub fn on_key_pressed(&mut self, key: u32) {
        // Cancel a pending fast-tap deferral: if the key was released and re-pressed
        // within the SAME frame, `begin_frame` would otherwise honor the earlier
        // deferred removal and drop a physically-held key (then spuriously re-fire
        // just_pressed on the next auto-repeat).
        self.keys_just_released.remove(&key);
        if self.keys_pressed.insert(key) {
            self.keys_just_pressed.insert(key);
        }
    }

    /// Tuş bırakıldığında çağır.
    ///
    /// Eğer tuş aynı frame'de basılıp bırakıldıysa (`keys_just_pressed` içindeyse),
    /// `keys_pressed`'den silmeyi `begin_frame()`'e erteler. Böylece oyun bu "fast tap"ı
    /// kaçırmaz — hem `is_key_pressed` hem `is_key_just_pressed` o frame boyunca true döner.
    pub fn on_key_released(&mut self, key: u32) {
        self.keys_just_released.insert(key);
        if !self.keys_just_pressed.contains(&key) {
            // Normal bırakma — hemen sil
            self.keys_pressed.remove(&key);
        }
        // else: fast-tap — begin_frame()'de silinecek
    }

    /// Basılı tüm tuş ve fare düğmelerini bırakılmış sayar (odak kaybı için).
    ///
    /// Pencere/canvas odağı kaybettiğinde (Alt-Tab, tarayıcı sekmesi değişimi)
    /// işletim sistemi artık key-up olayı GÖNDERMEZ → o an basılı olan tuşlar
    /// sonsuza dek "basılı" kalır ve kamera/karakter kayıp gider. Bu, tüm
    /// basılı durumları temizler; hâlâ fiziksel olarak basılı bir tuş, odak
    /// geri gelince yeni bir key-down ile yeniden kaydolur.
    pub fn release_all(&mut self) {
        for k in self.keys_pressed.drain() {
            self.keys_just_released.insert(k);
        }
        for b in self.mouse_buttons_pressed.drain() {
            self.mouse_buttons_just_released.insert(b);
        }
        self.mouse_delta = (0.0, 0.0);
        self.mouse_scroll_delta = 0.0;
    }

    /// Tuş şu an basılı mı? (sürekli kontrol)
    #[inline]
    pub fn is_key_pressed(&self, key: u32) -> bool {
        self.keys_pressed.contains(&key)
    }

    /// Tuş bu frame'de mi basıldı? (tek seferlik tetikleme)
    #[inline]
    pub fn is_key_just_pressed(&self, key: u32) -> bool {
        self.keys_just_pressed.contains(&key)
    }

    /// Tuş bu frame'de mi bırakıldı?
    #[inline]
    pub fn is_key_just_released(&self, key: u32) -> bool {
        self.keys_just_released.contains(&key)
    }

    // ==================== FARE GİRDİSİ ====================

    /// Fare butonu basıldığında çağır (0=Left, 1=Right, 2=Middle)
    pub fn on_mouse_button_pressed(&mut self, button: u32) {
        // See `on_key_pressed`: a re-press cancels a same-frame fast-tap deferral.
        self.mouse_buttons_just_released.remove(&button);
        if self.mouse_buttons_pressed.insert(button) {
            self.mouse_buttons_just_pressed.insert(button);
        }
    }

    /// Fare butonu bırakıldığında çağır
    pub fn on_mouse_button_released(&mut self, button: u32) {
        self.mouse_buttons_just_released.insert(button);
        if !self.mouse_buttons_just_pressed.contains(&button) {
            self.mouse_buttons_pressed.remove(&button);
        }
    }

    /// Fare butonu basılı mı?
    #[inline]
    pub fn is_mouse_button_pressed(&self, button: u32) -> bool {
        self.mouse_buttons_pressed.contains(&button)
    }

    /// Fare butonu bu frame'de mi basıldı?
    #[inline]
    pub fn is_mouse_button_just_pressed(&self, button: u32) -> bool {
        self.mouse_buttons_just_pressed.contains(&button)
    }

    /// Fare butonu bu frame'de mi bırakıldı?
    #[inline]
    pub fn is_mouse_button_just_released(&self, button: u32) -> bool {
        self.mouse_buttons_just_released.contains(&button)
    }

    // ==================== FARE POZİSYONU ====================

    /// Fare ekran pozisyonu değiştiğinde çağır.
    /// Pozisyon farkından delta biriktirilir — `DeviceEvent::MouseMotion`
    /// olmayan platformlarda (web, bazı Linux konfigürasyonları) fallback sağlar.
    pub fn on_mouse_moved(&mut self, x: f32, y: f32) {
        self.mouse_delta.0 += x - self.mouse_position.0;
        self.mouse_delta.1 += y - self.mouse_position.1;
        self.mouse_position = (x, y);
    }

    /// Fare ekran pozisyonunu günceller — delta BİRİKTİRMEZ.
    /// `DeviceEvent::MouseMotion` sağlayan platformlarda (masaüstü) delta o kanaldan
    /// (`on_mouse_delta`) gelir; `CursorMoved` yalnızca mutlak pozisyonu taşımalı,
    /// aksi halde ikisi birden delta'yı İKİ KEZ sayar (2× fare-bakış hassasiyeti).
    pub fn set_mouse_position(&mut self, x: f32, y: f32) {
        self.mouse_position = (x, y);
    }

    /// Fare delta hareketi (DeviceEvent::MouseMotion).
    /// `on_mouse_moved` zaten delta biriktirdiği için, bu metot yalnızca
    /// platform `DeviceEvent::MouseMotion` veriyorsa ek doğruluk sağlar.
    /// İkisi birlikte çağrılmamalı — platform'a göre birini kullanın.
    pub fn on_mouse_delta(&mut self, dx: f32, dy: f32) {
        self.mouse_delta.0 += dx;
        self.mouse_delta.1 += dy;
    }

    /// Fare ekran pozisyonu
    #[inline]
    pub fn mouse_position(&self) -> (f32, f32) {
        self.mouse_position
    }

    /// Bu frame'deki fare hareketi (delta)
    #[inline]
    pub fn mouse_delta(&self) -> (f32, f32) {
        self.mouse_delta
    }

    // ==================== FARE TEKERLEK (SCROLL) ====================

    /// Fare tekerleği hareket ettiğinde çağır.
    /// Pozitif = yukarı/ileri, negatif = aşağı/geri.
    pub fn on_mouse_scroll(&mut self, delta: f32) {
        self.mouse_scroll_delta += delta;
    }

    /// Bu frame'deki fare tekerlek deltası.
    /// Pozitif = yukarı/ileri, negatif = aşağı/geri.
    #[inline]
    pub fn mouse_scroll(&self) -> f32 {
        self.mouse_scroll_delta
    }
}

impl Default for Input {
    fn default() -> Self {
        Self::new()
    }
}

/// Fare buton sabitleri
pub mod mouse {
    pub const LEFT: u32 = 0;
    pub const RIGHT: u32 = 1;
    pub const MIDDLE: u32 = 2;
}

// Action mapping and the fighting-game input buffer live in submodules; re-export so the
// public paths (`input::ActionMap`, `input::InputBinding`, `input::FrameRecord`,
// `input::PlaybackData`, …) and the crate-root `pub use input::{...}` stay unchanged.
mod fighter;
mod mapping;
pub use fighter::{FighterInputBuffer, FrameActions, FrameRecord, PlaybackData};
pub use mapping::{ActionMap, InputBinding};

#[cfg(test)]
mod tests {
    use super::*;

    /// A held key released and re-pressed within the SAME frame must STAY held.
    /// The release defers removal to begin_frame (fast-tap protection); without
    /// cancelling that deferral on the re-press, begin_frame dropped the physically
    /// held key (and it then spuriously re-fired just_pressed on auto-repeat).
    #[test]
    fn fast_tap_release_then_repress_keeps_key_held() {
        let mut input = Input::new();
        input.on_key_pressed(5);
        input.begin_frame(); // 5 is now a plain held key
        assert!(input.is_key_pressed(5));

        // Same frame: release, then immediately re-press.
        input.on_key_released(5);
        input.on_key_pressed(5);
        input.begin_frame();

        assert!(input.is_key_pressed(5), "re-pressed key must stay held");
        assert!(!input.is_key_just_pressed(5), "no spurious just_pressed after begin_frame");
    }

    // ──── Fast-Tap Testleri ────

    #[test]
    fn test_fast_tap_preserves_pressed_for_one_frame() {
        let mut input = Input::new();

        // Aynı frame'de basılıp bırakılan tuş
        input.on_key_pressed(42);
        input.on_key_released(42);

        // O frame boyunca hem pressed hem just_pressed true olmalı
        assert!(input.is_key_pressed(42), "fast-tap: tuş pressed olmalı");
        assert!(
            input.is_key_just_pressed(42),
            "fast-tap: tuş just_pressed olmalı"
        );
        assert!(
            input.is_key_just_released(42),
            "fast-tap: tuş just_released olmalı"
        );

        // Sonraki frame
        input.begin_frame();

        // Artık hiçbiri true olmamalı
        assert!(
            !input.is_key_pressed(42),
            "sonraki frame: pressed false olmalı"
        );
        assert!(
            !input.is_key_just_pressed(42),
            "sonraki frame: just_pressed false olmalı"
        );
        assert!(
            !input.is_key_just_released(42),
            "sonraki frame: just_released false olmalı"
        );
    }

    #[test]
    fn test_normal_press_release_across_frames() {
        let mut input = Input::new();

        // Frame 1: Tuş basıldı
        input.on_key_pressed(10);
        assert!(input.is_key_pressed(10));
        assert!(input.is_key_just_pressed(10));

        // Frame 2: Tuş hâlâ basılı
        input.begin_frame();
        assert!(input.is_key_pressed(10));
        assert!(!input.is_key_just_pressed(10));

        // Frame 3: Tuş bırakıldı
        input.on_key_released(10);
        assert!(!input.is_key_pressed(10)); // Normal bırakma — hemen silinir
        assert!(input.is_key_just_released(10));

        // Frame 4: Temiz
        input.begin_frame();
        assert!(!input.is_key_pressed(10));
        assert!(!input.is_key_just_released(10));
    }

    #[test]
    fn test_fast_tap_mouse_button() {
        let mut input = Input::new();

        input.on_mouse_button_pressed(mouse::LEFT);
        input.on_mouse_button_released(mouse::LEFT);

        assert!(input.is_mouse_button_pressed(mouse::LEFT));
        assert!(input.is_mouse_button_just_pressed(mouse::LEFT));
        assert!(input.is_mouse_button_just_released(mouse::LEFT));

        input.begin_frame();

        assert!(!input.is_mouse_button_pressed(mouse::LEFT));
        assert!(!input.is_mouse_button_just_pressed(mouse::LEFT));
        assert!(!input.is_mouse_button_just_released(mouse::LEFT));
    }

    // ──── Mouse Delta Testleri ────

    #[test]
    fn test_mouse_moved_accumulates_delta() {
        let mut input = Input::new();

        input.on_mouse_moved(100.0, 200.0);
        // İlk hareket: (0,0) → (100,200) = delta (100, 200)
        assert_eq!(input.mouse_delta(), (100.0, 200.0));

        input.on_mouse_moved(150.0, 250.0);
        // İkinci hareket: (100,200) → (150,250) = ek delta (50, 50), toplam (150, 250)
        assert_eq!(input.mouse_delta(), (150.0, 250.0));

        assert_eq!(input.mouse_position(), (150.0, 250.0));
    }

    #[test]
    fn test_mouse_delta_resets_on_begin_frame() {
        let mut input = Input::new();

        input.on_mouse_moved(100.0, 200.0);
        assert_ne!(input.mouse_delta(), (0.0, 0.0));

        input.begin_frame();
        assert_eq!(input.mouse_delta(), (0.0, 0.0));
        // Pozisyon korunmalı
        assert_eq!(input.mouse_position(), (100.0, 200.0));
    }

    // ──── Odak Kaybı (release_all) Testleri ────

    #[test]
    fn test_release_all_clears_held_keys_and_buttons() {
        let mut input = Input::new();
        input.on_key_pressed(65); // 'A' basılı tutuluyor
        input.on_key_pressed(87); // 'W' basılı tutuluyor
        input.on_mouse_button_pressed(1);
        input.on_mouse_moved(10.0, 10.0);
        input.begin_frame(); // just_pressed temizlenir, pressed KALIR
        assert!(input.is_key_pressed(65));
        assert!(input.is_key_pressed(87));
        assert!(input.is_mouse_button_pressed(1));

        // Odak kaybı: OS artık key-up göndermez → release_all hepsini bırakmalı.
        input.release_all();
        assert!(!input.is_key_pressed(65), "A odak kaybından sonra hâlâ basılı");
        assert!(!input.is_key_pressed(87), "W odak kaybından sonra hâlâ basılı");
        assert!(!input.is_mouse_button_pressed(1));
        assert_eq!(input.mouse_delta(), (0.0, 0.0));
        // Bırakma bu frame'de just_released olarak görünür (temiz kenar).
        assert!(input.is_key_just_released(65));
    }

    // ──── Scroll Testleri ────

    #[test]
    fn test_scroll_accumulates_and_resets() {
        let mut input = Input::new();

        input.on_mouse_scroll(3.0);
        input.on_mouse_scroll(-1.0);
        assert_eq!(input.mouse_scroll(), 2.0);

        input.begin_frame();
        assert_eq!(input.mouse_scroll(), 0.0);
    }

    // ──── Pressed Keys ────

    #[test]
    fn test_pressed_keys() {
        let mut input = Input::new();
        input.on_key_pressed(1);
        input.on_key_pressed(2);
        input.on_key_pressed(3);

        let mut keys = input.pressed_keys();
        keys.sort();
        assert_eq!(keys, vec![1, 2, 3]);
    }

    // ──── ActionMap Testleri ────

    #[test]
    fn test_action_map_key_binding() {
        let mut input = Input::new();
        let mut actions = ActionMap::new();
        actions.bind_key("Jump", 42);

        input.on_key_pressed(42);
        assert!(actions.is_action_pressed(&input, "Jump"));
        assert!(actions.is_action_just_pressed(&input, "Jump"));
    }

    #[test]
    fn test_action_map_mouse_binding() {
        let mut input = Input::new();
        let mut actions = ActionMap::new();
        actions.bind_mouse_button("Attack", mouse::LEFT);

        input.on_mouse_button_pressed(mouse::LEFT);
        assert!(actions.is_action_pressed(&input, "Attack"));
        assert!(actions.is_action_just_pressed(&input, "Attack"));

        input.begin_frame();
        input.on_mouse_button_released(mouse::LEFT);
        assert!(actions.is_action_just_released(&input, "Attack"));
    }

    #[test]
    fn test_action_map_mixed_bindings() {
        let mut input = Input::new();
        let mut actions = ActionMap::new();
        actions.bind_key("Fire", 42);
        actions.bind_mouse_button("Fire", mouse::LEFT);

        // Hiçbiri basılı değil
        assert!(!actions.is_action_pressed(&input, "Fire"));

        // Sadece fare basılı
        input.on_mouse_button_pressed(mouse::LEFT);
        assert!(actions.is_action_pressed(&input, "Fire"));

        input.begin_frame();
        input.on_mouse_button_released(mouse::LEFT);

        // Sadece tuş basılı
        input.on_key_pressed(42);
        assert!(actions.is_action_pressed(&input, "Fire"));
    }

    #[test]
    fn test_action_map_just_released() {
        let mut input = Input::new();
        let mut actions = ActionMap::new();
        actions.bind_key("Charge", 99);

        input.on_key_pressed(99);
        input.begin_frame();
        input.on_key_released(99);

        assert!(actions.is_action_just_released(&input, "Charge"));
        assert!(!actions.is_action_pressed(&input, "Charge"));
    }

    #[test]
    fn test_bind_action_backward_compat() {
        let mut actions = ActionMap::new();
        actions.bind_action("Jump", 42); // Eski API
        assert!(matches!(
            actions.bindings.get("Jump").unwrap()[0],
            InputBinding::Key(42)
        ));
    }
}
