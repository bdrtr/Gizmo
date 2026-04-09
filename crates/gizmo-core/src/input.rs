use std::collections::HashSet;

/// Ergonomik input soyutlama katmanı.
/// 
/// Kullanım:
/// ```rust,ignore
/// if input.is_key_pressed(KeyCode::KeyW) { /* ileri git */ }
/// if input.is_key_just_pressed(KeyCode::Space) { /* zıpla (tek sefer) */ }
/// if input.is_mouse_button_pressed(MouseButton::Left) { /* ateş et */ }
/// let (dx, dy) = input.mouse_delta(); /* fare hareketi */
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

    // Fare pozisyonu
    mouse_position: (f32, f32),
    mouse_delta: (f32, f32),

    // Pencere boyutu
    window_size: (f32, f32),
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
            window_size: (1280.0, 720.0),
        }
    }

    // ==================== FRAME YAŞAM DÖNGÜSÜ ====================

    /// Her frame başında çağrılmalı — "just pressed/released" setlerini temizler.
    pub fn begin_frame(&mut self) {
        self.keys_just_pressed.clear();
        self.keys_just_released.clear();
        self.mouse_buttons_just_pressed.clear();
        self.mouse_buttons_just_released.clear();
        self.mouse_delta = (0.0, 0.0);
    }

    // ==================== TUŞ GİRDİSİ ====================

    /// Basılı tüm tuşları döndürür (Debug için)
    pub fn get_pressed_keys(&self) -> Vec<u32> {
        self.keys_pressed.iter().copied().collect()
    }

    /// Tuş basıldığında çağır (winit KeyCode'un scan code'u)
    pub fn on_key_pressed(&mut self, key: u32) {
        if self.keys_pressed.insert(key) {
            self.keys_just_pressed.insert(key);
        }
    }

    /// Tuş bırakıldığında çağır
    pub fn on_key_released(&mut self, key: u32) {
        self.keys_pressed.remove(&key);
        self.keys_just_released.insert(key);
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
        self.mouse_buttons_pressed.remove(&button);
        self.mouse_buttons_just_released.insert(button);
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

    /// Fare hareketi olduğunda çağır
    pub fn on_mouse_moved(&mut self, x: f32, y: f32) {
        self.mouse_position = (x, y);
    }

    /// Fare delta hareketi (DeviceEvent::MouseMotion)
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

    // ==================== PENCERE ====================

    /// Pencere boyutu değiştiğinde çağır
    pub fn on_window_resized(&mut self, width: f32, height: f32) {
        self.window_size = (width, height);
    }

    /// Pencere boyutu
    #[inline]
    pub fn window_size(&self) -> (f32, f32) {
        self.window_size
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

/// Evrensel Girdi Çevirici. 
/// "W" veya "Yukarı Ok" tuşlarını doğrudan kontrol etmek yerine, 
/// "Accelerate" veya "Jump" gibi mantıksal isimlendirmelerle dinlememizi sağlar.
#[derive(Clone)]
pub struct ActionMap {
    bindings: HashMap<String, Vec<u32>>,
}

impl ActionMap {
    pub fn new() -> Self {
        Self {
            bindings: HashMap::new(),
        }
    }

    /// Bir isme (Action) yeni bir tuş kodu bağlar
    pub fn bind_action(&mut self, action_name: &str, keycode: u32) {
        self.bindings
            .entry(action_name.to_string())
            .or_insert_with(Vec::new)
            .push(keycode);
    }

    /// Action (eylem) şu an uygulanıyor mu? (Basılı tutuluyor mu)
    pub fn is_action_pressed(&self, input: &Input, action_name: &str) -> bool {
        if let Some(keys) = self.bindings.get(action_name) {
            for &k in keys {
                if input.is_key_pressed(k) { return true; }
            }
        }
        false
    }

    /// Action bu frame'de yeni mi tetiklendi?
    pub fn is_action_just_pressed(&self, input: &Input, action_name: &str) -> bool {
        if let Some(keys) = self.bindings.get(action_name) {
            for &k in keys {
                if input.is_key_just_pressed(k) { return true; }
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
