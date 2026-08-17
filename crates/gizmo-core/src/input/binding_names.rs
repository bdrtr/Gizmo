//! An [`ActionMap`] as text a person can write, and read back.
//!
//! # Why the name tables were not enough
//!
//! [`NAMED_KEYS`](super::NAMED_KEYS) and [`NAMED_GAMEPAD_BUTTONS`](super::NAMED_GAMEPAD_BUTTONS)
//! have existed for a while, "so a config file can name controls". Nothing could write one: an
//! [`ActionMap`] had no way out to text and no way back, so the names named things nobody could
//! save. This module is the missing half, and it is what a rebinding UI needs before it can have
//! anywhere to put its answer.
//!
//! # The grammar, and why it has prefixes
//!
//! One binding is one string:
//!
//! ```text
//! key:w                       a keyboard key, from NAMED_KEYS
//! mouse:left                  left / right / middle
//! pad:south                   a gamepad button, from NAMED_GAMEPAD_BUTTONS
//! axis:left_stick_x+0.5       a stick or trigger past a threshold, in a direction
//! axis:left_stick_y-0.5       …the other end of the same stick
//! ```
//!
//! The `pad:`/`axis:` split is not decoration. `left_trigger` is a name in **both** tables — the
//! same physical control read as a digital press and as analog travel — and the two readings
//! behave differently enough that a config which could not distinguish them would be ambiguous
//! exactly where a driving game cares most. The prefix is what disambiguates, which is the job
//! [`gamepad_button_from_name`](super::gamepad_button_from_name) and
//! [`gamepad_axis_from_name`](super::gamepad_axis_from_name) do in code.
//!
//! # Unknown names are returned, not dropped
//!
//! [`ActionMap::apply_named`] hands back every entry it could not parse instead of skipping it.
//! A typo in a config file is a control that silently does nothing, and "nothing happens when I
//! press jump" is the least diagnosable bug a game can have. The caller decides whether that is
//! a log line, a red row in a settings panel, or a hard error — but it cannot decide if it never
//! hears about it.

use super::{
    gamepad_axis_from_name, gamepad_button_from_name, ActionMap, AxisDirection, Input, InputBinding,
    NAMED_GAMEPAD_AXES, NAMED_GAMEPAD_BUTTONS, NAMED_KEYS,
};
use std::collections::BTreeMap;

/// A binding whose text could not be understood, kept with the action it was written under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownBinding {
    /// The action the entry was written under.
    pub action: String,
    /// The text as written, so a caller can show it back rather than paraphrase it.
    pub text: String,
    /// What was wrong with it, in a form fit to put in front of whoever wrote it.
    pub reason: String,
}

/// The name of a key code, or `None` for a code outside [`NAMED_KEYS`].
fn key_name(code: u32) -> Option<&'static str> {
    NAMED_KEYS
        .iter()
        .find(|(_, c)| *c == code)
        .map(|(name, _)| *name)
}

fn mouse_name(button: u32) -> Option<&'static str> {
    match button {
        super::mouse::LEFT => Some("left"),
        super::mouse::RIGHT => Some("right"),
        super::mouse::MIDDLE => Some("middle"),
        _ => None,
    }
}

fn mouse_from_name(name: &str) -> Option<u32> {
    match name.to_ascii_lowercase().as_str() {
        "left" => Some(super::mouse::LEFT),
        "right" => Some(super::mouse::RIGHT),
        "middle" => Some(super::mouse::MIDDLE),
        _ => None,
    }
}

/// One binding as the text form above, or `None` when it names something the tables do not
/// cover — an unmapped key code, or a mouse button past the middle one.
///
/// `None` rather than a lossy fallback: writing `key:37` into a config that is meant to be
/// human-editable produces a file that only round-trips by accident, and a reader who cannot
/// tell `37` from a typo.
#[must_use]
pub fn binding_to_name(binding: &InputBinding) -> Option<String> {
    Some(match binding {
        InputBinding::Key(code) => format!("key:{}", key_name(*code)?),
        InputBinding::MouseButton(b) => format!("mouse:{}", mouse_name(*b)?),
        InputBinding::GamepadButton(button) => {
            let name = NAMED_GAMEPAD_BUTTONS
                .iter()
                .find(|(_, b)| b == button)
                .map(|(n, _)| *n)?;
            format!("pad:{name}")
        }
        InputBinding::GamepadAxis {
            axis,
            direction,
            threshold,
        } => {
            let name = NAMED_GAMEPAD_AXES
                .iter()
                .find(|(_, a)| a == axis)
                .map(|(n, _)| *n)?;
            let sign = match direction {
                AxisDirection::Positive => '+',
                AxisDirection::Negative => '-',
            };
            format!("axis:{name}{sign}{threshold}")
        }
    })
}

/// The inverse of [`binding_to_name`].
///
/// # Errors
///
/// Returns a sentence describing what was wrong, meant to be shown to whoever wrote the text.
pub fn binding_from_name(text: &str) -> Result<InputBinding, String> {
    let (kind, rest) = text
        .split_once(':')
        .ok_or_else(|| format!("`{text}` has no `kind:` prefix (key, mouse, pad or axis)"))?;
    let rest = rest.trim();
    match kind.trim().to_ascii_lowercase().as_str() {
        "key" => super::code_from_name(rest)
            .map(InputBinding::Key)
            .ok_or_else(|| format!("`{rest}` is not a key name")),
        "mouse" => mouse_from_name(rest)
            .map(InputBinding::MouseButton)
            .ok_or_else(|| format!("`{rest}` is not a mouse button (left, right, middle)")),
        "pad" => gamepad_button_from_name(rest)
            .map(InputBinding::GamepadButton)
            .ok_or_else(|| format!("`{rest}` is not a gamepad button name")),
        "axis" => {
            // The sign is searched for from the END, because an axis name contains no `+`/`-`
            // but a threshold can be written `-0.5`, and splitting at the first sign would cut
            // the name instead.
            let split = rest
                .rfind(['+', '-'])
                .ok_or_else(|| format!("`{rest}` needs a direction and threshold, e.g. `{rest}+0.5`"))?;
            let (name, tail) = rest.split_at(split);
            let direction = match &tail[..1] {
                "+" => AxisDirection::Positive,
                _ => AxisDirection::Negative,
            };
            let threshold: f32 = tail[1..]
                .parse()
                .map_err(|_| format!("`{}` is not a threshold", &tail[1..]))?;
            let axis = gamepad_axis_from_name(name)
                .ok_or_else(|| format!("`{name}` is not a gamepad axis name"))?;
            Ok(InputBinding::GamepadAxis {
                axis,
                direction,
                threshold,
            })
        }
        other => Err(format!(
            "`{other}` is not a binding kind (key, mouse, pad, axis)"
        )),
    }
}

impl ActionMap {
    /// This map as action → binding names, ready to be written to a config file.
    ///
    /// A `BTreeMap` so the output is **ordered**: a settings file that reshuffles itself every
    /// time it is saved produces a diff on every launch and is unreadable in version control.
    ///
    /// Bindings [`binding_to_name`] cannot name are omitted — see there for why that is silence
    /// rather than a lossy encoding. An action whose every binding is unnameable still appears,
    /// with an empty list, so the round trip does not quietly forget that the action exists.
    #[must_use]
    pub fn to_named(&self) -> BTreeMap<String, Vec<String>> {
        self.bindings
            .iter()
            .map(|(action, bindings)| {
                (
                    action.clone(),
                    bindings.iter().filter_map(binding_to_name).collect(),
                )
            })
            .collect()
    }

    /// Adds one binding to an action, keeping what is already there.
    ///
    /// **Adds rather than replaces**, because an action having several bindings is the normal
    /// case — a game bound to both a key and a pad button is one action with two bindings, not
    /// two actions. A rebinding UI's "Bind" therefore grows the list and its ✖ shrinks it, which
    /// is also what makes "add a controller binding without losing my keyboard one" expressible.
    ///
    /// A binding already present is not added twice: a duplicate does nothing on read and would
    /// show as two identical rows.
    pub fn add_binding(&mut self, action: &str, binding: InputBinding) {
        let list = self.bindings.entry(action.to_string()).or_default();
        if !list.contains(&binding) {
            list.push(binding);
        }
    }

    /// Removes the binding whose name is `name` — the text form from [`binding_to_name`], which
    /// is what a UI has to hand when the player clicks the ✖ next to a row.
    ///
    /// Returns whether anything was removed. An action left with no bindings **stays in the map**
    /// with an empty list: it is still an action the game asks about, and dropping it would make
    /// "unbind everything" indistinguishable from "this action does not exist", so the row would
    /// vanish from the settings screen and could never be bound again.
    pub fn remove_binding_named(&mut self, action: &str, name: &str) -> bool {
        let Some(list) = self.bindings.get_mut(action) else {
            return false;
        };
        let before = list.len();
        list.retain(|b| binding_to_name(b).as_deref() != Some(name));
        before != list.len()
    }

    /// Replaces every binding in this map with the named ones, returning whatever could not be
    /// understood.
    ///
    /// **Replaces, not merges.** A config file is the whole answer to "what is bound"; merging
    /// would mean a binding removed from the file stays live until the next fresh start, which is
    /// the bug where a player unbinds a key and it keeps firing.
    ///
    /// ```
    /// # use gizmo_core::prelude::*;
    /// # use std::collections::BTreeMap;
    /// let mut map = ActionMap::new();
    /// let mut config = BTreeMap::new();
    /// config.insert("jump".to_string(), vec!["key:space".to_string(), "pad:south".to_string()]);
    /// config.insert("fire".to_string(), vec!["mouse:left".to_string(), "axis:right_trigger+0.5".to_string()]);
    ///
    /// let unknown = map.apply_named(&config);
    /// assert!(unknown.is_empty());
    /// assert_eq!(map.to_named(), config); // round trip
    /// ```
    pub fn apply_named(
        &mut self,
        named: &BTreeMap<String, Vec<String>>,
    ) -> Vec<UnknownBinding> {
        self.bindings.clear();
        let mut unknown = Vec::new();
        for (action, texts) in named {
            let mut resolved = Vec::new();
            for text in texts {
                match binding_from_name(text) {
                    Ok(binding) => resolved.push(binding),
                    Err(reason) => unknown.push(UnknownBinding {
                        action: action.clone(),
                        text: text.clone(),
                        reason,
                    }),
                }
            }
            self.bindings.insert(action.clone(), resolved);
        }
        unknown
    }
}

impl InputBinding {
    /// The control the player has just pressed, as a binding — the read a rebinding UI is built
    /// on ("press a control now").
    ///
    /// Three decisions, each of which a naive version gets wrong:
    ///
    /// * **Edges, not held state.** A binding is captured from a *just-pressed* key or button, so
    ///   the click that opened the dialog does not bind itself and a key the player happens to be
    ///   leaning on is not offered. An axis has no press edge, so it is read against a threshold
    ///   instead — see below.
    /// * **The threshold is a default, not the reading.** A stick captured at 0.62 of travel would
    ///   bind an action that fires only past 0.62, which is not what pushing a stick meant.
    ///   `AXIS_CAPTURE_THRESHOLD` is what gets stored, and the push only has to clear
    ///   `AXIS_CAPTURE_PUSH` to be recognised as deliberate.
    /// * **One answer, in a fixed order.** A player pressing a key while a stick rests just past
    ///   its deadzone must get the key. Keyboard, then mouse, then pad buttons, then axes — most
    ///   specific intent first.
    ///
    /// Returns `None` when nothing was pressed this frame, which is the normal case and not an
    /// error: a capture UI calls this every frame until it answers.
    #[must_use]
    pub fn captured_from(input: &Input) -> Option<Self> {
        /// How far a stick must be pushed for the push to count as deliberate.
        const AXIS_CAPTURE_PUSH: f32 = 0.7;
        /// What gets STORED as the binding's threshold, whatever the push measured.
        const AXIS_CAPTURE_THRESHOLD: f32 = 0.5;

        if let Some(key) = input.pressed_keys().into_iter().find(|k| input.is_key_just_pressed(*k)) {
            return Some(Self::Key(key));
        }
        for button in [super::mouse::LEFT, super::mouse::RIGHT, super::mouse::MIDDLE] {
            if input.is_mouse_button_just_pressed(button) {
                return Some(Self::MouseButton(button));
            }
        }
        let pad = input.gamepad()?;
        for (_, button) in NAMED_GAMEPAD_BUTTONS {
            if pad.is_just_pressed(*button) {
                return Some(Self::GamepadButton(*button));
            }
        }
        for (_, axis) in NAMED_GAMEPAD_AXES {
            let value = pad.axis(*axis);
            if value.abs() >= AXIS_CAPTURE_PUSH {
                return Some(Self::GamepadAxis {
                    axis: *axis,
                    direction: if value > 0.0 {
                        AxisDirection::Positive
                    } else {
                        AxisDirection::Negative
                    },
                    threshold: AXIS_CAPTURE_THRESHOLD,
                });
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::{GamepadAxis, GamepadButton, Input};

    fn round_trip(binding: InputBinding) {
        let name = binding_to_name(&binding).expect("nameable");
        let back = binding_from_name(&name).expect("parses");
        assert_eq!(back, binding, "via `{name}`");
    }

    #[test]
    fn every_kind_of_binding_round_trips() {
        round_trip(InputBinding::Key(crate::input::code_from_name("w").unwrap()));
        round_trip(InputBinding::MouseButton(crate::input::mouse::RIGHT));
        round_trip(InputBinding::GamepadButton(GamepadButton::DPadLeft));
        round_trip(InputBinding::GamepadAxis {
            axis: GamepadAxis::LeftStickY,
            direction: AxisDirection::Negative,
            threshold: 0.25,
        });
    }

    /// The whole reason the grammar has prefixes: `left_trigger` is in both tables.
    #[test]
    fn the_same_control_can_be_a_button_or_an_axis_and_the_prefix_says_which() {
        assert_eq!(
            binding_from_name("pad:left_trigger").unwrap(),
            InputBinding::GamepadButton(GamepadButton::LeftTrigger)
        );
        assert_eq!(
            binding_from_name("axis:left_trigger+0.5").unwrap(),
            InputBinding::GamepadAxis {
                axis: GamepadAxis::LeftTrigger,
                direction: AxisDirection::Positive,
                threshold: 0.5,
            }
        );
    }

    /// The parser splits the sign from the RIGHT. An axis name has no sign in it, but a
    /// threshold can be written `-0.5`, and splitting at the first one would cut the name.
    #[test]
    fn a_negative_direction_does_not_cut_the_axis_name() {
        let b = binding_from_name("axis:right_stick_x-0.75").unwrap();
        assert_eq!(
            b,
            InputBinding::GamepadAxis {
                axis: GamepadAxis::RightStickX,
                direction: AxisDirection::Negative,
                threshold: 0.75,
            }
        );
    }

    #[test]
    fn a_typo_is_reported_rather_than_dropped() {
        let mut map = ActionMap::new();
        let mut config = BTreeMap::new();
        config.insert(
            "jump".to_string(),
            vec!["key:spcae".to_string(), "pad:south".to_string()],
        );
        let unknown = map.apply_named(&config);
        assert_eq!(unknown.len(), 1, "the typo must come back: {unknown:?}");
        assert_eq!(unknown[0].action, "jump");
        assert_eq!(unknown[0].text, "key:spcae");
        assert!(
            unknown[0].reason.contains("spcae"),
            "the message must quote what was written: {}",
            unknown[0].reason
        );
        // …and the binding that WAS understood still took effect. A file with one typo in it is
        // not a file to throw away.
        assert_eq!(map.to_named()["jump"], vec!["pad:south".to_string()]);
    }

    #[test]
    fn every_malformed_shape_is_a_message_and_not_a_panic() {
        for text in [
            "w",                       // no prefix
            "keyboard:w",              // unknown kind
            "key:",                    // empty name
            "mouse:side",              // not a mouse button this engine names
            "pad:triangle",            // a PlayStation name; this engine is position-named
            "axis:left_stick_x",       // no direction
            "axis:left_stick_x+lots",  // threshold is not a number
            "axis:nonsense+0.5",       // not an axis
        ] {
            let err = binding_from_name(text).expect_err("must not parse: {text}");
            assert!(!err.is_empty(), "`{text}` produced an empty message");
        }
    }

    /// Applying a config REPLACES: an action left out of the file must stop being bound, or a
    /// player who unbinds something keeps firing it until the next fresh start.
    #[test]
    fn applying_a_config_replaces_rather_than_merges() {
        let mut map = ActionMap::new();
        map.bind_key("old", crate::input::code_from_name("q").unwrap());

        let mut config = BTreeMap::new();
        config.insert("new".to_string(), vec!["key:e".to_string()]);
        map.apply_named(&config);

        let named = map.to_named();
        assert!(!named.contains_key("old"), "a removed action stayed bound: {named:?}");
        assert!(named.contains_key("new"));
    }

    /// The output is ordered, so a settings file does not produce a diff on every save.
    #[test]
    fn the_named_form_is_ordered() {
        let mut map = ActionMap::new();
        for action in ["zoom", "fire", "jump", "aim"] {
            map.bind_key(action, crate::input::code_from_name("w").unwrap());
        }
        let named = map.to_named();
        let keys: Vec<&str> = named.keys().map(String::as_str).collect();
        assert_eq!(keys, ["aim", "fire", "jump", "zoom"]);
    }

    #[test]
    fn adding_a_binding_keeps_the_existing_ones_and_refuses_duplicates() {
        let mut map = ActionMap::new();
        map.bind_key("jump", crate::input::code_from_name("space").unwrap());
        map.add_binding("jump", InputBinding::GamepadButton(GamepadButton::South));
        assert_eq!(
            map.to_named()["jump"],
            vec!["key:space".to_string(), "pad:south".to_string()],
            "a pad binding must not cost the keyboard one"
        );
        map.add_binding("jump", InputBinding::GamepadButton(GamepadButton::South));
        assert_eq!(map.to_named()["jump"].len(), 2, "a duplicate row helps nobody");
    }

    #[test]
    fn removing_the_last_binding_leaves_the_action_in_the_map() {
        let mut map = ActionMap::new();
        map.bind_key("jump", crate::input::code_from_name("space").unwrap());
        assert!(map.remove_binding_named("jump", "key:space"));
        assert!(!map.remove_binding_named("jump", "key:space"), "already gone");
        assert_eq!(
            map.to_named().get("jump"),
            Some(&Vec::<String>::new()),
            "an unbound action must still be listed, or its settings row disappears and it can \
             never be bound again"
        );
    }

    // ── capture ───────────────────────────────────────────────────────────────────────────

    #[test]
    fn capture_answers_nothing_when_nothing_was_pressed() {
        assert_eq!(InputBinding::captured_from(&Input::new()), None);
    }

    /// The click that opened the dialog must not bind itself, and a key the player is leaning on
    /// must not be offered — so capture reads EDGES.
    #[test]
    fn capture_reads_the_press_edge_and_not_the_held_state() {
        let mut input = Input::new();
        input.on_mouse_button_pressed(crate::input::mouse::LEFT);
        assert_eq!(
            InputBinding::captured_from(&input),
            Some(InputBinding::MouseButton(crate::input::mouse::LEFT)),
            "the frame of the press is the frame it is captured"
        );
        input.begin_frame(); // the button is still held, but its edge is gone
        assert_eq!(
            InputBinding::captured_from(&input),
            None,
            "a held button must not keep re-capturing"
        );
    }

    /// A stick captured at 0.83 of travel must not bind an action that fires only past 0.83.
    #[test]
    fn a_captured_axis_stores_a_default_threshold_not_the_reading() {
        let mut input = Input::new();
        let id = crate::input::GamepadId::new(0);
        input.on_gamepad_connected(id, "pad");
        input.on_gamepad_axis(id, GamepadAxis::RightStickX, 0.83);
        let captured = InputBinding::captured_from(&input).expect("a full push is deliberate");
        assert_eq!(
            captured,
            InputBinding::GamepadAxis {
                axis: GamepadAxis::RightStickX,
                direction: AxisDirection::Positive,
                threshold: 0.5,
            }
        );
    }

    #[test]
    fn a_stick_resting_near_centre_is_not_a_capture() {
        let mut input = Input::new();
        let id = crate::input::GamepadId::new(0);
        input.on_gamepad_connected(id, "pad");
        input.on_gamepad_axis(id, GamepadAxis::LeftStickY, 0.3);
        assert_eq!(
            InputBinding::captured_from(&input),
            None,
            "a stick barely off centre is drift, not an answer"
        );
    }

    /// A player pressing a key while a stick rests just past the push threshold must get the key.
    #[test]
    fn a_key_wins_over_a_stick_held_at_the_same_moment() {
        let mut input = Input::new();
        let id = crate::input::GamepadId::new(0);
        input.on_gamepad_connected(id, "pad");
        input.on_gamepad_axis(id, GamepadAxis::LeftStickX, 1.0);
        input.on_key_pressed(crate::input::code_from_name("j").unwrap());
        assert_eq!(
            InputBinding::captured_from(&input),
            Some(InputBinding::Key(crate::input::code_from_name("j").unwrap()))
        );
    }

    /// Capture and naming are two halves of the same UI: whatever is captured has to be
    /// displayable, or a panel shows a blank row for a control the player just pressed.
    #[test]
    fn everything_capture_can_return_has_a_name() {
        let mut input = Input::new();
        let id = crate::input::GamepadId::new(0);
        input.on_gamepad_connected(id, "pad");

        for (_, button) in crate::input::NAMED_GAMEPAD_BUTTONS {
            input.on_gamepad_button_pressed(id, *button);
            let captured = InputBinding::captured_from(&input).expect("a pressed button captures");
            assert!(
                binding_to_name(&captured).is_some(),
                "{captured:?} was captured but cannot be shown"
            );
            input.on_gamepad_button_released(id, *button);
            input.begin_frame();
        }
        for (_, axis) in crate::input::NAMED_GAMEPAD_AXES {
            input.on_gamepad_axis(id, *axis, 1.0);
            let captured = InputBinding::captured_from(&input).expect("a full push captures");
            assert!(binding_to_name(&captured).is_some(), "{captured:?} has no name");
            input.on_gamepad_axis(id, *axis, 0.0);
            input.begin_frame();
        }
    }

    /// A binding the tables cannot name is omitted rather than written as a number — but the
    /// action it belonged to still appears, so the round trip does not lose the fact that it
    /// exists.
    #[test]
    fn an_unnameable_binding_is_omitted_and_its_action_survives() {
        let mut map = ActionMap::new();
        map.bind_key("odd", 60_000); // a code outside NAMED_KEYS
        let named = map.to_named();
        assert_eq!(named["odd"], Vec::<String>::new());
    }
}
