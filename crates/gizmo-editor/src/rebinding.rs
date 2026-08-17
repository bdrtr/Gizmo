//! The rebinding panel: an [`ActionMap`] shown as rows a player can change.
//!
//! # What this closes
//!
//! `NAMED_GAMEPAD_BUTTONS` and `NAMED_KEYS` existed "so a config file can name controls", and
//! nothing consumed them — the names named things no UI could show and no file could hold.
//! `gizmo_core::input::binding_names` is the file half; this is the UI half.
//!
//! # The capture state is the whole design
//!
//! A rebinding panel is a state machine with exactly one interesting state: *listening*. While it
//! is listening, the next control the player touches becomes a binding — which means the click
//! that started listening must not be captured, and neither must the key they used to reach the
//! button. [`RebindState::listening_for`] is that state, and it is deliberately **one at a time**:
//! two rows listening at once would both capture the same press, which is not a thing a player can
//! have meant.
//!
//! The panel takes the map and the live [`Input`] and returns whether anything changed, rather
//! than owning either. An editor panel, a game's own settings screen and a test all want the same
//! rows over their own map; owning one would make it the editor's map and nobody else's.

use gizmo_core::input::{ActionMap, Input, InputBinding};

/// Which row, if any, is waiting for the player to press something.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RebindState {
    /// The action currently listening for a control, and how many frames it has been listening.
    ///
    /// The frame count is not decoration: the click that starts listening lands in the same
    /// `Input` the panel is about to read, so a capture on frame zero would bind the mouse button
    /// the player used to press "Rebind". One frame of grace is the whole fix, and it is why this
    /// is a `(String, u32)` rather than a bare `Option<String>`.
    pub listening_for: Option<(String, u32)>,
}

impl RebindState {
    /// Starts listening for `action`, replacing whatever was listening before.
    pub fn listen(&mut self, action: &str) {
        self.listening_for = Some((action.to_string(), 0));
    }

    /// Stops listening without binding anything.
    pub fn cancel(&mut self) {
        self.listening_for = None;
    }

    /// Is this action the one listening?
    #[must_use]
    pub fn is_listening_for(&self, action: &str) -> bool {
        self.listening_for
            .as_ref()
            .is_some_and(|(a, _)| a == action)
    }

    /// Advances the listening state by one frame and captures a control if one arrives.
    ///
    /// Returns the action that was just bound, if any. Pure apart from the map it edits, so a test
    /// can drive it with a hand-built [`Input`] and no window.
    ///
    /// **The first frame never captures**, for the reason on [`Self::listening_for`]. Escape
    /// cancels, and is checked before capture so it can never be bound by the act of cancelling.
    pub fn poll(&mut self, map: &mut ActionMap, input: &Input) -> Option<String> {
        let (action, frames) = self.listening_for.take()?;

        // Escape cancels. Checked first: otherwise the key that means "stop listening" is the key
        // that gets bound, every time.
        if input.is_key_just_pressed(escape_code()) {
            return None;
        }

        if frames == 0 {
            self.listening_for = Some((action, 1));
            return None;
        }

        match InputBinding::captured_from(input) {
            Some(binding) => {
                map.add_binding(&action, binding);
                Some(action)
            }
            None => {
                self.listening_for = Some((action, frames + 1));
                None
            }
        }
    }
}

/// Escape, in the desktop key-code convention.
///
/// Tied to the verified table rather than written as a literal — a second set of key numbers is
/// how the Lua API came to hold USB HID codes, where the arrow keys meant each other. The test
/// below is what keeps this honest.
fn escape_code() -> u32 {
    gizmo_core::input::code_from_name("escape").expect("`escape` is in NAMED_KEYS")
}

/// Draws the rebinding rows for `map`, and returns whether a binding changed.
///
/// One row per action, sorted, with each binding shown by name and a ✖ to remove it. Sorted
/// because a settings screen that reorders itself between frames is unusable, and the same reason
/// [`ActionMap::to_named`] returns a `BTreeMap`.
pub fn ui_rebinding_panel(
    ui: &mut egui::Ui,
    map: &mut ActionMap,
    state: &mut RebindState,
    input: &Input,
) -> bool {
    let mut changed = state.poll(map, input).is_some();

    let named = map.to_named();
    if named.is_empty() {
        ui.label("Bu haritada eylem yok. / No actions in this map.");
        return changed;
    }

    for (action, bindings) in &named {
        ui.horizontal(|ui| {
            ui.label(action);
            for name in bindings {
                if ui
                    .small_button(format!("{name} ✖"))
                    .on_hover_text("Bu bağlamayı kaldır / Remove this binding")
                    .clicked()
                {
                    map.remove_binding_named(action, name);
                    changed = true;
                }
            }
            if state.is_listening_for(action) {
                if ui
                    .button("⌨ bir tuşa bas… / press a control…")
                    .on_hover_text("Esc iptal eder / Esc cancels")
                    .clicked()
                {
                    state.cancel();
                }
            } else if ui.button("➕ Bağla / Bind").clicked() {
                state.listen(action);
            }
        });
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_core::input::{code_from_name, mouse, GamepadButton, GamepadId};

    fn map_with_jump() -> ActionMap {
        let mut map = ActionMap::new();
        map.bind_key("jump", code_from_name("space").unwrap());
        map
    }

    /// The click that starts listening must not become the binding. This is the whole reason the
    /// listening state counts frames.
    #[test]
    fn the_click_that_started_listening_is_not_captured() {
        let mut map = map_with_jump();
        let mut state = RebindState::default();
        state.listen("jump");

        let mut input = Input::new();
        input.on_mouse_button_pressed(mouse::LEFT); // the click on "Bind"
        assert_eq!(state.poll(&mut map, &input), None, "frame zero must not capture");
        assert_eq!(
            map.to_named()["jump"],
            vec!["key:space".to_string()],
            "the binding must be untouched"
        );

        // Next frame, the same click is no longer an edge, so still nothing.
        input.begin_frame();
        assert_eq!(state.poll(&mut map, &input), None);
        assert!(state.is_listening_for("jump"), "and it is still listening");
    }

    #[test]
    fn the_next_control_pressed_becomes_the_binding() {
        let mut map = map_with_jump();
        let mut state = RebindState::default();
        state.listen("jump");

        let mut input = Input::new();
        state.poll(&mut map, &input); // frame zero
        let id = GamepadId::new(0);
        input.on_gamepad_connected(id, "pad");
        input.on_gamepad_button_pressed(id, GamepadButton::East);

        assert_eq!(state.poll(&mut map, &input).as_deref(), Some("jump"));
        assert_eq!(
            map.to_named()["jump"],
            vec!["key:space".to_string(), "pad:east".to_string()],
            "binding a pad control must not cost the keyboard one — see `add_binding`"
        );
        assert!(!state.is_listening_for("jump"), "capture ends the listening state");
    }

    /// Escape must cancel rather than bind itself — the key that means "stop" cannot be the key
    /// that gets stored.
    #[test]
    fn escape_cancels_instead_of_binding_escape() {
        let mut map = map_with_jump();
        let mut state = RebindState::default();
        state.listen("jump");

        let mut input = Input::new();
        state.poll(&mut map, &input); // frame zero
        input.on_key_pressed(escape_code());

        assert_eq!(state.poll(&mut map, &input), None);
        assert!(!state.is_listening_for("jump"), "listening must stop");
        assert_eq!(
            map.to_named()["jump"],
            vec!["key:space".to_string()],
            "and Escape must not have been bound"
        );
    }

    #[test]
    fn escape_is_the_code_the_verified_table_gives() {
        assert_eq!(escape_code(), 114, "KeyCode::Escape in the desktop convention");
    }

    /// Two rows listening at once would both capture the same press, which is not a thing a
    /// player can have meant.
    #[test]
    fn only_one_action_listens_at_a_time() {
        let mut state = RebindState::default();
        state.listen("jump");
        state.listen("fire");
        assert!(state.is_listening_for("fire"));
        assert!(!state.is_listening_for("jump"));
    }

    /// The panel draws headlessly, which is what makes any of this testable at all.
    #[test]
    fn the_panel_draws_a_row_per_action() {
        let mut map = map_with_jump();
        map.bind_key("fire", code_from_name("f").unwrap());
        let mut state = RebindState::default();
        let input = Input::new();

        let ctx = egui::Context::default();
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui_rebinding_panel(ui, &mut map, &mut state, &input);
        });

        let mut texts = Vec::new();
        collect_text(&output.shapes, &mut texts);
        for expected in ["jump", "fire", "key:space", "key:f"] {
            assert!(
                texts.iter().any(|t| t.contains(expected)),
                "`{expected}` is not on screen; saw {texts:?}"
            );
        }
        output.drop_without_applying_deltas();
    }

    #[test]
    fn an_empty_map_says_so_rather_than_drawing_nothing() {
        let mut map = ActionMap::new();
        let mut state = RebindState::default();
        let input = Input::new();
        let ctx = egui::Context::default();
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui_rebinding_panel(ui, &mut map, &mut state, &input);
        });
        let mut texts = Vec::new();
        collect_text(&output.shapes, &mut texts);
        assert!(
            texts.iter().any(|t| t.contains("No actions")),
            "an empty panel must explain itself; saw {texts:?}"
        );
        output.drop_without_applying_deltas();
    }

    /// `Shape::Vec` nests, so a scan that does not recurse misses most of the frame.
    fn collect_text(shapes: &[egui::epaint::ClippedShape], out: &mut Vec<String>) {
        fn walk(shape: &egui::Shape, out: &mut Vec<String>) {
            match shape {
                egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
        }
        for clipped in shapes {
            walk(&clipped.shape, out);
        }
    }
}
