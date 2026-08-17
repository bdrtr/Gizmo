//! Gamepad state: which pads are connected, which of their buttons are held, and where their
//! sticks and triggers sit.
//!
//! This module is the *state*, not the driver. `gizmo-core` has no windowing and no device
//! dependency, so nothing here opens a device or knows what evdev, XInput or the browser's
//! Gamepad API are: the platform layer pushes events in through [`Input`]'s `on_gamepad_*`
//! methods exactly as it does for keys and mouse buttons, and this module answers questions
//! about the result. In this workspace that platform layer is `gizmo-app` (gilrs, native only).
//!
//! Three properties are worth knowing before reading the API:
//!
//! - **Buttons are edge-tracked like keys, with the same fast-tap guarantee.** A button pressed
//!   and released inside one frame reads as held *and* just-pressed *and* just-released for that
//!   frame; the removal is deferred to [`Input::begin_frame`]. A one-frame tap on a face button
//!   is exactly what a fighting game must not miss.
//! - **Sticks are read through a radial deadzone, not a per-axis one.** The distinction is not
//!   cosmetic: a per-axis deadzone leaves a *square* dead region, so a stick pushed diagonally
//!   snaps to whichever axis clears its threshold first. [`apply_stick_deadzone`] is the pure
//!   function that implements the radial version, and it also clamps the magnitude to 1 — square
//!   hardware reports √2 at full diagonal, which is a 41 % speed bonus for running diagonally.
//! - **A pad that vanishes releases what it was holding.** Unplugging a controller mid-hold
//!   emits release edges and zeroes the axes before the pad disappears, so a held throttle does
//!   not stay held forever. Focus loss does the same through [`Input::release_all`].
//!
//! ```
//! use gizmo_core::prelude::*;
//! use gizmo_core::input::{GamepadAxis, GamepadButton, GamepadId};
//!
//! let mut input = Input::new();
//! let pad = GamepadId::new(0);
//!
//! // The platform layer reports what it found and what the player did:
//! input.on_gamepad_connected(pad, "Xbox 360 Controller");
//! input.on_gamepad_button_pressed(pad, GamepadButton::South);
//! input.on_gamepad_axis(pad, GamepadAxis::LeftStickX, 0.8);
//!
//! let pad = input.gamepad().expect("one pad is connected");
//! assert!(pad.is_just_pressed(GamepadButton::South)); // jump
//! assert!(pad.left_stick().0 > 0.0); // walk right
//! ```

use super::*;

/// Identifies one gamepad for as long as it stays connected.
///
/// The number is the platform layer's, not an index into anything here: it is whatever the
/// backend uses to tell two pads apart (gilrs's `GamepadId` on native), and this type only
/// carries it. Two consequences follow. Ids are **not** dense — unplugging pad 0 of two leaves
/// pad 1 called pad 1 — so never treat one as a player number or a slot; and an id may be
/// reused for a different physical pad after a disconnect, so a stored id outlives the thing
/// it named. Use [`Gamepads::first`] to mean "the pad in use", not `GamepadId::new(0)`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct GamepadId(u32);

impl GamepadId {
    /// Wraps a platform-supplied id.
    ///
    /// Any value is accepted, including one no backend would produce: this is a newtype, not a
    /// registry, and nothing checks that a pad with this id exists. Feeding an event for an
    /// unknown id is what *creates* the entry (see [`Input::on_gamepad_button_pressed`]).
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// The platform-supplied number back out, for logging or for keying the caller's own map.
    pub const fn raw(self) -> u32 {
        self.0
    }
}

impl std::fmt::Display for GamepadId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "gamepad {}", self.0)
    }
}

/// A button on a gamepad, named by **position** rather than by the letter printed on it.
///
/// `South`/`East`/`North`/`West` are the four face buttons in clockwise order from the bottom.
/// That is deliberate and it is the one naming decision in this enum that will look wrong at
/// first: the bottom face button is `A` on an Xbox pad, `B` on a Nintendo one and `✕` on a
/// PlayStation one, so a `GamepadButton::A` would have to mean "whatever is bottom-left-ish on
/// this brand" and would silently mislead on two thirds of hardware. Position is the only
/// property all three share.
///
/// The trigger names invert gilrs's, and this is the second thing to know. Here `LeftBumper` is
/// the *shoulder* button (LB / L1) and `LeftTrigger` is the analog trigger behind it (LT / L2) —
/// gilrs calls those `LeftTrigger` and `LeftTrigger2`. The translation lives in one `match` in
/// `gizmo-app` and is covered by a test, because a silent swap there gives every player a
/// handbrake where they expected a gear change.
///
/// Analog triggers appear twice on purpose: as a button here (crossing the driver's press
/// threshold) and as [`GamepadAxis::LeftTrigger`] / [`GamepadAxis::RightTrigger`] for the 0..1
/// travel. A racing game wants the axis; a shooter wants the button.
///
/// `#[non_exhaustive]`: pads keep growing buttons (paddles, touchpad clicks, a second Mode),
/// and adding one here must not break a downstream `match`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub enum GamepadButton {
    /// Bottom face button — `A` on Xbox, `B` on Nintendo, cross on PlayStation. Conventionally
    /// jump/confirm.
    South,
    /// Right face button — `B` on Xbox, `A` on Nintendo, circle on PlayStation. Conventionally
    /// back/cancel.
    East,
    /// Top face button — `Y` on Xbox, `X` on Nintendo, triangle on PlayStation.
    North,
    /// Left face button — `X` on Xbox, `Y` on Nintendo, square on PlayStation.
    West,
    /// The extra face button on six-button pads (Sega-style `C`). Absent on the mainstream
    /// three layouts, which is why it is listed after the four that always exist.
    C,
    /// The second extra button on six-button pads (Sega-style `Z`).
    Z,
    /// Left shoulder button — LB / L1. **Not** the analog trigger; see the type docs.
    LeftBumper,
    /// Right shoulder button — RB / R1.
    RightBumper,
    /// Left analog trigger seen as a button — LT / L2, pressed once it passes the driver's
    /// threshold. Read [`GamepadAxis::LeftTrigger`] for how far it actually travelled.
    LeftTrigger,
    /// Right analog trigger seen as a button — RT / R2.
    RightTrigger,
    /// The left-hand menu button: `Back` on Xbox 360, `View` on newer Xbox, `Share` on
    /// PlayStation, `-` on Nintendo.
    Select,
    /// The right-hand menu button: `Start` / `Menu` / `Options` / `+`.
    Start,
    /// The centre logo button — Xbox Guide, PS button, Home. Some drivers reserve it and never
    /// deliver it to applications, so do not make it the only way to reach a menu.
    Mode,
    /// Pressing the left stick down as a button (L3). Distinct from moving it, which is
    /// [`GamepadAxis::LeftStickX`] / [`GamepadAxis::LeftStickY`].
    LeftStick,
    /// Pressing the right stick down as a button (R3).
    RightStick,
    /// D-pad up. Pads that report their d-pad as a hat axis are normalised to these four
    /// buttons by the backend, so a game never has to handle both shapes.
    DPadUp,
    /// D-pad down.
    DPadDown,
    /// D-pad left.
    DPadLeft,
    /// D-pad right.
    DPadRight,
}

impl GamepadButton {
    /// Every button this engine knows, in the order their bit positions are assigned.
    ///
    /// Iterate this to poll or display a whole pad — `#[non_exhaustive]` means downstream code
    /// cannot write the list itself.
    pub const ALL: [GamepadButton; 19] = [
        GamepadButton::South,
        GamepadButton::East,
        GamepadButton::North,
        GamepadButton::West,
        GamepadButton::C,
        GamepadButton::Z,
        GamepadButton::LeftBumper,
        GamepadButton::RightBumper,
        GamepadButton::LeftTrigger,
        GamepadButton::RightTrigger,
        GamepadButton::Select,
        GamepadButton::Start,
        GamepadButton::Mode,
        GamepadButton::LeftStick,
        GamepadButton::RightStick,
        GamepadButton::DPadUp,
        GamepadButton::DPadDown,
        GamepadButton::DPadLeft,
        GamepadButton::DPadRight,
    ];

    /// This button's bit position in the held/pressed/released masks.
    ///
    /// **These numbers are the replay format.** [`Input`] is what a recording stores
    /// ([`PlaybackData`](super::PlaybackData)), the button sets inside it are serialised as
    /// integer masks, so renumbering a button here silently re-labels every button in every
    /// recording ever made. New buttons are appended, never inserted, and
    /// `the_button_bit_positions_are_the_replay_format` pins all nineteen so that a reorder
    /// fails a test instead of a replay.
    const fn index(self) -> u32 {
        match self {
            GamepadButton::South => 0,
            GamepadButton::East => 1,
            GamepadButton::North => 2,
            GamepadButton::West => 3,
            GamepadButton::C => 4,
            GamepadButton::Z => 5,
            GamepadButton::LeftBumper => 6,
            GamepadButton::RightBumper => 7,
            GamepadButton::LeftTrigger => 8,
            GamepadButton::RightTrigger => 9,
            GamepadButton::Select => 10,
            GamepadButton::Start => 11,
            GamepadButton::Mode => 12,
            GamepadButton::LeftStick => 13,
            GamepadButton::RightStick => 14,
            GamepadButton::DPadUp => 15,
            GamepadButton::DPadDown => 16,
            GamepadButton::DPadLeft => 17,
            GamepadButton::DPadRight => 18,
        }
    }

    #[inline]
    const fn bit(self) -> u32 {
        1 << self.index()
    }
}

/// Every gamepad button paired with the name it answers to, lower-case ASCII.
///
/// The counterpart of [`NAMED_KEYS`](super::NAMED_KEYS), and it exists for the same reason: a
/// name is what a config file, a rebinding UI or a script has to work with, and a second
/// transcription of these names somewhere else is a second thing to get wrong. The Lua input
/// API reads this table rather than carrying its own copy — the copy is exactly how the key
/// table came to describe the wrong keyboard for months.
pub const NAMED_GAMEPAD_BUTTONS: &[(&str, GamepadButton)] = &[
    ("south", GamepadButton::South),
    ("east", GamepadButton::East),
    ("north", GamepadButton::North),
    ("west", GamepadButton::West),
    ("c", GamepadButton::C),
    ("z", GamepadButton::Z),
    ("left_bumper", GamepadButton::LeftBumper),
    ("right_bumper", GamepadButton::RightBumper),
    ("left_trigger", GamepadButton::LeftTrigger),
    ("right_trigger", GamepadButton::RightTrigger),
    ("select", GamepadButton::Select),
    ("start", GamepadButton::Start),
    ("mode", GamepadButton::Mode),
    ("left_stick", GamepadButton::LeftStick),
    ("right_stick", GamepadButton::RightStick),
    ("dpad_up", GamepadButton::DPadUp),
    ("dpad_down", GamepadButton::DPadDown),
    ("dpad_left", GamepadButton::DPadLeft),
    ("dpad_right", GamepadButton::DPadRight),
];

/// Every axis paired with its name — see [`NAMED_GAMEPAD_BUTTONS`].
pub const NAMED_GAMEPAD_AXES: &[(&str, GamepadAxis)] = &[
    ("left_stick_x", GamepadAxis::LeftStickX),
    ("left_stick_y", GamepadAxis::LeftStickY),
    ("right_stick_x", GamepadAxis::RightStickX),
    ("right_stick_y", GamepadAxis::RightStickY),
    ("left_trigger", GamepadAxis::LeftTrigger),
    ("right_trigger", GamepadAxis::RightTrigger),
];

/// The button a name refers to, case-insensitively. `None` for a name not in
/// [`NAMED_GAMEPAD_BUTTONS`].
#[must_use]
pub fn gamepad_button_from_name(name: &str) -> Option<GamepadButton> {
    NAMED_GAMEPAD_BUTTONS
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, button)| *button)
}

/// The axis a name refers to, case-insensitively. `None` for a name not in
/// [`NAMED_GAMEPAD_AXES`].
///
/// Note that `"left_trigger"` names a button in [`gamepad_button_from_name`] and an axis here.
/// That is the same control read two ways — the digital press and the analog travel — and the
/// two functions are what disambiguates them.
#[must_use]
pub fn gamepad_axis_from_name(name: &str) -> Option<GamepadAxis> {
    NAMED_GAMEPAD_AXES
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, axis)| *axis)
}

/// A continuously-valued control on a gamepad: the two sticks and the two analog triggers.
///
/// Ranges and signs, which are the whole content of this type:
///
/// - Sticks run `-1.0 ..= 1.0` per axis, **Y positive up** — which is what the rest of this
///   engine means by +Y, so a stick pushed forward and a character walking forward agree
///   without a compensating minus sign anywhere. The kernel disagrees: Linux reports a stick
///   pushed up as its axis *minimum*. Normalising that is the platform layer's job (gilrs
///   already does it, so `gizmo-app` passes the value through), and
///   `a_virtual_pad_arrives_as_the_buttons_and_axes_this_engine_names` is what proves the sign
///   against a real kernel device rather than against a belief about one.
/// - Triggers run `0.0 ..= 1.0`, 0 at rest.
///
/// Read a stick through [`Gamepad::left_stick`] / [`Gamepad::right_stick`] unless you have a
/// reason not to: [`Gamepad::axis`] is the raw value, and raw axes need a *radial* deadzone
/// applied together, which is not something per-axis reads can do.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub enum GamepadAxis {
    /// Left stick, left `-1` to right `+1`.
    LeftStickX,
    /// Left stick, down `-1` to up `+1`.
    LeftStickY,
    /// Right stick, left `-1` to right `+1`.
    RightStickX,
    /// Right stick, down `-1` to up `+1`.
    RightStickY,
    /// Left analog trigger travel, `0.0` released to `1.0` fully pulled.
    LeftTrigger,
    /// Right analog trigger travel, `0.0` released to `1.0` fully pulled.
    RightTrigger,
}

impl GamepadAxis {
    /// Every axis, in storage order — the counterpart of [`GamepadButton::ALL`].
    pub const ALL: [GamepadAxis; 6] = [
        GamepadAxis::LeftStickX,
        GamepadAxis::LeftStickY,
        GamepadAxis::RightStickX,
        GamepadAxis::RightStickY,
        GamepadAxis::LeftTrigger,
        GamepadAxis::RightTrigger,
    ];

    /// Index into the per-pad axis array. Like [`GamepadButton::index`] this is part of the
    /// replay format — append, never insert.
    const fn index(self) -> usize {
        match self {
            GamepadAxis::LeftStickX => 0,
            GamepadAxis::LeftStickY => 1,
            GamepadAxis::RightStickX => 2,
            GamepadAxis::RightStickY => 3,
            GamepadAxis::LeftTrigger => 4,
            GamepadAxis::RightTrigger => 5,
        }
    }

    /// Whether this axis is a trigger (`0..=1`) rather than a stick axis (`-1..=1`).
    ///
    /// The distinction decides which deadzone applies on read, and it is the reason
    /// [`GamepadDeadzone`] has two numbers instead of one.
    #[inline]
    pub const fn is_trigger(self) -> bool {
        matches!(self, GamepadAxis::LeftTrigger | GamepadAxis::RightTrigger)
    }
}

/// Which way an axis was pushed, for the bindings and queries that treat an axis as a button.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum AxisDirection {
    /// Right, up, or trigger pulled — the `+1` end.
    Positive,
    /// Left or down — the `-1` end. Triggers never reach it.
    Negative,
}

/// How much of a stick's or trigger's travel is discarded as noise, per pad.
///
/// Both numbers are fractions of full travel and both are applied *on read*, so the stored axis
/// value stays what the device reported and changing a deadzone re-reads the same input
/// differently — which is what makes a settings slider possible without re-recording anything.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GamepadDeadzone {
    /// Radial deadzone for both sticks. Default `0.15`.
    ///
    /// A worn analog stick can rest 10 % off-centre and a fresh one wanders 2–3 %; 0.15 is the
    /// value that hides both without eating enough travel to be felt. It is *radial* — the
    /// threshold is on the stick's distance from centre, not on each axis separately (see
    /// [`apply_stick_deadzone`]).
    pub stick: f32,
    /// Deadzone at the bottom of each analog trigger's travel. Default `0.05`.
    ///
    /// Smaller than the stick's on purpose: a trigger rests against a physical stop, so it
    /// needs only enough to swallow sensor noise, and every hundredth taken here is throttle
    /// resolution a driving game loses.
    pub trigger: f32,
}

impl GamepadDeadzone {
    /// The stick deadzone used when nothing sets one: `0.15` of full travel.
    pub const DEFAULT_STICK: f32 = 0.15;
    /// The trigger deadzone used when nothing sets one: `0.05` of full travel.
    pub const DEFAULT_TRIGGER: f32 = 0.05;
}

impl Default for GamepadDeadzone {
    fn default() -> Self {
        Self {
            stick: Self::DEFAULT_STICK,
            trigger: Self::DEFAULT_TRIGGER,
        }
    }
}

/// Applies a **radial** deadzone to a stick position and rescales what is left to the full range.
///
/// Two properties, both of which a per-axis deadzone gets wrong:
///
/// - The dead region is a *disc*, so the threshold does not depend on direction. Under a
///   per-axis deadzone a stick pushed to `(0.14, 0.14)` — clearly diagonal, clearly deflected —
///   reads as exactly centred, and one pushed to `(0.16, 0.14)` snaps to pure horizontal.
/// - Output magnitude is rescaled from `deadzone..1` to `0..1`, so it is continuous at the edge
///   of the disc (no jump from 0 to 0.15) and reaches exactly 1 at full deflection.
///
/// The magnitude is also clamped to 1 *before* rescaling, which is what stops a full diagonal
/// from being faster than a full push along an axis: hardware that gates each axis separately
/// reports `(1, 1)`, magnitude 1.41, and an engine that passes that through gives 41 % extra
/// speed on the diagonal.
///
/// `deadzone` is clamped into `0.0..=0.99`, so a nonsensical value cannot divide by zero or
/// invert the scale; `0.0` disables the dead region but keeps the magnitude clamp.
///
/// ```
/// use gizmo_core::input::apply_stick_deadzone;
///
/// // Inside the disc: exactly centred, on both axes.
/// assert_eq!(apply_stick_deadzone(0.1, 0.05, 0.15), (0.0, 0.0));
///
/// // Full diagonal has magnitude 1, not 1.41.
/// let (x, y) = apply_stick_deadzone(1.0, 1.0, 0.15);
/// assert!(((x * x + y * y).sqrt() - 1.0).abs() < 1e-6);
/// ```
pub fn apply_stick_deadzone(x: f32, y: f32, deadzone: f32) -> (f32, f32) {
    let deadzone = deadzone.clamp(0.0, 0.99);
    let magnitude = (x * x + y * y).sqrt();
    if magnitude <= deadzone || magnitude <= f32::EPSILON {
        return (0.0, 0.0);
    }
    let rescaled = ((magnitude.min(1.0) - deadzone) / (1.0 - deadzone)).clamp(0.0, 1.0);
    // Divide by the ORIGINAL magnitude, not the clamped one: this scales the input vector to
    // the rescaled length while keeping its direction exactly.
    let k = rescaled / magnitude;
    (x * k, y * k)
}

/// Applies a one-dimensional deadzone to a trigger and rescales the remainder to `0..=1`.
///
/// The trigger counterpart of [`apply_stick_deadzone`]: there is no direction to preserve, so
/// this is the same rescale on a scalar. Values are clamped into `0..=1` first, which is what
/// makes it safe to hand it a pad that reports its triggers on a `-1..=1` axis — the released
/// half of that range collapses onto 0.
pub fn apply_trigger_deadzone(value: f32, deadzone: f32) -> f32 {
    let deadzone = deadzone.clamp(0.0, 0.99);
    let value = value.clamp(0.0, 1.0);
    if value <= deadzone {
        return 0.0;
    }
    ((value - deadzone) / (1.0 - deadzone)).clamp(0.0, 1.0)
}

/// One connected gamepad: its identity, its buttons' edge state and its axes.
///
/// Obtained from [`Input::gamepad`] (the pad in use) or [`Gamepads::get`] (a specific one).
/// There is no way to build one — the platform layer creates them through [`Input`]'s
/// `on_gamepad_*` methods — and a pad that has been unplugged survives for exactly one frame
/// with [`Gamepad::is_connected`] false so its release edges can be read.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Gamepad {
    id: GamepadId,
    name: String,
    connected: bool,
    /// Buttons held right now, one bit per [`GamepadButton::index`].
    pressed: u32,
    just_pressed: u32,
    just_released: u32,
    axes: [f32; GamepadAxis::ALL.len()],
    /// The axis values as of the previous frame — what makes an axis able to report an edge.
    previous_axes: [f32; GamepadAxis::ALL.len()],
    deadzone: GamepadDeadzone,
}

impl Gamepad {
    fn new(id: GamepadId, name: &str, deadzone: GamepadDeadzone) -> Self {
        Self {
            id,
            name: name.to_string(),
            connected: true,
            pressed: 0,
            just_pressed: 0,
            just_released: 0,
            axes: [0.0; GamepadAxis::ALL.len()],
            previous_axes: [0.0; GamepadAxis::ALL.len()],
            deadzone,
        }
    }

    /// The id the platform layer knows this pad by.
    #[inline]
    pub fn id(&self) -> GamepadId {
        self.id
    }

    /// The device name the driver reported, e.g. `"Xbox 360 Controller"`.
    ///
    /// Free-form vendor text, useful for showing the player which pad is which and for
    /// choosing button glyphs. It can be empty: a pad created by an event that arrived before
    /// its connection notice has no name until one turns up.
    #[inline]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Is the pad still plugged in?
    ///
    /// False for exactly one frame after a disconnect — the frame that gets to see the release
    /// edges this pad emitted on its way out. After the next [`Input::begin_frame`] the pad is
    /// gone from [`Gamepads`] entirely, so this rarely needs asking; a UI that lists pads is
    /// the case that does.
    #[inline]
    pub fn is_connected(&self) -> bool {
        self.connected
    }

    /// Is the button held right now?
    #[inline]
    pub fn is_pressed(&self, button: GamepadButton) -> bool {
        self.pressed & button.bit() != 0
    }

    /// Did this button's press edge land on this frame? (one-shot triggers)
    #[inline]
    pub fn is_just_pressed(&self, button: GamepadButton) -> bool {
        self.just_pressed & button.bit() != 0
    }

    /// Did this button's release edge land on this frame? (charge-release, toggles)
    #[inline]
    pub fn is_just_released(&self, button: GamepadButton) -> bool {
        self.just_released & button.bit() != 0
    }

    /// Every button held right now, for debug overlays and rebinding UIs.
    pub fn pressed_buttons(&self) -> impl Iterator<Item = GamepadButton> + '_ {
        GamepadButton::ALL
            .into_iter()
            .filter(|b| self.is_pressed(*b))
    }

    /// The raw axis value as the platform delivered it — no deadzone, no rescale.
    ///
    /// Use this to implement something the accessors here do not cover, and remember what
    /// "raw" leaves you: a stick at rest reads a few hundredths off zero on most hardware, and
    /// treating each axis separately is what [`apply_stick_deadzone`] exists to avoid.
    #[inline]
    pub fn axis(&self, axis: GamepadAxis) -> f32 {
        self.axes[axis.index()]
    }

    /// What [`Gamepad::axis`] returned on the previous frame.
    ///
    /// An axis has no press events, so this is the only way to see one *cross* a threshold
    /// rather than merely be past it — which is what an action bound to "stick pushed right"
    /// needs in order to fire once instead of every frame. [`ActionMap`] is built on it.
    #[inline]
    pub fn axis_previous(&self, axis: GamepadAxis) -> f32 {
        self.previous_axes[axis.index()]
    }

    /// The left stick as a vector, with this pad's radial deadzone applied and the remaining
    /// travel rescaled to a unit disc. `(-1..=1, -1..=1)`, +Y up.
    #[inline]
    pub fn left_stick(&self) -> (f32, f32) {
        apply_stick_deadzone(
            self.axis(GamepadAxis::LeftStickX),
            self.axis(GamepadAxis::LeftStickY),
            self.deadzone.stick,
        )
    }

    /// The right stick as a vector — see [`Gamepad::left_stick`].
    #[inline]
    pub fn right_stick(&self) -> (f32, f32) {
        apply_stick_deadzone(
            self.axis(GamepadAxis::RightStickX),
            self.axis(GamepadAxis::RightStickY),
            self.deadzone.stick,
        )
    }

    /// Left analog trigger travel with this pad's trigger deadzone applied, `0.0..=1.0`.
    #[inline]
    pub fn left_trigger(&self) -> f32 {
        apply_trigger_deadzone(self.axis(GamepadAxis::LeftTrigger), self.deadzone.trigger)
    }

    /// Right analog trigger travel with this pad's trigger deadzone applied, `0.0..=1.0`.
    #[inline]
    pub fn right_trigger(&self) -> f32 {
        apply_trigger_deadzone(self.axis(GamepadAxis::RightTrigger), self.deadzone.trigger)
    }

    /// The d-pad as a vector, `(-1 | 0 | 1, -1 | 0 | 1)`, +Y up.
    ///
    /// Opposite buttons cancel, so pressing left and right together reads as centred rather
    /// than as one of them winning — the cheap kind of ambiguity that is better resolved here
    /// than in every menu.
    pub fn dpad(&self) -> (f32, f32) {
        let axis = |neg: GamepadButton, pos: GamepadButton| {
            f32::from(self.is_pressed(pos)) - f32::from(self.is_pressed(neg))
        };
        (
            axis(GamepadButton::DPadLeft, GamepadButton::DPadRight),
            axis(GamepadButton::DPadDown, GamepadButton::DPadUp),
        )
    }

    /// This pad's deadzones, as used by the accessors above.
    #[inline]
    pub fn deadzone(&self) -> GamepadDeadzone {
        self.deadzone
    }

    /// Is the axis pushed past `threshold` in `direction`? (an axis used as a button)
    ///
    /// `threshold` is a magnitude and is compared against the raw axis value, not the
    /// deadzoned one — a threshold *is* a deadzone for this purpose, and applying both would
    /// make the effective trigger point depend on a setting the caller did not mention.
    pub fn is_axis_beyond(&self, axis: GamepadAxis, direction: AxisDirection, threshold: f32) -> bool {
        beyond(self.axis(axis), direction, threshold)
    }

    /// Did the axis cross `threshold` on this frame — past it now, not past it last frame?
    pub fn is_axis_just_beyond(
        &self,
        axis: GamepadAxis,
        direction: AxisDirection,
        threshold: f32,
    ) -> bool {
        beyond(self.axis(axis), direction, threshold)
            && !beyond(self.axis_previous(axis), direction, threshold)
    }

    /// Did the axis fall back inside `threshold` on this frame — past it last frame, not now?
    pub fn is_axis_just_within(
        &self,
        axis: GamepadAxis,
        direction: AxisDirection,
        threshold: f32,
    ) -> bool {
        !beyond(self.axis(axis), direction, threshold)
            && beyond(self.axis_previous(axis), direction, threshold)
    }

    fn press(&mut self, button: GamepadButton) {
        // A re-press cancels a pending same-frame release deferral, exactly as `on_key_pressed`
        // does — otherwise `begin_frame` would drop a physically held button.
        self.just_released &= !button.bit();
        if self.pressed & button.bit() == 0 {
            self.pressed |= button.bit();
            self.just_pressed |= button.bit();
        }
    }

    fn release(&mut self, button: GamepadButton) {
        self.just_released |= button.bit();
        if self.just_pressed & button.bit() == 0 {
            // Normal release — clear it now.
            self.pressed &= !button.bit();
        }
        // else: pressed and released inside one frame; `begin_frame` clears it (fast tap).
    }

    fn set_axis(&mut self, axis: GamepadAxis, value: f32) {
        self.axes[axis.index()] = value;
    }

    fn begin_frame(&mut self) {
        self.pressed &= !self.just_released;
        self.just_pressed = 0;
        self.just_released = 0;
        self.previous_axes = self.axes;
    }

    /// Report everything held as released and centre every axis, keeping the pad itself.
    fn release_all(&mut self) {
        self.just_released |= self.pressed;
        self.pressed = 0;
        self.just_pressed = 0;
        self.axes = [0.0; GamepadAxis::ALL.len()];
    }
}

#[inline]
fn beyond(value: f32, direction: AxisDirection, threshold: f32) -> bool {
    match direction {
        AxisDirection::Positive => value >= threshold.abs(),
        AxisDirection::Negative => value <= -threshold.abs(),
    }
}

/// Every gamepad the platform layer has told us about, and the deadzones new ones inherit.
///
/// Reachable as [`Input::gamepads`]; the pads themselves are [`Gamepad`]. Ordered by
/// [`GamepadId`], which makes iteration reproducible — a property this type owes to the replay
/// format, since `Input` is what a recording stores.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Gamepads {
    pads: Vec<Gamepad>,
    #[serde(default)]
    default_deadzone: GamepadDeadzone,
}

impl Gamepads {
    /// No pads, default deadzones.
    pub fn new() -> Self {
        Self {
            pads: Vec::new(),
            default_deadzone: GamepadDeadzone::default(),
        }
    }

    /// The pad a single-player game should read: the connected one with the lowest id.
    ///
    /// `None` when nothing is plugged in, which is the normal case and not an error — a game
    /// that supports both keyboard and pad asks every frame and gets `None` until a controller
    /// appears.
    ///
    /// One deliberate exception: when *no* pad is connected but one is living out its final
    /// frame after being unplugged, that pad is returned. Without it the promise that a
    /// vanishing pad releases what it held would be empty for the code people actually write —
    /// `if let Some(pad) = input.gamepad()` would go `None` on exactly the frame the release
    /// edges exist, and a charge-release mechanic would keep its charge forever. A pad that is
    /// still connected always wins over one that is going away, so a second controller never
    /// blinks out because the first was unplugged.
    pub fn first(&self) -> Option<&Gamepad> {
        self.pads
            .iter()
            .find(|p| p.connected)
            .or_else(|| self.pads.first())
    }

    /// A specific pad by id, connected or in its final frame after a disconnect.
    pub fn get(&self, id: GamepadId) -> Option<&Gamepad> {
        self.pads.iter().find(|p| p.id == id)
    }

    /// Every pad, ordered by id — including, for one frame, a pad that has just been unplugged
    /// (check [`Gamepad::is_connected`] when that matters).
    pub fn iter(&self) -> impl Iterator<Item = &Gamepad> + '_ {
        self.pads.iter()
    }

    /// How many pads are currently connected.
    pub fn connected_count(&self) -> usize {
        self.pads.iter().filter(|p| p.connected).count()
    }

    /// Is anything plugged in? A cheap "show the controller hints" test.
    pub fn any_connected(&self) -> bool {
        self.pads.iter().any(|p| p.connected)
    }

    /// The deadzones a newly connected pad starts with.
    pub fn default_deadzone(&self) -> GamepadDeadzone {
        self.default_deadzone
    }

    /// Sets the deadzones for **every pad**: the ones connected now and the ones that connect
    /// later.
    ///
    /// This is the settings-menu entry point, and it deliberately does not leave the pad in the
    /// player's hand on the old value. Per-pad overrides come from
    /// [`Gamepads::set_deadzone_for`], which this call overwrites.
    pub fn set_deadzone(&mut self, deadzone: GamepadDeadzone) {
        self.default_deadzone = deadzone;
        for pad in &mut self.pads {
            pad.deadzone = deadzone;
        }
    }

    /// Sets the deadzones for one pad, leaving the others and the default alone.
    ///
    /// Returns whether that pad exists. Use it for a worn controller, or for a player who wants
    /// a tighter stick than the rest of the table.
    pub fn set_deadzone_for(&mut self, id: GamepadId, deadzone: GamepadDeadzone) -> bool {
        match self.pads.iter_mut().find(|p| p.id == id) {
            Some(pad) => {
                pad.deadzone = deadzone;
                true
            }
            None => false,
        }
    }

    /// Registers a pad, or brings a disconnected entry back to life under the same id.
    ///
    /// Re-registering a connected pad refreshes its name and leaves its held buttons alone,
    /// which is what makes it safe for the platform layer to re-announce pads after a focus
    /// change without wiping state.
    pub(super) fn connect(&mut self, id: GamepadId, name: &str) {
        match self.pads.iter_mut().find(|p| p.id == id) {
            Some(pad) => {
                pad.connected = true;
                if !name.is_empty() {
                    pad.name = name.to_string();
                }
            }
            None => {
                let pad = Gamepad::new(id, name, self.default_deadzone);
                let at = self.pads.partition_point(|p| p.id < id);
                self.pads.insert(at, pad);
            }
        }
    }

    /// Marks a pad gone, releasing whatever it held.
    ///
    /// The entry stays until the next `begin_frame` so that the release edges and the zeroed
    /// axes are visible to this frame's systems: a pad yanked out mid-corner must not leave the
    /// throttle pinned.
    pub(super) fn disconnect(&mut self, id: GamepadId) {
        if let Some(pad) = self.pads.iter_mut().find(|p| p.id == id) {
            pad.release_all();
            pad.connected = false;
        }
    }

    /// The pad with this id, creating it if the platform layer never announced it.
    ///
    /// Auto-creation is deliberate: a driver that delivers a button before its `Connected`
    /// notice would otherwise have that press silently dropped, and a dropped press is far
    /// harder to notice than a pad with an empty name.
    fn entry(&mut self, id: GamepadId) -> &mut Gamepad {
        let at = self.pads.partition_point(|p| p.id < id);
        if self.pads.get(at).map(|p| p.id) != Some(id) {
            self.pads.insert(at, Gamepad::new(id, "", self.default_deadzone));
        }
        &mut self.pads[at]
    }

    pub(super) fn press(&mut self, id: GamepadId, button: GamepadButton) {
        self.entry(id).press(button);
    }

    pub(super) fn release(&mut self, id: GamepadId, button: GamepadButton) {
        self.entry(id).release(button);
    }

    pub(super) fn set_axis(&mut self, id: GamepadId, axis: GamepadAxis, value: f32) {
        self.entry(id).set_axis(axis, value);
    }

    pub(super) fn begin_frame(&mut self) {
        // Drop pads whose disconnect frame has been seen, then roll the survivors' edges.
        self.pads.retain(|p| p.connected);
        for pad in &mut self.pads {
            pad.begin_frame();
        }
    }

    pub(super) fn release_all(&mut self) {
        for pad in &mut self.pads {
            pad.release_all();
        }
    }
}

impl Default for Gamepads {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAD: GamepadId = GamepadId::new(0);
    const PAD2: GamepadId = GamepadId::new(1);

    fn one_pad() -> Input {
        let mut input = Input::new();
        input.on_gamepad_connected(PAD, "Test Pad");
        input
    }

    /// The bit positions are the on-disk format for every recorded replay, so they are pinned
    /// here rather than left to the order of the enum. A reorder must fail this test; it must
    /// not quietly re-label the buttons in recordings made before it.
    #[test]
    fn the_button_bit_positions_are_the_replay_format() {
        let expected: [(GamepadButton, u32); 19] = [
            (GamepadButton::South, 0),
            (GamepadButton::East, 1),
            (GamepadButton::North, 2),
            (GamepadButton::West, 3),
            (GamepadButton::C, 4),
            (GamepadButton::Z, 5),
            (GamepadButton::LeftBumper, 6),
            (GamepadButton::RightBumper, 7),
            (GamepadButton::LeftTrigger, 8),
            (GamepadButton::RightTrigger, 9),
            (GamepadButton::Select, 10),
            (GamepadButton::Start, 11),
            (GamepadButton::Mode, 12),
            (GamepadButton::LeftStick, 13),
            (GamepadButton::RightStick, 14),
            (GamepadButton::DPadUp, 15),
            (GamepadButton::DPadDown, 16),
            (GamepadButton::DPadLeft, 17),
            (GamepadButton::DPadRight, 18),
        ];
        for (button, index) in expected {
            assert_eq!(button.index(), index, "{button:?} moved bit position");
        }
        assert_eq!(GamepadButton::ALL.len(), expected.len());
        // And the axis order, for the same reason.
        for (i, axis) in GamepadAxis::ALL.into_iter().enumerate() {
            assert_eq!(axis.index(), i, "{axis:?} moved storage slot");
        }
    }

    /// A one-frame tap must be visible as held AND just-pressed AND just-released — the same
    /// guarantee keys have, and the reason a fighting game can read a single-frame input.
    #[test]
    fn a_fast_tap_on_a_face_button_survives_the_frame() {
        let mut input = one_pad();
        input.on_gamepad_button_pressed(PAD, GamepadButton::South);
        input.on_gamepad_button_released(PAD, GamepadButton::South);

        let pad = input.gamepad().unwrap();
        assert!(pad.is_pressed(GamepadButton::South));
        assert!(pad.is_just_pressed(GamepadButton::South));
        assert!(pad.is_just_released(GamepadButton::South));

        input.begin_frame();
        let pad = input.gamepad().unwrap();
        assert!(!pad.is_pressed(GamepadButton::South));
        assert!(!pad.is_just_pressed(GamepadButton::South));
        assert!(!pad.is_just_released(GamepadButton::South));
    }

    /// Released and re-pressed inside one frame: the button is physically down, so it must
    /// stay down. Without cancelling the deferral, `begin_frame` would drop it.
    #[test]
    fn re_pressing_within_the_frame_keeps_the_button_held() {
        let mut input = one_pad();
        input.on_gamepad_button_pressed(PAD, GamepadButton::RightTrigger);
        input.begin_frame();

        input.on_gamepad_button_released(PAD, GamepadButton::RightTrigger);
        input.on_gamepad_button_pressed(PAD, GamepadButton::RightTrigger);
        input.begin_frame();

        let pad = input.gamepad().unwrap();
        assert!(pad.is_pressed(GamepadButton::RightTrigger));
        assert!(!pad.is_just_pressed(GamepadButton::RightTrigger));
    }

    /// Pulling the cable mid-hold must not leave the throttle pinned: the release edges and the
    /// centred axes are readable on the disconnect frame, and the pad is gone after it.
    #[test]
    fn an_unplugged_pad_releases_what_it_held_then_disappears() {
        let mut input = one_pad();
        input.on_gamepad_button_pressed(PAD, GamepadButton::South);
        input.on_gamepad_axis(PAD, GamepadAxis::RightTrigger, 1.0);
        input.begin_frame();

        input.on_gamepad_disconnected(PAD);

        let pad = input.gamepads().get(PAD).expect("pad survives its last frame");
        assert!(!pad.is_connected());
        assert!(!pad.is_pressed(GamepadButton::South));
        assert!(
            pad.is_just_released(GamepadButton::South),
            "a held button must be seen to end"
        );
        assert_eq!(pad.right_trigger(), 0.0, "axes centre on disconnect");
        assert!(
            input
                .gamepad()
                .is_some_and(|p| p.is_just_released(GamepadButton::South)),
            "the release must be visible through `gamepad()` — that is the accessor a game \
             reads, and a charge-release mechanic that cannot see it never lets go"
        );

        input.begin_frame();
        assert!(input.gamepads().get(PAD).is_none(), "and then it is gone");
        assert_eq!(input.gamepads().connected_count(), 0);
    }

    /// The dying pad is a fallback, not a preference: with a second controller still plugged
    /// in, `gamepad()` must keep pointing at the live one rather than blink to the corpse.
    #[test]
    fn a_live_pad_outranks_one_that_just_went_away() {
        let mut input = one_pad();
        input.on_gamepad_connected(PAD2, "Second Pad");
        input.begin_frame();

        input.on_gamepad_disconnected(PAD);
        assert_eq!(
            input.gamepad().map(|p| p.id()),
            Some(PAD2),
            "the connected pad wins"
        );
        assert_eq!(input.gamepads().connected_count(), 1);
    }

    /// A disconnect for a pad nobody announced must not conjure one into existence.
    #[test]
    fn a_disconnect_for_an_unknown_pad_creates_nothing() {
        let mut input = Input::new();
        input.on_gamepad_disconnected(PAD);
        assert_eq!(input.gamepads().iter().count(), 0);
    }

    /// A button event that arrives before the connection notice keeps its press. Dropping it
    /// would be invisible; a pad with an empty name is not.
    #[test]
    fn an_event_for_an_unannounced_pad_creates_it() {
        let mut input = Input::new();
        input.on_gamepad_button_pressed(PAD, GamepadButton::Start);
        let pad = input.gamepad().expect("the event created the pad");
        assert!(pad.is_pressed(GamepadButton::Start));
        assert_eq!(pad.name(), "");

        // …and the notice, when it turns up, fills the name in without wiping the state.
        input.on_gamepad_connected(PAD, "Late Pad");
        let pad = input.gamepad().unwrap();
        assert_eq!(pad.name(), "Late Pad");
        assert!(pad.is_pressed(GamepadButton::Start));
    }

    /// Focus loss stops the pad even though the driver keeps delivering events to an
    /// unfocused window — otherwise an Alt-Tabbed game keeps driving.
    #[test]
    fn focus_loss_centres_the_sticks_and_releases_the_buttons() {
        let mut input = one_pad();
        input.on_gamepad_button_pressed(PAD, GamepadButton::West);
        input.on_gamepad_axis(PAD, GamepadAxis::LeftStickX, -1.0);
        input.begin_frame();

        input.release_all();

        let pad = input.gamepad().expect("the pad is still plugged in");
        assert!(pad.is_connected());
        assert!(!pad.is_pressed(GamepadButton::West));
        assert!(pad.is_just_released(GamepadButton::West));
        assert_eq!(pad.left_stick(), (0.0, 0.0));
    }

    /// The dead region is a disc, not a square. `(0.14, 0.14)` is a real diagonal push — a
    /// per-axis deadzone of 0.15 reads it as dead centre, which is the bug this prevents.
    #[test]
    fn the_stick_deadzone_is_radial_not_per_axis() {
        let dead = apply_stick_deadzone(0.10, 0.05, 0.15);
        assert_eq!(dead, (0.0, 0.0), "inside the disc is centred");

        let (x, y) = apply_stick_deadzone(0.14, 0.14, 0.15);
        assert!(
            x > 0.0 && y > 0.0,
            "a diagonal push past the disc must survive, got ({x}, {y})"
        );

        // And it must not snap to an axis: a per-axis filter would zero the 0.14 here.
        let (_, y) = apply_stick_deadzone(0.9, 0.14, 0.15);
        assert!(y > 0.0, "the smaller component must not be discarded");
    }

    /// Square-gated hardware reports (1, 1) at full diagonal — magnitude 1.41. Passing that
    /// through is 41 % extra speed for running diagonally.
    #[test]
    fn a_full_diagonal_push_is_not_faster_than_a_straight_one() {
        let (x, y) = apply_stick_deadzone(1.0, 1.0, 0.15);
        let diagonal = (x * x + y * y).sqrt();
        let (sx, sy) = apply_stick_deadzone(1.0, 0.0, 0.15);
        let straight = (sx * sx + sy * sy).sqrt();
        assert!((diagonal - 1.0).abs() < 1e-5, "diagonal magnitude {diagonal}");
        assert!((straight - 1.0).abs() < 1e-5, "straight magnitude {straight}");
    }

    /// Output is continuous at the edge of the disc and reaches exactly 1 at full deflection:
    /// no 15 % jump the moment the stick leaves the dead zone.
    #[test]
    fn the_deadzone_rescales_the_remaining_travel_to_the_full_range() {
        let (just_out, _) = apply_stick_deadzone(0.1501, 0.0, 0.15);
        assert!(
            just_out > 0.0 && just_out < 0.01,
            "just past the edge must be near zero, got {just_out}"
        );
        let (full, _) = apply_stick_deadzone(1.0, 0.0, 0.15);
        assert!((full - 1.0).abs() < 1e-6, "full deflection must reach 1, got {full}");

        // Halfway along the live travel is halfway along the output.
        let (half, _) = apply_stick_deadzone(0.15 + 0.85 / 2.0, 0.0, 0.15);
        assert!((half - 0.5).abs() < 1e-5, "midpoint {half}");
    }

    /// Rescaling must not rotate the stick: the output points exactly where the input did.
    #[test]
    fn the_deadzone_preserves_direction() {
        let (x, y) = (0.6_f32, 0.3_f32);
        let (ox, oy) = apply_stick_deadzone(x, y, 0.15);
        assert!(
            (oy / ox - y / x).abs() < 1e-5,
            "direction changed: ({x}, {y}) -> ({ox}, {oy})"
        );
    }

    /// Triggers get their own, smaller deadzone, and a pad that reports its triggers on a
    /// −1..1 axis has its released half collapsed onto zero rather than read as half-pulled.
    #[test]
    fn trigger_travel_maps_to_zero_through_one() {
        assert_eq!(apply_trigger_deadzone(0.0, 0.05), 0.0);
        assert_eq!(apply_trigger_deadzone(-1.0, 0.05), 0.0);
        assert_eq!(apply_trigger_deadzone(0.04, 0.05), 0.0);
        assert!((apply_trigger_deadzone(1.0, 0.05) - 1.0).abs() < 1e-6);
        let mid = apply_trigger_deadzone(0.525, 0.05);
        assert!((mid - 0.5).abs() < 1e-5, "midpoint {mid}");
    }

    /// A deadzone that could not be changed after the fact would need the device to move again
    /// to take effect. It is applied on read, so the same stored input re-reads differently —
    /// which is what a settings slider needs.
    #[test]
    fn changing_the_deadzone_re_reads_the_same_input() {
        let mut input = one_pad();
        input.on_gamepad_axis(PAD, GamepadAxis::LeftStickX, 0.2);
        assert!(input.gamepad().unwrap().left_stick().0 > 0.0);

        input.gamepads_mut().set_deadzone(GamepadDeadzone {
            stick: 0.5,
            trigger: 0.05,
        });
        assert_eq!(
            input.gamepad().unwrap().left_stick().0,
            0.0,
            "a wider deadzone must swallow the same deflection"
        );
        assert!(
            input.gamepad().unwrap().axis(GamepadAxis::LeftStickX) > 0.0,
            "the stored value is untouched — only the reading changed"
        );
    }

    /// Per-pad overrides exist for the worn controller in the drawer, and they do not leak to
    /// the others.
    #[test]
    fn a_per_pad_deadzone_leaves_the_other_pads_alone() {
        let mut input = one_pad();
        input.on_gamepad_connected(PAD2, "Second Pad");
        input.on_gamepad_axis(PAD, GamepadAxis::LeftStickX, 0.2);
        input.on_gamepad_axis(PAD2, GamepadAxis::LeftStickX, 0.2);

        assert!(input.gamepads_mut().set_deadzone_for(
            PAD,
            GamepadDeadzone {
                stick: 0.5,
                trigger: 0.05
            }
        ));
        assert_eq!(input.gamepads().get(PAD).unwrap().left_stick().0, 0.0);
        assert!(input.gamepads().get(PAD2).unwrap().left_stick().0 > 0.0);
        assert!(
            !input.gamepads_mut().set_deadzone_for(
                GamepadId::new(9),
                GamepadDeadzone::default()
            ),
            "an absent pad reports that it was not set"
        );
    }

    /// An axis has no press events, so an action bound to "stick pushed right" needs last
    /// frame's value to fire once instead of every frame. That is what `axis_previous` is.
    #[test]
    fn axes_report_edges_against_the_previous_frame() {
        let mut input = one_pad();
        input.on_gamepad_axis(PAD, GamepadAxis::LeftStickX, 0.9);

        let pad = input.gamepad().unwrap();
        assert!(pad.is_axis_beyond(GamepadAxis::LeftStickX, AxisDirection::Positive, 0.5));
        assert!(pad.is_axis_just_beyond(GamepadAxis::LeftStickX, AxisDirection::Positive, 0.5));
        assert!(!pad.is_axis_beyond(GamepadAxis::LeftStickX, AxisDirection::Negative, 0.5));

        input.begin_frame(); // held, not crossed again
        let pad = input.gamepad().unwrap();
        assert!(pad.is_axis_beyond(GamepadAxis::LeftStickX, AxisDirection::Positive, 0.5));
        assert!(!pad.is_axis_just_beyond(GamepadAxis::LeftStickX, AxisDirection::Positive, 0.5));

        input.on_gamepad_axis(PAD, GamepadAxis::LeftStickX, 0.0);
        let pad = input.gamepad().unwrap();
        assert!(pad.is_axis_just_within(GamepadAxis::LeftStickX, AxisDirection::Positive, 0.5));
    }

    /// Pads are kept in id order so that iteration — and therefore a recording — is
    /// reproducible, and `first` means the lowest connected id rather than the first to arrive.
    #[test]
    fn pads_are_ordered_by_id_whatever_order_they_arrive_in() {
        let mut input = Input::new();
        input.on_gamepad_connected(GamepadId::new(7), "seven");
        input.on_gamepad_connected(GamepadId::new(2), "two");
        input.on_gamepad_connected(GamepadId::new(4), "four");

        let ids: Vec<u32> = input.gamepads().iter().map(|p| p.id().raw()).collect();
        assert_eq!(ids, vec![2, 4, 7]);
        assert_eq!(input.gamepad().unwrap().name(), "two");
        assert_eq!(input.gamepads().connected_count(), 3);
        assert!(input.gamepads().any_connected());
    }

    /// Opposite d-pad directions cancel instead of one winning, so a menu does not jitter when
    /// a worn hat reports both.
    #[test]
    fn opposite_dpad_directions_cancel() {
        let mut input = one_pad();
        input.on_gamepad_button_pressed(PAD, GamepadButton::DPadLeft);
        input.on_gamepad_button_pressed(PAD, GamepadButton::DPadRight);
        input.on_gamepad_button_pressed(PAD, GamepadButton::DPadUp);
        assert_eq!(input.gamepad().unwrap().dpad(), (0.0, 1.0));
    }

    /// The name tables are what a config file and the Lua API address controls by, so every
    /// control must have exactly one name and every name must resolve.
    #[test]
    fn every_control_has_exactly_one_name() {
        for button in GamepadButton::ALL {
            let names: Vec<&str> = NAMED_GAMEPAD_BUTTONS
                .iter()
                .filter(|(_, b)| *b == button)
                .map(|(n, _)| *n)
                .collect();
            assert_eq!(names.len(), 1, "{button:?} has names {names:?}");
            assert_eq!(gamepad_button_from_name(names[0]), Some(button));
        }
        for axis in GamepadAxis::ALL {
            let names: Vec<&str> = NAMED_GAMEPAD_AXES
                .iter()
                .filter(|(_, a)| *a == axis)
                .map(|(n, _)| *n)
                .collect();
            assert_eq!(names.len(), 1, "{axis:?} has names {names:?}");
            assert_eq!(gamepad_axis_from_name(names[0]), Some(axis));
        }
        for (name, _) in NAMED_GAMEPAD_BUTTONS {
            assert_eq!(*name, name.to_ascii_lowercase(), "names are lower-case");
        }
        assert_eq!(gamepad_button_from_name("SOUTH"), Some(GamepadButton::South));
        assert_eq!(gamepad_button_from_name("no such button"), None);
        assert_eq!(gamepad_axis_from_name("south"), None);
    }

    #[test]
    fn pressed_buttons_lists_what_is_held() {
        let mut input = one_pad();
        input.on_gamepad_button_pressed(PAD, GamepadButton::North);
        input.on_gamepad_button_pressed(PAD, GamepadButton::DPadDown);
        let held: Vec<GamepadButton> = input.gamepad().unwrap().pressed_buttons().collect();
        assert_eq!(held, vec![GamepadButton::North, GamepadButton::DPadDown]);
    }

    /// Replays are `Input` written to RON. A recording made before gamepads existed has no
    /// `gamepads` field at all, and must still load — that is what `#[serde(default)]` on the
    /// field buys, and it is worth a test because the failure mode is every old replay in the
    /// project becoming unreadable.
    #[test]
    fn a_replay_recorded_before_gamepads_still_loads() {
        let old = "(keys_pressed:[17],keys_just_pressed:[],keys_just_released:[],\
                   mouse_buttons_pressed:[],mouse_buttons_just_pressed:[],\
                   mouse_buttons_just_released:[],mouse_position:(0.0,0.0),\
                   mouse_delta:(0.0,0.0),mouse_scroll_delta:0.0)";
        let input: Input = ron::from_str(old).expect("a pre-gamepad recording must still load");
        assert!(input.is_key_pressed(17));
        assert!(input.gamepad().is_none());
        assert_eq!(
            input.gamepads().default_deadzone(),
            GamepadDeadzone::default(),
            "a missing deadzone must default, not zero"
        );
    }

    /// And the other direction: a recording made now carries the pad, so a run played with a
    /// controller replays as one.
    #[test]
    fn a_recording_round_trips_the_pad() {
        let mut input = one_pad();
        input.on_gamepad_button_pressed(PAD, GamepadButton::LeftBumper);
        input.on_gamepad_axis(PAD, GamepadAxis::RightStickY, -0.75);

        let text = ron::ser::to_string(&input).expect("serialise");
        let back: Input = ron::from_str(&text).expect("deserialise");

        let pad = back.gamepad().expect("the pad came back");
        assert_eq!(pad.name(), "Test Pad");
        assert!(pad.is_pressed(GamepadButton::LeftBumper));
        assert!(pad.is_just_pressed(GamepadButton::LeftBumper));
        assert_eq!(pad.axis(GamepadAxis::RightStickY), -0.75);
    }
}
