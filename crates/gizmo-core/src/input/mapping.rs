//! Action mapping: `InputBinding` (key/mouse) and the `ActionMap` resource that resolves named
//! actions against `Input`. Extracted verbatim from input.rs (pure move).

use super::*;
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
    // `pub(super)` (visible within the `input` module) so the action-map tests in `input/mod.rs`
    // can assert on the resolved bindings; not part of the public API.
    pub(super) bindings: HashMap<String, Vec<InputBinding>>,
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
