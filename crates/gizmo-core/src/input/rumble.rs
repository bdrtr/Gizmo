//! Rumble: the one thing on a gamepad that travels *outwards*.
//!
//! # Why this is a queue and not a field
//!
//! Everything else in [`Input`] is **state** — what the player is doing right now — and the loop
//! treats it accordingly: it is cloned into the world each frame, rolled by
//! [`begin_frame`](Input::begin_frame), cleared on focus loss, and recorded into replays. Rumble is
//! none of those things. It is a **request**, made once, consumed once, by the platform layer.
//!
//! Modelling it as state would go wrong in three specific ways, and each is the reason for a line
//! below:
//!
//! * **A replay would shake the controller.** A recording is what the player *did*; rumble is what
//!   the game *answered*. Replaying the answer means the pad buzzes at the recorded moments even
//!   though nothing is being played — so the queue is `#[serde(skip)]` and a replay carries none of
//!   it.
//! * **A dropped frame would repeat it.** A field holding "strong: 0.8" is true until something
//!   sets it back, so a frame that forgot to clear it rumbles again. A queue that is drained
//!   cannot: [`Input::take_rumble_requests`] leaves it empty.
//! * **Losing focus would fire it late.** Alt-Tab away mid-explosion and the request would sit in
//!   the queue until the window came back. [`Input::release_all`] drops pending requests for the
//!   same reason it drops held buttons.
//!
//! # Two motors, named for what they are
//!
//! A gamepad has two rumble motors and they are not interchangeable: a small one that buzzes
//! (high frequency, low amplitude) and a large one that thumps (low frequency, high amplitude). A
//! game that sets only [`RumbleRequest::strong`] feels like a distant impact; one that sets only
//! [`RumbleRequest::weak`] feels like texture or an engine idling. They are named `weak`/`strong`
//! after the effect rather than `left`/`right` after the housing, because which motor sits on
//! which side is a fact about one controller's plastic and not about the feeling asked for.

use super::{GamepadId, Input};
use serde::{Deserialize, Serialize};

/// One rumble the game has asked for, waiting to be handed to the platform layer.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RumbleRequest {
    /// Which pad to shake.
    pub gamepad: GamepadId,
    /// The small, high-frequency motor — buzz. Clamped to `0..=1` at construction.
    pub weak: f32,
    /// The large, low-frequency motor — thump. Clamped to `0..=1` at construction.
    pub strong: f32,
    /// How long to run, in seconds. Clamped to at least zero; a zero-length request is kept
    /// rather than dropped, because "stop rumbling" is a legitimate thing to ask for and the
    /// backend is what knows how to express it.
    pub duration_secs: f32,
}

impl RumbleRequest {
    /// A request with every field clamped into range.
    ///
    /// Clamped here rather than at the backend: the fields are `pub`, so a builder-side clamp
    /// would not be an enforcement point, and a magnitude above 1 is not a louder rumble — it is
    /// an integer overflow two layers down, where the value becomes a `u16`.
    #[must_use]
    pub fn new(gamepad: GamepadId, weak: f32, strong: f32, duration_secs: f32) -> Self {
        Self {
            gamepad,
            weak: weak.clamp(0.0, 1.0),
            strong: strong.clamp(0.0, 1.0),
            duration_secs: duration_secs.max(0.0),
        }
    }

    /// Is this request asking for silence? Both motors at zero — which a backend should treat as
    /// "stop", not as "play nothing".
    #[must_use]
    pub fn is_stop(&self) -> bool {
        self.weak == 0.0 && self.strong == 0.0
    }
}

impl Input {
    /// Asks the pad in use to rumble — see [`Gamepads::first`](super::Gamepads::first) for which
    /// one that is.
    ///
    /// Does nothing if there is no pad, which is the common case on a desktop and must not be an
    /// error: a game calls this on impact, and "no controller plugged in" is not a failure of the
    /// impact.
    ///
    /// ```
    /// # use gizmo_core::prelude::*;
    /// # use gizmo_core::input::GamepadId;
    /// let mut input = Input::new();
    /// input.on_gamepad_connected(GamepadId::new(0), "pad");
    ///
    /// input.rumble(0.2, 0.9, 0.25); // a thump with a little buzz on top
    /// let queued = input.take_rumble_requests();
    /// assert_eq!(queued.len(), 1);
    /// assert_eq!(queued[0].strong, 0.9);
    ///
    /// // Draining empties it: a request is consumed once, not held.
    /// assert!(input.take_rumble_requests().is_empty());
    /// ```
    pub fn rumble(&mut self, weak: f32, strong: f32, duration_secs: f32) {
        if let Some(id) = self.gamepads().first().map(super::Gamepad::id) {
            self.rumble_pad(id, weak, strong, duration_secs);
        }
    }

    /// Asks a specific pad to rumble — for a local-multiplayer game, where "the pad in use" is not
    /// a question with one answer.
    pub fn rumble_pad(&mut self, gamepad: GamepadId, weak: f32, strong: f32, duration_secs: f32) {
        self.rumble_queue
            .push(RumbleRequest::new(gamepad, weak, strong, duration_secs));
    }

    /// Asks every connected pad to stop rumbling immediately.
    ///
    /// Queued rather than applied here, like every other request: this type cannot reach a device.
    pub fn stop_rumble(&mut self) {
        let ids: Vec<GamepadId> = self.gamepads().iter().map(super::Gamepad::id).collect();
        for id in ids {
            self.rumble_pad(id, 0.0, 0.0, 0.0);
        }
    }

    /// Takes the queued rumble requests, leaving the queue empty. **The platform layer calls
    /// this**; a game calls [`Input::rumble`].
    ///
    /// Returns them in the order they were made, so a game that asks for a long rumble and then
    /// cancels it gets that sequence rather than the reverse.
    #[must_use]
    pub fn take_rumble_requests(&mut self) -> Vec<RumbleRequest> {
        std::mem::take(&mut self.rumble_queue)
    }

    /// Whether anything is waiting to be drained. Cheap enough for a backend to check before it
    /// touches its device layer at all.
    #[must_use]
    pub fn has_rumble_requests(&self) -> bool {
        !self.rumble_queue.is_empty()
    }

    /// Drops pending rumble requests **and** queues a stop for every connected pad.
    ///
    /// Called from [`Input::release_all`], i.e. on focus loss. Both halves matter: dropping the
    /// pending ones stops a queued explosion from firing when the window comes back, and the stop
    /// is what silences a rumble already running on the device — which the engine cannot otherwise
    /// reach, because the effect lives in the driver and outlives the frame that started it.
    pub(super) fn clear_rumble_on_focus_loss(&mut self) {
        self.rumble_queue.clear();
        self.stop_rumble();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::GamepadId;

    fn with_pad() -> Input {
        let mut input = Input::new();
        input.on_gamepad_connected(GamepadId::new(0), "test pad");
        input
    }

    #[test]
    fn a_request_is_consumed_once() {
        let mut input = with_pad();
        input.rumble(0.5, 0.5, 0.1);
        assert!(input.has_rumble_requests());
        assert_eq!(input.take_rumble_requests().len(), 1);
        assert!(
            !input.has_rumble_requests(),
            "a drained queue must be empty — a request that survives its drain rumbles every \\
             frame until something clears it, which is the failure mode this is not a field for"
        );
    }

    #[test]
    fn magnitudes_are_clamped_where_the_fields_are_public() {
        let mut input = with_pad();
        input.rumble(5.0, -2.0, -1.0);
        let r = input.take_rumble_requests().remove(0);
        assert_eq!((r.weak, r.strong), (1.0, 0.0));
        assert_eq!(r.duration_secs, 0.0, "a negative duration is not a rumble backwards");
    }

    #[test]
    fn asking_with_no_pad_is_not_an_error() {
        let mut input = Input::new();
        input.rumble(1.0, 1.0, 1.0);
        assert!(
            !input.has_rumble_requests(),
            "with nothing plugged in there is nothing to shake, and a game must not have to check"
        );
    }

    #[test]
    fn the_two_motors_are_independent() {
        let mut input = with_pad();
        input.rumble(1.0, 0.0, 0.1); // buzz only
        input.rumble(0.0, 1.0, 0.1); // thump only
        let q = input.take_rumble_requests();
        assert_eq!((q[0].weak, q[0].strong), (1.0, 0.0));
        assert_eq!((q[1].weak, q[1].strong), (0.0, 1.0));
        assert!(!q[0].is_stop() && !q[1].is_stop());
    }

    #[test]
    fn both_motors_at_zero_is_a_stop_not_a_no_op() {
        let mut input = with_pad();
        input.stop_rumble();
        let q = input.take_rumble_requests();
        assert_eq!(q.len(), 1, "one stop per connected pad");
        assert!(q[0].is_stop());
    }

    /// Focus loss must not leave a rumble running on the device, and must not fire a queued one
    /// when the window comes back.
    #[test]
    fn losing_focus_drops_pending_requests_and_asks_for_silence() {
        let mut input = with_pad();
        input.rumble(1.0, 1.0, 10.0); // a long one, mid-explosion
        input.release_all();
        let q = input.take_rumble_requests();
        assert_eq!(
            q.len(),
            1,
            "the pending explosion must be gone and a stop left in its place, got {q:?}"
        );
        assert!(q[0].is_stop());
    }

    /// A replay records what the player did. Rumble is what the game answered, and replaying an
    /// answer would shake a controller nobody is holding.
    #[test]
    fn a_replay_carries_no_rumble() {
        let mut input = with_pad();
        input.rumble(1.0, 1.0, 1.0);
        let json = serde_json::to_string(&input).expect("Input serialises");
        assert!(
            !json.contains("rumble"),
            "the rumble queue reached a recording: {json}"
        );
        let restored: Input = serde_json::from_str(&json).expect("and deserialises");
        assert!(!restored.has_rumble_requests());
    }

    #[test]
    fn a_second_pad_can_be_addressed_by_id() {
        let mut input = with_pad();
        input.on_gamepad_connected(GamepadId::new(1), "second pad");
        input.rumble_pad(GamepadId::new(1), 0.0, 0.7, 0.2);
        let q = input.take_rumble_requests();
        assert_eq!(q[0].gamepad, GamepadId::new(1));
        assert_eq!(q[0].strong, 0.7);
    }
}
