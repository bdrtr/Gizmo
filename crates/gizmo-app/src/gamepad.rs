//! The gamepad backend: physical controllers → [`gizmo_core::input::Input`].
//!
//! `gizmo-core` holds gamepad *state* and knows nothing about devices; this module is the other
//! half — it opens the platform's gamepad subsystem through [`gilrs`], drains its event queue
//! once per frame and pushes the result into `Input` exactly as the winit handlers push keys and
//! mouse buttons. Nothing gilrs-shaped leaves this file: the type below takes and returns only
//! `gizmo-core` types, so the dependency stays private to `gizmo-app`.
//!
//! The windowed runtime owns one of these and drives it automatically, so a game normally never
//! names this type — it just reads `input.gamepad()`. It is public for the case that does need
//! it: an application driving its own event loop.
//!
//! **Native only.** Browsers expose gamepads through `navigator.getGamepads()`, which is a
//! different API on a different clock (polled, not evented) and is not wired yet; on `wasm32`
//! this module does not exist and `input.gamepad()` simply stays `None`.
//!
//! ### What the translation has to get right
//!
//! - **Names.** gilrs's `Button::LeftTrigger` is the *bumper* (LB/L1) and `LeftTrigger2` is the
//!   analog trigger (LT/L2). This engine calls those [`GamepadButton::LeftBumper`] and
//!   [`GamepadButton::LeftTrigger`], because "trigger" meaning the shoulder button is a trap.
//!   The one `match` that crosses the two namings is `translate_button`, and
//!   `the_bumper_and_the_trigger_do_not_swap` is the test that holds it.
//! - **Triggers are both.** An analog trigger arrives as a button (crossing gilrs's press
//!   threshold) *and* as a value; both are forwarded, the first as
//!   [`GamepadButton::LeftTrigger`], the second as [`GamepadAxis::LeftTrigger`].
//! - **Hat d-pads.** Many pads report their d-pad as an axis pair. gilrs's default filters
//!   normalise that into the four d-pad buttons before we ever see it, which is why
//!   [`GamepadAxis`] has no d-pad entry.

// ── The browser ──────────────────────────────────────────────────────────────────────────────
//
// This module builds and runs on wasm32 as of 2026-08-18, through gilrs's own wasm backend: it
// reads the **Web Gamepad API** with web-sys and turns the browser's polled snapshot into the
// same event stream the native backends produce. Nothing above this file knows the difference,
// which is the point — `pump`, `resync` and the `KnownPad` mirror are unchanged.
//
// Two browser facts that have no native counterpart, and that a game will meet before this code
// does:
//
// * **A page must be a secure context** (https, or localhost) or the API is absent. gilrs warns
//   rather than failing, so the symptom is a pad that never connects.
// * **The gamepad list stays empty until the player presses a button.** Browsers hide connected
//   pads from a page that has not been interacted with, as a fingerprinting defence. So the
//   "control held at launch" gap documented above is not merely still open on the web — it is
//   the *normal* first state there, and a game that waits for `input.gamepad()` before showing
//   "press a button to start" has it the wrong way round.
//
// Not verified against a real browser: nothing here can drive one. What is verified is that the
// arm compiles and lints for wasm32 with the feature on, which is its own CI invocation (see the
// `wasm` job) — because a target that is built and never linted is exactly how the *last* wasm
// arm in this workspace rotted.

use gilrs::{Axis, Button, Event, EventType, Gilrs};
use gizmo_core::input::{GamepadAxis, GamepadButton, GamepadId, Input};

/// Reads physical gamepads and feeds them into an [`Input`].
///
/// Construction never fails: if the platform has no gamepad subsystem — a container without
/// `/dev/input` access, a locked-down session — the backend logs once and becomes inert, and
/// every game that also supports the keyboard keeps working. Ask [`GamepadBackend::is_available`]
/// if you want to say so in a UI.
pub struct GamepadBackend {
    /// `None` when the platform refused to start gilrs. Inert, not fatal.
    gilrs: Option<Gilrs>,
    /// What each pad is doing, as far as anything can know — see [`KnownPad`].
    known: Vec<KnownPad>,
    /// The rumble effect currently uploaded per pad, held so it can be stopped and replaced.
    ///
    /// gilrs's `Effect` owns its slot in the driver: dropping it removes the effect, and a driver
    /// has a small fixed number of slots (16 on a typical Linux pad). Building a new effect per
    /// request without dropping the old one therefore stops working after a dozen explosions —
    /// with an error the game has no way to act on. One effect per pad, replaced in place.
    effects: Vec<(GamepadId, gilrs::ff::Effect)>,
}

/// The backend's own mirror of a pad's state, kept because gilrs does not keep one for us.
///
/// **Measured, 2026-08-17:** immediately after `Gilrs::new()`, a connected pad holding its right
/// trigger at maximum reports `button_data(RightTrigger2) == None` and `value(RightZ) == 0.0`;
/// the value appears only once an event moves it. gilrs builds its state *from the event stream*
/// and reads nothing from the device when it opens it. So "ask gilrs what the pad is doing" is
/// not a thing that can be done, and a resync that tried it restored nothing — which the first
/// live run of `car_demo` showed as a car that would not move when the throttle had been held
/// down since before launch.
///
/// What *can* be known is everything since the pad connected, because the backend saw every
/// event. That is what this holds, and what [`GamepadBackend::resync`] replays.
struct KnownPad {
    id: GamepadId,
    name: String,
    held: Vec<GamepadButton>,
    /// Every axis with its last value, in [`GamepadAxis::ALL`] order.
    axes: [(GamepadAxis, f32); 6],
}

impl KnownPad {
    fn new(id: GamepadId, name: &str) -> Self {
        Self {
            id,
            name: name.to_string(),
            held: Vec::new(),
            axes: GamepadAxis::ALL.map(|axis| (axis, 0.0)),
        }
    }
}

impl GamepadBackend {
    /// Opens the platform's gamepad subsystem and registers the pads already plugged in.
    ///
    /// Their *state* is not registered, because it cannot be: a pad that is already plugged in
    /// generates no connection event and gilrs reads nothing from the device at open (see
    /// [`KnownPad`]), so a control held while the game launches reads as at rest until it moves.
    /// Registering the pads themselves is still worth doing — a game that shows controller hints
    /// when one is connected shows them from the first frame.
    pub fn new(input: &mut Input) -> Self {
        let gilrs = match Gilrs::new() {
            Ok(gilrs) => Some(gilrs),
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "[gamepad] gamepad support unavailable; keyboard and mouse still work"
                );
                None
            }
        };
        let mut backend = Self {
            gilrs,
            known: Vec::new(),
            effects: Vec::new(),
        };
        if let Some(gilrs) = &backend.gilrs {
            for (id, gamepad) in gilrs.gamepads() {
                let pad = GamepadId::new(usize::from(id) as u32);
                backend.known.push(KnownPad::new(pad, gamepad.name()));
                input.on_gamepad_connected(pad, gamepad.name());
            }
            tracing::info!(count = backend.known.len(), "[gamepad] backend ready");
        }
        backend
    }

    /// Is a gamepad subsystem actually running? False means no pad will ever appear.
    pub fn is_available(&self) -> bool {
        self.gilrs.is_some()
    }

    /// Hands the frame's rumble requests to the driver, and empties the queue.
    ///
    /// Call it **after** the frame's systems have run — that is when the requests exist — and
    /// before the next `pump`. A game asks with [`Input::rumble`]; this is the other end.
    ///
    /// Silently does nothing when gilrs did not start, when the pad is gone, or when the device
    /// has no motors: `is_ff_supported()` is false for a great many pads (every wired Xbox 360
    /// clone without the rumble pack, most flight sticks, every virtual pad that did not declare
    /// `FF_RUMBLE`), and a game must not have to ask before it can call `rumble` on impact.
    ///
    /// **Duration is expressed as the effect's own scheduling, not by a timer here.** A rumble
    /// that the engine had to stop on a later frame would keep buzzing through a stall, a
    /// breakpoint or a dropped frame — the driver has the clock that does not stop.
    pub fn apply_rumble(&mut self, input: &mut Input) {
        if !input.has_rumble_requests() {
            return;
        }
        let requests = input.take_rumble_requests();
        let Some(gilrs) = &mut self.gilrs else {
            return;
        };

        for request in requests {
            let Some(pad_id) = gilrs
                .gamepads()
                .find(|(id, _)| GamepadId::new(usize::from(*id) as u32) == request.gamepad)
                .map(|(id, _)| id)
            else {
                continue;
            };
            if !gilrs.gamepad(pad_id).is_ff_supported() {
                continue;
            }

            // A stop is a stop, not an effect of zero magnitude: dropping the effect frees the
            // driver slot, where a zero-magnitude effect would keep holding it.
            if request.is_stop() {
                self.effects.retain(|(id, _)| *id != request.gamepad);
                continue;
            }

            // f32 0..=1 → the u16 the driver speaks. Rounded rather than truncated so a request
            // of 1.0 reaches full scale instead of one short of it.
            let scale = |v: f32| (v.clamp(0.0, 1.0) * f32::from(u16::MAX)).round() as u16;
            let ms = (request.duration_secs * 1000.0).round().clamp(0.0, 60_000.0) as u32;
            let play_for = gilrs::ff::Ticks::from_ms(ms);

            let mut builder = gilrs::ff::EffectBuilder::new();
            // Both motors, always — a request that asked for only one gets the other at zero,
            // which is what makes "buzz" and "thump" separable rather than a single intensity.
            builder
                .add_effect(gilrs::ff::BaseEffect {
                    kind: gilrs::ff::BaseEffectType::Weak {
                        magnitude: scale(request.weak),
                    },
                    scheduling: gilrs::ff::Replay {
                        play_for,
                        ..Default::default()
                    },
                    ..Default::default()
                })
                .add_effect(gilrs::ff::BaseEffect {
                    kind: gilrs::ff::BaseEffectType::Strong {
                        magnitude: scale(request.strong),
                    },
                    scheduling: gilrs::ff::Replay {
                        play_for,
                        ..Default::default()
                    },
                    ..Default::default()
                })
                .add_gamepad(&gilrs.gamepad(pad_id));

            match builder.finish(gilrs) {
                Ok(effect) => {
                    if let Err(e) = effect.play() {
                        tracing::debug!(pad = %request.gamepad, error = ?e, "[gamepad] rumble refused");
                        continue;
                    }
                    // Replace rather than append: see `effects`.
                    self.effects.retain(|(id, _)| *id != request.gamepad);
                    self.effects.push((request.gamepad, effect));
                }
                Err(e) => {
                    tracing::debug!(pad = %request.gamepad, error = ?e, "[gamepad] rumble effect rejected");
                }
            }
        }
    }

    /// Drains every pending device event into `input`. Call once per frame, before the frame's
    /// systems run and before any replay overwrites `Input`.
    pub fn pump(&mut self, input: &mut Input) {
        let Some(gilrs) = &mut self.gilrs else {
            return;
        };
        while let Some(Event { id, event, .. }) = gilrs.next_event() {
            let pad = GamepadId::new(usize::from(id) as u32);
            match event {
                EventType::Connected => {
                    let name = gilrs.gamepad(id).name().to_string();
                    tracing::info!(pad = %pad, name = %name, "[gamepad] connected");
                    input.on_gamepad_connected(pad, &name);
                    // `entry` rather than a push: an event can arrive before its connection
                    // notice, and that entry is holding real state under an empty name.
                    Self::entry(&mut self.known, pad).name = name;
                }
                EventType::Disconnected => {
                    tracing::info!(pad = %pad, "[gamepad] disconnected");
                    input.on_gamepad_disconnected(pad);
                    self.known.retain(|k| k.id != pad);
                }
                EventType::ButtonPressed(button, _) => {
                    if let Some(ours) = translate_button(button) {
                        input.on_gamepad_button_pressed(pad, ours);
                        let known = Self::entry(&mut self.known, pad);
                        if !known.held.contains(&ours) {
                            known.held.push(ours);
                        }
                    }
                    sync_trigger_travel(gilrs, id, button, pad, &mut self.known, input);
                }
                EventType::ButtonReleased(button, _) => {
                    if let Some(ours) = translate_button(button) {
                        input.on_gamepad_button_released(pad, ours);
                        Self::entry(&mut self.known, pad).held.retain(|b| *b != ours);
                    }
                    sync_trigger_travel(gilrs, id, button, pad, &mut self.known, input);
                }
                // The analog half of a trigger. Digital buttons also emit this (0.0 / 1.0) and
                // are deliberately ignored here — their edges arrive as Pressed/Released.
                EventType::ButtonChanged(button, value, _) => {
                    if let Some(axis) = trigger_axis_of(button) {
                        input.on_gamepad_axis(pad, axis, value);
                        Self::remember_axis(&mut self.known, pad, axis, value);
                    }
                }
                EventType::AxisChanged(axis, value, _) => {
                    if let Some(axis) = translate_axis(axis) {
                        input.on_gamepad_axis(pad, axis, value);
                        Self::remember_axis(&mut self.known, pad, axis, value);
                    }
                }
                // `Dropped` is a filtered-out event, `ButtonRepeated` needs a filter we do not
                // install, and force-feedback completion is not wired.
                _ => {}
            }
        }
    }

    /// The mirror entry for a pad, created if an event arrived before its connection notice.
    fn entry(known: &mut Vec<KnownPad>, pad: GamepadId) -> &mut KnownPad {
        if let Some(i) = known.iter().position(|k| k.id == pad) {
            return &mut known[i];
        }
        known.push(KnownPad::new(pad, ""));
        known.last_mut().expect("just pushed")
    }

    fn remember_axis(known: &mut Vec<KnownPad>, pad: GamepadId, axis: GamepadAxis, value: f32) {
        let entry = Self::entry(known, pad);
        if let Some(slot) = entry.axes.iter_mut().find(|(a, _)| *a == axis) {
            slot.1 = value;
        }
    }

    /// Pushes everything the backend knows back into `input`.
    ///
    /// This is the other half of [`Input::release_all`]. A driver that reads the device directly
    /// keeps delivering events while the window is unfocused, so the windowed loop drops gamepad
    /// state on focus loss; the buttons a player never let go of therefore produce no new events,
    /// and without this they would stay silent until released and pressed again.
    ///
    /// It replays the backend's own mirror rather than asking gilrs, because gilrs has nothing
    /// to answer with — see [`KnownPad`]. Buttons are pushed as presses only: a release for
    /// something that was never pressed would be a spurious `just_released` edge. Axes are all
    /// written, since "at rest" is a value like any other.
    pub fn resync(&mut self, input: &mut Input) {
        for known in &self.known {
            input.on_gamepad_connected(known.id, &known.name);
            for button in &known.held {
                input.on_gamepad_button_pressed(known.id, *button);
            }
            for (axis, value) in known.axes {
                input.on_gamepad_axis(known.id, axis, value);
            }
        }
    }
}

/// Pushes an analog trigger's current travel whenever its *button* edge fires.
///
/// Necessary because gilrs does not emit `ButtonChanged` at the crossing: a trigger pulled from
/// rest to the floor produces `ButtonPressed(RightTrigger2)` and nothing else, so a backend that
/// only listened to `ButtonChanged` would report a pressed trigger whose travel was still 0.
/// Measured against a virtual Xbox 360 pad — `ButtonChanged` arrived for the *left* trigger at
/// half travel and for both on release, but never for the press that crossed the threshold, and
/// a driving game reading `right_trigger()` therefore got no throttle at all.
///
/// The value comes from gilrs's own state rather than being assumed to be 1.0, because
/// "pressed" only means past the threshold — a trigger held at 80 % is pressed and is not
/// floored.
fn sync_trigger_travel(
    gilrs: &Gilrs,
    id: gilrs::GamepadId,
    button: Button,
    pad: GamepadId,
    known: &mut Vec<KnownPad>,
    input: &mut Input,
) {
    let Some(axis) = trigger_axis_of(button) else {
        return;
    };
    if let Some(data) = gilrs.gamepad(id).button_data(button) {
        input.on_gamepad_axis(pad, axis, data.value());
        GamepadBackend::remember_axis(known, pad, axis, data.value());
    }
}

/// The button naming crosswalk. Written as a table rather than as a `match` so that the resync
/// path and the event path cannot disagree about it.
const BUTTONS: [(GamepadButton, Button); 19] = [
    (GamepadButton::South, Button::South),
    (GamepadButton::East, Button::East),
    (GamepadButton::North, Button::North),
    (GamepadButton::West, Button::West),
    (GamepadButton::C, Button::C),
    (GamepadButton::Z, Button::Z),
    // The two that invert: gilrs's `LeftTrigger` is the bumper, its `LeftTrigger2` the trigger.
    (GamepadButton::LeftBumper, Button::LeftTrigger),
    (GamepadButton::RightBumper, Button::RightTrigger),
    (GamepadButton::LeftTrigger, Button::LeftTrigger2),
    (GamepadButton::RightTrigger, Button::RightTrigger2),
    (GamepadButton::Select, Button::Select),
    (GamepadButton::Start, Button::Start),
    (GamepadButton::Mode, Button::Mode),
    (GamepadButton::LeftStick, Button::LeftThumb),
    (GamepadButton::RightStick, Button::RightThumb),
    (GamepadButton::DPadUp, Button::DPadUp),
    (GamepadButton::DPadDown, Button::DPadDown),
    (GamepadButton::DPadLeft, Button::DPadLeft),
    (GamepadButton::DPadRight, Button::DPadRight),
];

/// The axis crosswalk. gilrs's `DPadX`/`DPadY` are absent on purpose: its default filters have
/// already turned a hat d-pad into the four buttons by the time we see events.
const AXES: [(GamepadAxis, Axis); 6] = [
    (GamepadAxis::LeftStickX, Axis::LeftStickX),
    (GamepadAxis::LeftStickY, Axis::LeftStickY),
    (GamepadAxis::RightStickX, Axis::RightStickX),
    (GamepadAxis::RightStickY, Axis::RightStickY),
    (GamepadAxis::LeftTrigger, Axis::LeftZ),
    (GamepadAxis::RightTrigger, Axis::RightZ),
];

fn translate_button(button: Button) -> Option<GamepadButton> {
    BUTTONS
        .iter()
        .find(|(_, theirs)| *theirs == button)
        .map(|(ours, _)| *ours)
}

fn translate_axis(axis: Axis) -> Option<GamepadAxis> {
    AXES.iter()
        .find(|(_, theirs)| *theirs == axis)
        .map(|(ours, _)| *ours)
}

/// The trigger axis a gilrs button carries travel for, if it is one of the two analog triggers.
fn trigger_axis_of(button: Button) -> Option<GamepadAxis> {
    match button {
        Button::LeftTrigger2 => Some(GamepadAxis::LeftTrigger),
        Button::RightTrigger2 => Some(GamepadAxis::RightTrigger),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The one mistake in this file that hardware would not reveal until a player complained:
    /// gilrs's `LeftTrigger` is the shoulder button, and mapping it to our `LeftTrigger` would
    /// give every game a handbrake where it expected a gear change. Both directions are pinned.
    #[test]
    fn the_bumper_and_the_trigger_do_not_swap() {
        assert_eq!(
            translate_button(Button::LeftTrigger),
            Some(GamepadButton::LeftBumper)
        );
        assert_eq!(
            translate_button(Button::LeftTrigger2),
            Some(GamepadButton::LeftTrigger)
        );
        assert_eq!(
            translate_button(Button::RightTrigger),
            Some(GamepadButton::RightBumper)
        );
        assert_eq!(
            translate_button(Button::RightTrigger2),
            Some(GamepadButton::RightTrigger)
        );
    }

    /// Every button this engine can express must have a source event, or it is a control no
    /// player can ever press — and the table must not name one twice, or two physical buttons
    /// collapse into one.
    #[test]
    fn the_crosswalk_covers_every_button_exactly_once() {
        for ours in GamepadButton::ALL {
            assert!(
                BUTTONS.iter().any(|(b, _)| *b == ours),
                "{ours:?} has no gilrs source"
            );
        }
        for (i, (ours, theirs)) in BUTTONS.iter().enumerate() {
            for (other_ours, other_theirs) in &BUTTONS[i + 1..] {
                assert_ne!(ours, other_ours, "{ours:?} appears twice");
                assert_ne!(theirs, other_theirs, "{theirs:?} appears twice");
            }
        }
        assert_eq!(BUTTONS.len(), GamepadButton::ALL.len());
    }

    /// The same for axes, plus the thing the table cannot say on its own: `Button::Unknown` and
    /// `Axis::Unknown` are what gilrs sends for a control it has no mapping for, and translating
    /// one into a real button would fire an action nobody pressed.
    #[test]
    fn unmapped_controls_are_dropped_rather_than_guessed() {
        assert_eq!(translate_button(Button::Unknown), None);
        assert_eq!(translate_axis(Axis::Unknown), None);
        // A hat d-pad has already been filtered into buttons; if one ever reached us it must
        // not be mistaken for a stick.
        assert_eq!(translate_axis(Axis::DPadX), None);
        assert_eq!(translate_axis(Axis::DPadY), None);
        for ours in GamepadAxis::ALL {
            assert!(
                AXES.iter().any(|(a, _)| *a == ours),
                "{ours:?} has no gilrs source"
            );
        }
    }

    /// Only the analog triggers carry travel; a face button's `ButtonChanged` must not be
    /// written to an axis.
    #[test]
    fn only_the_analog_triggers_are_treated_as_axes() {
        assert_eq!(
            trigger_axis_of(Button::LeftTrigger2),
            Some(GamepadAxis::LeftTrigger)
        );
        assert_eq!(
            trigger_axis_of(Button::RightTrigger2),
            Some(GamepadAxis::RightTrigger)
        );
        assert_eq!(trigger_axis_of(Button::South), None);
        assert_eq!(trigger_axis_of(Button::LeftTrigger), None, "that is a bumper");
    }
}
