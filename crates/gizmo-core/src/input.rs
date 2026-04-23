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
#[derive(Clone)]
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

// ==================== ACTION MAP (Tuş Soyutlama) ====================

use std::collections::HashMap;

/// Girdi binding türü — klavye tuşu veya fare butonu.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum InputBinding {
    /// Klavye tuşu (winit KeyCode as u32)
    Key(u32),
    /// Fare butonu (mouse::LEFT, mouse::RIGHT, mouse::MIDDLE)
    MouseButton(u32),
}

/// Evrensel Girdi Çevirici.
/// "W" veya "Yukarı Ok" tuşlarını doğrudan kontrol etmek yerine,
/// "Accelerate" veya "Jump" gibi mantıksal isimlendirmelerle dinlememizi sağlar.
///
/// # Örnek
/// ```rust,ignore
/// let mut actions = ActionMap::new();
/// actions.bind_key("Jump", KeyCode::Space as u32);
/// actions.bind_mouse_button("Attack", mouse::LEFT);
///
/// if actions.is_action_just_pressed(&input, "Jump") { player.jump(); }
/// if actions.is_action_pressed(&input, "Attack") { player.attack(); }
/// ```
#[derive(Clone)]
pub struct ActionMap {
    bindings: HashMap<String, Vec<InputBinding>>,
}

impl ActionMap {
    pub fn new() -> Self {
        Self {
            bindings: HashMap::new(),
        }
    }

    /// Bir isme (Action) klavye tuşu bağlar
    pub fn bind_key(&mut self, action_name: &str, keycode: u32) {
        self.bindings
            .entry(action_name.to_string())
            .or_default()
            .push(InputBinding::Key(keycode));
    }

    /// Bir isme (Action) fare butonu bağlar
    pub fn bind_mouse_button(&mut self, action_name: &str, button: u32) {
        self.bindings
            .entry(action_name.to_string())
            .or_default()
            .push(InputBinding::MouseButton(button));
    }

    /// Geriye dönük uyumluluk — `bind_key()` ile aynı.
    pub fn bind_action(&mut self, action_name: &str, keycode: u32) {
        self.bind_key(action_name, keycode);
    }

    /// Action (eylem) şu an uygulanıyor mu? (Basılı tutuluyor mu)
    pub fn is_action_pressed(&self, input: &Input, action_name: &str) -> bool {
        if let Some(bindings) = self.bindings.get(action_name) {
            for binding in bindings {
                match binding {
                    InputBinding::Key(k) => {
                        if input.is_key_pressed(*k) {
                            return true;
                        }
                    }
                    InputBinding::MouseButton(b) => {
                        if input.is_mouse_button_pressed(*b) {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    /// Action bu frame'de yeni mi tetiklendi?
    pub fn is_action_just_pressed(&self, input: &Input, action_name: &str) -> bool {
        if let Some(bindings) = self.bindings.get(action_name) {
            for binding in bindings {
                match binding {
                    InputBinding::Key(k) => {
                        if input.is_key_just_pressed(*k) {
                            return true;
                        }
                    }
                    InputBinding::MouseButton(b) => {
                        if input.is_mouse_button_just_pressed(*b) {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    /// Action bu frame'de mi bırakıldı? (Şarj-bırak, toggle gibi mekanikler için)
    pub fn is_action_just_released(&self, input: &Input, action_name: &str) -> bool {
        if let Some(bindings) = self.bindings.get(action_name) {
            for binding in bindings {
                match binding {
                    InputBinding::Key(k) => {
                        if input.is_key_just_released(*k) {
                            return true;
                        }
                    }
                    InputBinding::MouseButton(b) => {
                        if input.is_mouse_button_just_released(*b) {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }
}

impl Default for ActionMap {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
