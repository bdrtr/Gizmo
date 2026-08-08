//! Action mapping: `InputBinding` (key/mouse) and the `ActionMap` resource that resolves named
//! actions against `Input`. Extracted verbatim from input.rs (pure move).

use super::*;
use std::collections::HashMap;

/// Input binding kind — a keyboard key or a mouse button.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum InputBinding {
    /// Keyboard key (winit KeyCode as u32)
    Key(u32),
    /// Mouse button (mouse::LEFT, mouse::RIGHT, mouse::MIDDLE)
    MouseButton(u32),
}

/// Universal Input Translator.
/// Instead of checking the "W" or "Up Arrow" keys directly, it lets us
/// listen via logical names such as "Accelerate" or "Jump".
///
/// # Example
/// ```
/// use gizmo_core::prelude::*;
/// # enum KeyCode { Space = 57 }
/// let mut actions = ActionMap::new();
/// actions.bind_key("Jump", KeyCode::Space as u32);
/// actions.bind_mouse_button("Attack", mouse::LEFT);
///
/// let mut input = Input::new();
/// input.on_key_pressed(KeyCode::Space as u32);
/// input.on_mouse_button_pressed(mouse::LEFT);
///
/// assert!(actions.is_action_just_pressed(&input, "Jump")); // player.jump()
/// assert!(actions.is_action_pressed(&input, "Attack")); // player.attack()
///
/// // An unbound (e.g. misspelt) name does not panic — it quietly returns `false`.
/// assert!(!actions.is_action_pressed(&input, "Jmup"));
/// ```
#[derive(Clone)]
pub struct ActionMap {
    // `pub(super)` (visible within the `input` module) so the action-map tests in `input/mod.rs`
    // can assert on the resolved bindings; not part of the public API.
    pub(super) bindings: HashMap<String, Vec<InputBinding>>,
}

impl ActionMap {
    /// Creates a map with no bindings at all.
    ///
    /// An unbound action is not an error condition: every `is_action_*` query answers `false`
    /// for a name that was never bound, so a misspelled action name fails silently and
    /// forever rather than panicking. There is no way to ask whether a name is bound.
    pub fn new() -> Self {
        Self {
            bindings: HashMap::new(),
        }
    }

    /// Binds a keyboard key to a name (Action)
    pub fn bind_key(&mut self, action_name: &str, keycode: u32) {
        self.bindings
            .entry(action_name.to_string())
            .or_default()
            .push(InputBinding::Key(keycode));
    }

    /// Binds a mouse button to a name (Action)
    pub fn bind_mouse_button(&mut self, action_name: &str, button: u32) {
        self.bindings
            .entry(action_name.to_string())
            .or_default()
            .push(InputBinding::MouseButton(button));
    }

    /// Backward compatibility — same as `bind_key()`.
    pub fn bind_action(&mut self, action_name: &str, keycode: u32) {
        self.bind_key(action_name, keycode);
    }

    /// Is the Action being applied right now? (Is it being held down)
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

    /// Was the Action newly triggered on this frame?
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

    /// Was the Action released on this frame? (For mechanics such as charge-release, toggle)
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
