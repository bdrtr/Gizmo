//! Action mapping: `InputBinding` (key/mouse) and the `ActionMap` resource that resolves named
//! actions against `Input`. Extracted verbatim from input.rs (pure move).

use super::*;
use std::collections::HashMap;

/// Input binding kind — a keyboard key, a mouse button, or a gamepad control.
///
/// `#[non_exhaustive]`: this list grew once (gamepads) and will grow again — touch, a wheel's
/// pedals — and each addition would otherwise break every downstream `match`.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub enum InputBinding {
    /// Keyboard key (winit KeyCode as u32)
    Key(u32),
    /// Mouse button (mouse::LEFT, mouse::RIGHT, mouse::MIDDLE)
    MouseButton(u32),
    /// A gamepad button, on whichever pad the [`ActionMap`] is watching.
    GamepadButton(GamepadButton),
    /// A stick or trigger pushed far enough in one direction to count as a button.
    ///
    /// This is how "steer left" and "accelerate" become the same kind of thing as a key, and
    /// it is also how an axis gets *edges*: the press edge is the frame the axis crosses
    /// `threshold`, so an action bound this way fires once per push rather than every frame
    /// the stick is held over.
    GamepadAxis {
        /// Which stick axis or trigger to watch.
        axis: GamepadAxis,
        /// Which end of it counts — `Positive` for right/up/pulled, `Negative` for left/down.
        direction: AxisDirection,
        /// How far along that direction the axis must travel, as a fraction of full
        /// deflection. `0.5` is the usual choice for a stick used as a d-pad; a trigger used
        /// as a button wants something lower, around `0.2`.
        ///
        /// Compared against the raw axis value, deliberately: the threshold *is* the deadzone
        /// for this binding, so [`GamepadDeadzone`](super::GamepadDeadzone) does not move the
        /// point at which the action fires.
        threshold: f32,
    },
}

// `InputBinding` carries an `f32` threshold, so `PartialEq`/`Eq`/`Hash` are written by hand
// over its bit pattern instead of derived. Deriving `PartialEq` and implementing `Eq` on top of
// it would be unsound in the ordinary sense: a `NaN` threshold is not equal to itself, and `Eq`
// promises reflexivity. Comparing bits makes `NaN == NaN` hold, which keeps the type usable as
// a `HashSet` member and a `HashMap` key — which is what it was before gamepads existed, and
// what a rebinding UI wants it to stay.
impl PartialEq for InputBinding {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (InputBinding::Key(a), InputBinding::Key(b)) => a == b,
            (InputBinding::MouseButton(a), InputBinding::MouseButton(b)) => a == b,
            (InputBinding::GamepadButton(a), InputBinding::GamepadButton(b)) => a == b,
            (
                InputBinding::GamepadAxis {
                    axis: a1,
                    direction: d1,
                    threshold: t1,
                },
                InputBinding::GamepadAxis {
                    axis: a2,
                    direction: d2,
                    threshold: t2,
                },
            ) => a1 == a2 && d1 == d2 && t1.to_bits() == t2.to_bits(),
            _ => false,
        }
    }
}

impl Eq for InputBinding {}

impl std::hash::Hash for InputBinding {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            InputBinding::Key(k) => k.hash(state),
            InputBinding::MouseButton(b) => b.hash(state),
            InputBinding::GamepadButton(b) => b.hash(state),
            InputBinding::GamepadAxis {
                axis,
                direction,
                threshold,
            } => {
                axis.hash(state);
                direction.hash(state);
                threshold.to_bits().hash(state);
            }
        }
    }
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
    /// Which pad this map's gamepad bindings read; `None` means any connected pad.
    gamepad: Option<GamepadId>,
}

impl ActionMap {
    /// Creates a map with no bindings at all.
    ///
    /// An unbound action is not an error condition: every `is_action_*` query answers `false`
    /// for a name that was never bound, so a misspelled action name fails silently and
    /// forever rather than panicking. There is no way to ask whether a name is bound.
    ///
    /// Gamepad bindings read *any* connected pad until [`ActionMap::watch_gamepad`] narrows
    /// them to one.
    pub fn new() -> Self {
        Self {
            bindings: HashMap::new(),
            gamepad: None,
        }
    }

    /// Restricts this map's gamepad bindings to one pad, or (with `None`) opens them to any.
    ///
    /// This is how local multiplayer is written: one `ActionMap` per player, each watching that
    /// player's [`GamepadId`], with identical bindings inside. Leaving it `None` — the default
    /// — is right for a single-player game, where "any pad" is what a player who just plugged
    /// in a second controller expects.
    ///
    /// Keyboard and mouse bindings are unaffected; there is only one keyboard.
    pub fn watch_gamepad(&mut self, id: Option<GamepadId>) {
        self.gamepad = id;
    }

    /// The pad this map's gamepad bindings read, or `None` for any.
    pub fn watched_gamepad(&self) -> Option<GamepadId> {
        self.gamepad
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

    /// Binds a gamepad button to a name.
    ///
    /// Bindings accumulate, so calling this after [`ActionMap::bind_key`] for the same name is
    /// what makes an action playable on both keyboard and pad — the action is held if *any* of
    /// its bindings is.
    ///
    /// ```
    /// use gizmo_core::prelude::*;
    /// use gizmo_core::input::{GamepadButton, GamepadId};
    ///
    /// let mut actions = ActionMap::new();
    /// actions.bind_key("Jump", 57); // Space
    /// actions.bind_gamepad_button("Jump", GamepadButton::South);
    ///
    /// let mut input = Input::new();
    /// input.on_gamepad_connected(GamepadId::new(0), "pad");
    /// input.on_gamepad_button_pressed(GamepadId::new(0), GamepadButton::South);
    /// assert!(actions.is_action_just_pressed(&input, "Jump"));
    /// ```
    pub fn bind_gamepad_button(&mut self, action_name: &str, button: GamepadButton) {
        self.bindings
            .entry(action_name.to_string())
            .or_default()
            .push(InputBinding::GamepadButton(button));
    }

    /// Binds one direction of a stick or trigger to a name, treating it as a button.
    ///
    /// `threshold` is the fraction of full travel at which the action starts counting as
    /// pressed — `0.5` for a stick standing in for a d-pad, lower for a trigger. The press edge
    /// is the frame the axis *crosses* it, so an action bound this way fires once per push.
    pub fn bind_gamepad_axis(
        &mut self,
        action_name: &str,
        axis: GamepadAxis,
        direction: AxisDirection,
        threshold: f32,
    ) {
        self.bindings
            .entry(action_name.to_string())
            .or_default()
            .push(InputBinding::GamepadAxis {
                axis,
                direction,
                threshold,
            });
    }

    /// Is the Action being applied right now? (Is it being held down)
    pub fn is_action_pressed(&self, input: &Input, action_name: &str) -> bool {
        self.resolve(input, action_name, Edge::Held)
    }

    /// Was the Action newly triggered on this frame?
    pub fn is_action_just_pressed(&self, input: &Input, action_name: &str) -> bool {
        self.resolve(input, action_name, Edge::Pressed)
    }

    /// Was the Action released on this frame? (For mechanics such as charge-release, toggle)
    pub fn is_action_just_released(&self, input: &Input, action_name: &str) -> bool {
        self.resolve(input, action_name, Edge::Released)
    }

    /// The one place a binding is turned into a yes/no, for all three queries.
    ///
    /// One function rather than three near-identical ones because every binding kind has to
    /// answer all three questions: with a copy per question, a new kind is added correctly in
    /// two of them and forgotten in the third — and the forgotten one is `just_released`, which
    /// no demo exercises and a charge-release mechanic silently loses.
    fn resolve(&self, input: &Input, action_name: &str, edge: Edge) -> bool {
        let Some(bindings) = self.bindings.get(action_name) else {
            return false;
        };
        bindings.iter().any(|binding| match *binding {
            InputBinding::Key(k) => match edge {
                Edge::Held => input.is_key_pressed(k),
                Edge::Pressed => input.is_key_just_pressed(k),
                Edge::Released => input.is_key_just_released(k),
            },
            InputBinding::MouseButton(b) => match edge {
                Edge::Held => input.is_mouse_button_pressed(b),
                Edge::Pressed => input.is_mouse_button_just_pressed(b),
                Edge::Released => input.is_mouse_button_just_released(b),
            },
            InputBinding::GamepadButton(button) => self.any_pad(input, |pad| match edge {
                Edge::Held => pad.is_pressed(button),
                Edge::Pressed => pad.is_just_pressed(button),
                Edge::Released => pad.is_just_released(button),
            }),
            InputBinding::GamepadAxis {
                axis,
                direction,
                threshold,
            } => self.any_pad(input, |pad| match edge {
                Edge::Held => pad.is_axis_beyond(axis, direction, threshold),
                Edge::Pressed => pad.is_axis_just_beyond(axis, direction, threshold),
                Edge::Released => pad.is_axis_just_within(axis, direction, threshold),
            }),
        })
    }

    /// Runs `f` over the pad this map watches, or over every pad when it watches none.
    ///
    /// A pad in its final frame after being unplugged is included deliberately: that frame is
    /// where its release edges live, and an action bound to a button that was held when the
    /// cable came out must still be seen to end.
    fn any_pad(&self, input: &Input, f: impl Fn(&Gamepad) -> bool) -> bool {
        match self.gamepad {
            Some(id) => input.gamepads().get(id).is_some_and(f),
            None => input.gamepads().iter().any(f),
        }
    }
}

/// Which of the three questions [`ActionMap::resolve`] is answering.
#[derive(Clone, Copy)]
enum Edge {
    /// Held down right now.
    Held,
    /// Went down on this frame.
    Pressed,
    /// Came up on this frame.
    Released,
}

impl Default for ActionMap {
    fn default() -> Self {
        Self::new()
    }
}
