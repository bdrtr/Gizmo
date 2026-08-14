//! Per-frame keyboard and mouse state ([`Input`]), logical action names layered on top of it
//! ([`ActionMap`]), and the fighting-game motion buffer and replay records
//! ([`FighterInputBuffer`], [`PlaybackData`]).
//!
//! [`Input`] is a snapshot, not a stream. The platform layer pushes events into it with the
//! `on_*` methods and calls [`Input::begin_frame`] exactly once per frame to roll the
//! edge-triggered "just pressed" / "just released" sets over; skipping or double-calling
//! `begin_frame` is what makes edge queries misfire, not the event methods themselves.
//!
//! Keys and mouse buttons are opaque `u32` codes. `gizmo-core` has no windowing dependency,
//! so the mapping from a physical key to a code is entirely the caller's convention — the
//! examples here spell it `KeyCode as u32` — and it must stay stable, or saved bindings and
//! recorded replays silently start meaning different keys.

pub mod keys;
pub use keys::{code_from_name, NAMED_KEYS};

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Ergonomic input abstraction layer.
///
/// Usage:
/// ```
/// use gizmo_core::prelude::*;
/// // Key codes are the caller's convention — on desktop, winit's `KeyCode as u32`.
/// # enum KeyCode { KeyW = 17, Space = 57 }
///
/// // The platform layer forwards the events:
/// let mut input = Input::new();
/// input.on_key_pressed(KeyCode::KeyW as u32);
/// input.on_key_pressed(KeyCode::Space as u32);
/// input.on_mouse_button_pressed(mouse::LEFT);
/// input.on_mouse_delta(3.0, -2.0);
/// input.on_mouse_scroll(1.0);
///
/// assert!(input.is_key_pressed(KeyCode::KeyW as u32)); // ileri git
/// assert!(input.is_key_just_pressed(KeyCode::Space as u32)); // jump (one-shot)
/// assert!(input.is_mouse_button_pressed(mouse::LEFT)); // fire
/// assert_eq!(input.mouse_delta(), (3.0, -2.0)); // fare hareketi
/// assert_eq!(input.mouse_scroll(), 1.0); // tekerlek
///
/// // `begin_frame` only clears the edge-triggered queries; a held key stays held.
/// input.begin_frame();
/// assert!(input.is_key_pressed(KeyCode::KeyW as u32));
/// assert!(!input.is_key_just_pressed(KeyCode::Space as u32));
/// assert_eq!(input.mouse_delta(), (0.0, 0.0));
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
    /// Creates an empty input state: nothing held, no pending edges, zero scroll, and the
    /// cursor at `(0.0, 0.0)`.
    ///
    /// That cursor origin is a real value, not "unknown". [`Input::on_mouse_moved`] derives
    /// its delta from the previous position, so the very first `on_mouse_moved(x, y)` after
    /// construction reports a delta of the full `(x, y)` — one large spurious flick, which a
    /// mouse-look camera will act on. Seed the position with [`Input::set_mouse_position`]
    /// first when that matters.
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

    /// Must be called at the start of every frame — clears the "just pressed/released" sets
    /// and performs the deferred key releases.
    ///
    /// Logic:
    /// - `on_key_released()` has deferred the removal from `keys_pressed` for keys pressed
    ///   and released in the same frame (fast-tap protection). `begin_frame()` performs
    ///   these deferred removals.
    /// - Then the just_pressed and just_released sets are cleared, the mouse deltas are zeroed.
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

    /// Returns all pressed keys (for Debug)
    pub fn pressed_keys(&self) -> Vec<u32> {
        self.keys_pressed.iter().copied().collect()
    }

    /// Call when a key is pressed (winit KeyCode's scan code)
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

    /// Call when a key is released.
    ///
    /// If the key was pressed and released in the same frame (if it is in `keys_just_pressed`),
    /// it defers the removal from `keys_pressed` to `begin_frame()`. This way the game does not
    /// miss this "fast tap" — both `is_key_pressed` and `is_key_just_pressed` return true
    /// throughout that frame.
    pub fn on_key_released(&mut self, key: u32) {
        self.keys_just_released.insert(key);
        if !self.keys_just_pressed.contains(&key) {
            // Normal bırakma — hemen sil
            self.keys_pressed.remove(&key);
        }
        // else: fast-tap — begin_frame()'de silinecek
    }

    /// Counts all pressed keys and mouse buttons as released (for focus loss).
    ///
    /// When the window/canvas loses focus (Alt-Tab, browser tab change) the
    /// operating system NO LONGER SENDS a key-up event → the keys pressed at
    /// that moment stay "pressed" forever and the camera/character drifts
    /// away. This clears all pressed states; a key that is still physically
    /// pressed registers again with a new key-down when focus comes back.
    ///
    /// Not-yet-consumed "just pressed" edges are cancelled too — a key pressed on this frame
    /// does not trigger the one-shot action (jump/fire) after the focus loss.
    /// The release edge (`is_key_just_released`), on the other hand, is reported: that is the
    /// method's purpose.
    pub fn release_all(&mut self) {
        for k in self.keys_pressed.drain() {
            self.keys_just_released.insert(k);
        }
        for b in self.mouse_buttons_pressed.drain() {
            self.mouse_buttons_just_released.insert(b);
        }
        // Bekleyen BASMA kenarları da gitmeli, yalnız basılı durum değil. Diğer tüm
        // yollarda `just_pressed ⊆ pressed` tutar (`on_key_pressed` kenarı yalnız
        // `keys_pressed`'e ekleme YENİ ise kaydeder; fast-tap ertelemesi de tuşu
        // `keys_pressed`'de tutar). Yalnız `keys_pressed`'i boşaltmak bu değişmezi
        // kırıyordu: son `begin_frame`'den beri basılan tuş için `is_key_just_pressed()`
        // true kalırken `is_key_pressed()` false oluyordu.
        //
        // Bu durum teorik değil, gözlemleniyor: pencereli döngü `begin_frame()`'i redraw
        // işleyicisinin SONUNDA çağırıyor (gizmo-app windowed/event.rs:692), yani
        // key-down'dan sonra gelen `Focused(false)` → `release_all()` bir SONRAKİ frame'in
        // sistemleri tarafından görülüyor ve odağın kaybedildiği frame'de tek-seferlik
        // aksiyon tetikleniyordu — release_all'ın amacının tam tersi.
        self.keys_just_pressed.clear();
        self.mouse_buttons_just_pressed.clear();
        self.mouse_delta = (0.0, 0.0);
        self.mouse_scroll_delta = 0.0;
    }

    /// Is the key pressed right now? (continuous check)
    #[inline]
    pub fn is_key_pressed(&self, key: u32) -> bool {
        self.keys_pressed.contains(&key)
    }

    /// Was the key pressed on this frame? (one-shot trigger)
    #[inline]
    pub fn is_key_just_pressed(&self, key: u32) -> bool {
        self.keys_just_pressed.contains(&key)
    }

    /// Was the key released on this frame?
    #[inline]
    pub fn is_key_just_released(&self, key: u32) -> bool {
        self.keys_just_released.contains(&key)
    }

    // ==================== FARE GİRDİSİ ====================

    /// Call when a mouse button is pressed (0=Left, 1=Right, 2=Middle)
    pub fn on_mouse_button_pressed(&mut self, button: u32) {
        // See `on_key_pressed`: a re-press cancels a same-frame fast-tap deferral.
        self.mouse_buttons_just_released.remove(&button);
        if self.mouse_buttons_pressed.insert(button) {
            self.mouse_buttons_just_pressed.insert(button);
        }
    }

    /// Call when a mouse button is released
    pub fn on_mouse_button_released(&mut self, button: u32) {
        self.mouse_buttons_just_released.insert(button);
        if !self.mouse_buttons_just_pressed.contains(&button) {
            self.mouse_buttons_pressed.remove(&button);
        }
    }

    /// Is the mouse button pressed?
    #[inline]
    pub fn is_mouse_button_pressed(&self, button: u32) -> bool {
        self.mouse_buttons_pressed.contains(&button)
    }

    /// Was the mouse button pressed on this frame?
    #[inline]
    pub fn is_mouse_button_just_pressed(&self, button: u32) -> bool {
        self.mouse_buttons_just_pressed.contains(&button)
    }

    /// Was the mouse button released on this frame?
    #[inline]
    pub fn is_mouse_button_just_released(&self, button: u32) -> bool {
        self.mouse_buttons_just_released.contains(&button)
    }

    // ==================== FARE POZİSYONU ====================

    /// Call when the mouse screen position changes.
    /// The delta is accumulated from the position difference — it provides a fallback on
    /// platforms without `DeviceEvent::MouseMotion` (web, some Linux configurations).
    pub fn on_mouse_moved(&mut self, x: f32, y: f32) {
        self.mouse_delta.0 += x - self.mouse_position.0;
        self.mouse_delta.1 += y - self.mouse_position.1;
        self.mouse_position = (x, y);
    }

    /// Updates the mouse screen position — DOES NOT ACCUMULATE a delta.
    /// On platforms that provide `DeviceEvent::MouseMotion` (desktop) the delta comes from
    /// that channel (`on_mouse_delta`); `CursorMoved` must carry only the absolute position,
    /// otherwise the two of them together count the delta TWICE (2× mouse-look sensitivity).
    pub fn set_mouse_position(&mut self, x: f32, y: f32) {
        self.mouse_position = (x, y);
    }

    /// Mouse delta movement (DeviceEvent::MouseMotion).
    /// Since `on_mouse_moved` already accumulates the delta, this method only
    /// provides extra accuracy if the platform supplies `DeviceEvent::MouseMotion`.
    /// The two must not be called together — use one of them according to the platform.
    pub fn on_mouse_delta(&mut self, dx: f32, dy: f32) {
        self.mouse_delta.0 += dx;
        self.mouse_delta.1 += dy;
    }

    /// Mouse screen position
    #[inline]
    pub fn mouse_position(&self) -> (f32, f32) {
        self.mouse_position
    }

    /// The mouse movement on this frame (delta)
    #[inline]
    pub fn mouse_delta(&self) -> (f32, f32) {
        self.mouse_delta
    }

    // ==================== FARE TEKERLEK (SCROLL) ====================

    /// Call when the mouse wheel moves.
    /// Positive = up/forward, negative = down/back.
    pub fn on_mouse_scroll(&mut self, delta: f32) {
        self.mouse_scroll_delta += delta;
    }

    /// The mouse wheel delta on this frame.
    /// Positive = up/forward, negative = down/back.
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

/// Mouse button constants
///
/// The canonical codes for the three standard mouse buttons — the names to use on both sides
/// of the API rather than writing the literals.
///
/// The set is not exhaustive: `Input` stores a button as an opaque `u32`, so codes outside
/// this module (side buttons, tilt wheels) work just as well; there is simply no constant for
/// them here. What matters is that the code the platform layer passes to
/// `on_mouse_button_pressed` is the same one the game queries with.
pub mod mouse {
    /// Primary (left) button.
    ///
    /// Its code is `0`, which is also what an uninitialised or defaulted `u32` holds: there is
    /// no "no button" sentinel in this API, so a button code that was never assigned reads as
    /// a left click rather than as nothing.
    pub const LEFT: u32 = 0;
    /// Secondary button — the one a context menu or alternate fire hangs off.
    pub const RIGHT: u32 = 1;
    /// Wheel click, code `2`. Distinct from wheel *rotation*, which is not a button at all
    /// and arrives through [`Input::on_mouse_scroll`](super::Input::on_mouse_scroll).
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

    /// Focus loss must cancel PENDING press edges too, not just the pressed state.
    ///
    /// `begin_frame()` is called at the END of the frame (gizmo-app windowed/event.rs:692), so
    /// in the "key-down, then `Focused(false)`" sequence the key was still inside
    /// `keys_just_pressed` when the next frame's systems ran: jump/fire was being triggered on
    /// the frame where the window was lost. Moreover, while `just_pressed == true` it left
    /// `pressed == false` — a state no other path could produce (in the fast-tap deferral the
    /// key STAYS in `keys_pressed`, see the test above).
    #[test]
    fn release_all_cancels_pending_just_pressed_edges() {
        let mut input = Input::new();
        input.on_key_pressed(32); // Space — bu frame içinde basıldı, henüz tüketilmedi
        input.on_mouse_button_pressed(mouse::LEFT);
        assert!(input.is_key_just_pressed(32));
        assert!(input.is_mouse_button_just_pressed(mouse::LEFT));

        input.release_all(); // odak kaybı, frame'in sistemleri daha koşmadan

        assert!(
            !input.is_key_just_pressed(32),
            "odak kaybı bekleyen basma kenarını iptal etmeli"
        );
        assert!(
            !input.is_mouse_button_just_pressed(mouse::LEFT),
            "fare düğmesi için de aynısı"
        );
        assert!(!input.is_key_pressed(32));
        // Bırakma kenarı RAPORLANMALI — release_all'ın amacı bu.
        assert!(input.is_key_just_released(32));
        assert!(input.is_mouse_button_just_released(mouse::LEFT));

        // Odak geri gelince fiziksel olarak basılı tuş yeni bir key-down ile TAZE kenar alır.
        input.on_key_pressed(32);
        assert!(input.is_key_pressed(32) && input.is_key_just_pressed(32));
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
