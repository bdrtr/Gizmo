//! The mixer: the one place a playing sound's volume and speed are decided.
//!
//! Before this module every modifier wrote **into** the player: the underwater muffle multiplied
//! each sink's volume by `0.4` and undid it with `2.5`, and the spatial system overwrote the same
//! field with `attenuation × source.volume` every frame. Two writers, one field, no memory of what
//! anyone had asked for — measured on real hardware (2026-08-18):
//!
//! | step | volume | speed |
//! |---|---|---|
//! | 3D sound playing | 1.00 | 1.00 |
//! | `set_underwater(true)` | 0.40 | 0.85 |
//! | **one frame of `audio_spatial_system`** | **1.00** | **1.00** |
//! | `set_underwater(false)` | **2.50** | 1.00 |
//!
//! So the muffle lasted until the next frame — i.e. never, for any 3D sound — and surfacing then
//! multiplied by 2.5 anyway, which a `Player` obeys: 250 % of the volume the game asked for. A 2D
//! sound whose volume was set *while* submerged came back at 0.55 instead of 0.22.
//!
//! The fix is not a guard, it is the shape: a sound's volume is **composed**, never accumulated.
//! Each live sink keeps what the game asked for (its [`Route`]), and the number written to rodio is
//!
//! ```text
//! volume = route.volume × bus.gain × master.gain × environment      (0 if either is muted)
//! speed  = route.pitch                                              (sanitised)
//! cutoff = environment                                              (Hz, 0 = no filter)
//! reverb = the manager's space                                      (on or off)
//! ```
//!
//! The environment — the underwater level and cutoff, and the reverb — reaches a sound through its
//! bus: a bus set [dry](Mixer::set_bus_dry) is outside the world the listener is in, so a menu's
//! music stays as it was mixed while the camera is under water or in a cave.
//!
//! recomputed from those inputs every time any of them moves. Nothing accumulates, so nothing
//! drifts, and the order the modifiers arrive in stops mattering.
//!
//! The `speed` line lost its environment term on the same day it gained the `cutoff` line: the
//! underwater effect used to be a 0.85× playback slow-down standing in for a filter rodio could
//! not run live, and a slow-down is a pitch shift — it detunes music and drops a looped engine a
//! tone. `crate::filter::Muffle` puts a real biquad inside each source instead, so the cutoff
//! travels through one atomic rather than per sink.
//!
//! The buses are the half a game ships: a music slider and an SFX slider are one gain each, applied
//! to every sound routed there — including sounds that started before the player touched the slider.

use std::collections::{BTreeMap, HashMap};

/// A bus's gain, mute flag and whether the environment reaches it. Gains are clamped on write (see
/// [`Mixer::set_bus_gain`]).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Bus {
    gain: f32,
    muted: bool,
    /// Outside the listener's environment: no underwater muffle, no reverb.
    dry: bool,
}

impl Default for Bus {
    fn default() -> Self {
        Self { gain: 1.0, muted: false, dry: false }
    }
}

impl Bus {
    /// The factor this bus contributes: its gain, or nothing at all when muted.
    fn factor(self) -> f32 {
        if self.muted {
            0.0
        } else {
            self.gain
        }
    }
}

/// What one live sink was asked to play at, before any bus or environment is applied.
///
/// `volume` and `pitch` are the game's intent — the distance attenuation the spatial system
/// computes, the value behind a `set_volume` call, the Doppler-shifted pitch. Keeping them is what
/// makes the modifiers composable: the same intent can be re-priced whenever a bus moves.
#[derive(Debug, Clone, PartialEq)]
struct Route {
    bus: String,
    volume: f32,
    pitch: f32,
    /// The fade's own factor, apart from `volume`: the spatial system rewrites `volume` every
    /// frame from the distance, and a fade riding on it would be undone by the next frame.
    fade: f32,
}

/// A sink's fade factor on its way somewhere: linear from `from` to `to` over `secs`, and the
/// sink stopped at the end when `stop` says so.
#[derive(Debug, Clone, Copy)]
struct Fade {
    from: f32,
    to: f32,
    secs: f32,
    t: f32,
    stop: bool,
}

/// What [`Mixer::advance_fades`] moved: the sinks whose volume changed, and the ones whose
/// fade-out ended and are to be stopped.
#[derive(Debug, Default)]
pub(crate) struct FadeStep {
    pub(crate) changed: Vec<u64>,
    pub(crate) stop: Vec<u64>,
}

/// Why [`Mixer::set_send`] refused a send.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SendError {
    /// A bus cannot send to itself.
    ToItself(String),
    /// The send would close a loop: `to` already reaches `from`, through the sends named in order.
    Loop {
        /// The bus the send was to leave.
        from: String,
        /// The bus it was to reach.
        to: String,
        /// The way `to` already gets back to `from`, both ends included.
        through: Vec<String>,
    },
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ToItself(bus) => write!(f, "bus '{bus}' cannot send to itself"),
            Self::Loop { from, to, through } => {
                write!(f, "a send from '{from}' to '{to}' would close a loop: {}", through.join(" -> "))
            }
        }
    }
}

impl std::error::Error for SendError {}

/// Volume multiplier while the listener is underwater. Water attenuates, so the level drops —
/// but the *character* of underwater sound is the missing top end, and that is
/// [`UNDERWATER_CUTOFF_HZ`]'s job, not this one's.
const UNDERWATER_VOLUME: f32 = 0.4;
/// Low-pass corner while underwater, in Hz.
///
/// This used to be a **0.85× playback speed** instead, because rodio's `Player` offers no live
/// filter — but slowing playback is a pitch shift: a looped engine drops a tone, a music track
/// detunes, and nothing about water does that. `crate::filter::Muffle` puts a real biquad inside
/// each source, so the muffle is now what it always claimed to be. 700 Hz is inside the band where
/// speech and engine harmonics live, so what is removed is audibly the "air" and not the sound.
const UNDERWATER_CUTOFF_HZ: u32 = 700;

/// Clamp a gain to something a mixer can multiply through.
///
/// Negative is not "quieter", it is a phase flip; `NaN` propagates through every product and
/// reaches the device as `NaN` samples, which is a click and then silence you cannot debug from
/// the outside. Both collapse to `0.0` — the only wrong value with no audible consequence. Above
/// `1.0` is left alone: amplification is a legitimate thing to ask a mixer for.
fn sanitize_gain(gain: f32) -> f32 {
    if gain.is_finite() {
        gain.max(0.0)
    } else {
        0.0
    }
}

/// Buses, a master, the environment modifier, and one `Route` per live sink.
///
/// A game holds this through [`AudioManager::mixer_mut`](crate::AudioManager::mixer_mut) and turns
/// gains on it; [`AudioManager`](crate::AudioManager) is what writes the resulting numbers to rodio,
/// and it does so for every live sink whenever anything here moves.
///
/// ```
/// # use gizmo_audio::Mixer;
/// let mut mixer = Mixer::new();
/// mixer.set_bus_gain(Mixer::MUSIC, 0.3);   // the player's music slider
/// assert_eq!(mixer.bus_gain(Mixer::MUSIC), 0.3);
/// assert_eq!(mixer.bus_gain(Mixer::SFX), 1.0); // untouched buses stay at unity
/// ```
#[derive(Debug, Clone)]
pub struct Mixer {
    master: Bus,
    buses: HashMap<String, Bus>,
    underwater: bool,
    routes: HashMap<u64, Route>,
    fades: HashMap<u64, Fade>,
    /// The sends between buses, `(from, to) → level`, in name order (`set_send`).
    sends: BTreeMap<(String, String), f32>,
    /// Set by every mutator, cleared by [`Mixer::take_dirty`]. It exists because a game can reach
    /// the mixer directly (`mixer_mut()`), so the manager cannot know a gain moved unless it is
    /// told — and re-writing every sink's two atomics on every frame to avoid asking is a cost
    /// paid forever for a slider that moves twice a session.
    dirty: bool,
}

impl Default for Mixer {
    fn default() -> Self {
        Self::new()
    }
}

impl Mixer {
    /// The bus a music track belongs on.
    pub const MUSIC: &'static str = "music";
    /// The bus sound effects belong on — and the one a sound gets when nobody chooses.
    pub const SFX: &'static str = "sfx";
    /// The bus interface sounds belong on.
    pub const UI: &'static str = "ui";
    /// The bus dialogue belongs on.
    pub const VOICE: &'static str = "voice";
    /// Where a sound is routed when nothing says otherwise.
    pub const DEFAULT_BUS: &'static str = Self::SFX;

    /// A mixer at unity: master `1.0`, no bus turned down, not underwater.
    ///
    /// Buses are created on demand, so the four named constants are a vocabulary rather than a
    /// fixed set — a game with a `"footsteps"` bus needs no engine change. The cost is that a
    /// misspelled bus name is a *new* bus at unity rather than an error, i.e. a sound the slider
    /// does not reach; the constants exist so the common four cannot be misspelled.
    pub fn new() -> Self {
        Self {
            master: Bus::default(),
            buses: HashMap::new(),
            underwater: false,
            routes: HashMap::new(),
            fades: HashMap::new(),
            sends: BTreeMap::new(),
            dirty: false,
        }
    }

    // ── master ───────────────────────────────────────────────────────────────

    /// Sets the gain every sound passes through. Clamped: see [`Mixer::set_bus_gain`].
    pub fn set_master_gain(&mut self, gain: f32) {
        self.master.gain = sanitize_gain(gain);
        self.dirty = true;
    }

    /// The master gain, as clamped on write.
    #[inline]
    pub fn master_gain(&self) -> f32 {
        self.master.gain
    }

    /// Silences (or restores) everything **without losing any gain** — the gains keep their values
    /// and come back when the mute is lifted, which is what a mute button has to do.
    pub fn set_master_muted(&mut self, muted: bool) {
        self.master.muted = muted;
        self.dirty = true;
    }

    /// Whether the master is muted.
    #[inline]
    pub fn is_master_muted(&self) -> bool {
        self.master.muted
    }

    // ── buses ────────────────────────────────────────────────────────────────

    /// Sets a bus's gain, creating the bus if this is the first mention of it.
    ///
    /// The value is clamped by `sanitize_gain`: negative and non-finite gains become `0.0`,
    /// anything above `1.0` is kept.
    pub fn set_bus_gain(&mut self, bus: &str, gain: f32) {
        self.bus_entry(bus).gain = sanitize_gain(gain);
        self.dirty = true;
    }

    /// A bus's gain — `1.0` for a bus nobody has touched, which is also what an unknown bus reads
    /// as, because an unmentioned bus and a bus at unity price a sound identically.
    pub fn bus_gain(&self, bus: &str) -> f32 {
        self.buses.get(bus).copied().unwrap_or_default().gain
    }

    /// Mutes (or unmutes) one bus, keeping its gain. See [`Mixer::set_master_muted`].
    pub fn set_bus_muted(&mut self, bus: &str, muted: bool) {
        self.bus_entry(bus).muted = muted;
        self.dirty = true;
    }

    /// Whether this bus is muted.
    pub fn is_bus_muted(&self, bus: &str) -> bool {
        self.buses.get(bus).copied().unwrap_or_default().muted
    }

    /// Every bus this mixer knows about, in no particular order — the list a settings panel draws
    /// a slider for. A bus appears once something mentions it, by name or by routing a sound to it.
    pub fn buses(&self) -> impl Iterator<Item = &str> {
        self.buses.keys().map(String::as_str)
    }

    /// Takes a bus out of the listener's environment, or puts it back: the sounds on a **dry** bus
    /// get no underwater muffle (neither its level nor its low-pass) and no reverb, and keep their
    /// gain, mute and fades. For what is not in the world the camera is in — a menu's music, the
    /// interface's clicks, a narrator. Every bus is in it until it is set dry, and it applies to the
    /// sounds already playing on it.
    pub fn set_bus_dry(&mut self, bus: &str, dry: bool) {
        self.bus_entry(bus).dry = dry;
        self.dirty = true;
    }

    /// Whether a bus is outside the listener's environment. See [`Mixer::set_bus_dry`].
    pub fn is_bus_dry(&self, bus: &str) -> bool {
        self.buses.get(bus).copied().unwrap_or_default().dry
    }

    // ── sends ────────────────────────────────────────────────────────────────

    /// Sends what bus `from` puts out — after its gain, its sounds' own and its effect — at `level`
    /// into bus `to`, where it is summed with `to`'s sounds, goes through `to`'s effect and comes
    /// out under `to`'s gain: a reverb bus fed by the others, say. `from` is still heard as it was;
    /// the send is a copy. Level `0` removes the send. Clamped as a gain is ([`set_bus_gain`](Self::set_bus_gain)).
    /// Heard from the next [`AudioManager::update`](crate::AudioManager::update), a frame of audio
    /// late.
    ///
    /// ```
    /// # use gizmo_audio::Mixer;
    /// let mut mixer = Mixer::new();
    /// mixer.set_send(Mixer::SFX, "hall", 0.4).unwrap();
    /// mixer.set_send(Mixer::VOICE, "hall", 0.2).unwrap();
    /// assert!(mixer.set_send("hall", Mixer::SFX, 0.1).is_err(), "it would come back to itself");
    /// ```
    ///
    /// # Errors
    ///
    /// [`SendError::ToItself`] for `from == to`, and [`SendError::Loop`] for a send whose bus already
    /// reaches `from`: its sound would come back a frame later, and again, for ever. Nothing changes
    /// when it is refused.
    pub fn set_send(&mut self, from: &str, to: &str, level: f32) -> Result<(), SendError> {
        if from == to {
            return Err(SendError::ToItself(from.to_string()));
        }
        let level = sanitize_gain(level);
        let key = (from.to_string(), to.to_string());
        if level == 0.0 {
            if self.sends.remove(&key).is_some() {
                self.dirty = true;
            }
            return Ok(());
        }
        if let Some(mut through) = self.way(to, from) {
            through.insert(0, from.to_string());
            return Err(SendError::Loop { from: from.to_string(), to: to.to_string(), through });
        }
        self.bus_entry(from);
        self.bus_entry(to);
        self.sends.insert(key, level);
        self.dirty = true;
        Ok(())
    }

    /// The level bus `from` sends to bus `to` at, `0` for no send.
    pub fn send_level(&self, from: &str, to: &str) -> f32 {
        self.sends.get(&(from.to_string(), to.to_string())).copied().unwrap_or(0.0)
    }

    /// Every send, `(from, to, level)`, in name order.
    pub fn sends(&self) -> impl Iterator<Item = (&str, &str, f32)> {
        self.sends.iter().map(|((from, to), level)| (from.as_str(), to.as_str(), *level))
    }

    /// The buses the sends lead along from `from` to `to`, both ends included, if any do.
    fn way(&self, from: &str, to: &str) -> Option<Vec<String>> {
        let mut seen = std::collections::BTreeSet::new();
        let mut stack = vec![vec![from.to_string()]];
        while let Some(path) = stack.pop() {
            let last = path.last()?.clone();
            if last == to {
                return Some(path);
            }
            if !seen.insert(last.clone()) {
                continue;
            }
            for ((a, b), _) in self.sends.range((last.clone(), String::new())..) {
                if *a != last {
                    break;
                }
                let mut next = path.clone();
                next.push(b.clone());
                stack.push(next);
            }
        }
        None
    }

    /// The factor a bus puts on what is sent into it: its gain, or nothing when muted. The sounds
    /// sent carry their own bus's gain and the master already.
    pub(crate) fn return_gain(&self, bus: &str) -> f32 {
        self.buses.get(bus).copied().unwrap_or_default().factor()
    }

    fn bus_entry(&mut self, bus: &str) -> &mut Bus {
        self.buses.entry(bus.to_string()).or_default()
    }

    // ── environment ──────────────────────────────────────────────────────────

    /// Turns the underwater muffle on or off, for every bus but the [dry](Mixer::set_bus_dry) ones.
    pub fn set_underwater(&mut self, on: bool) {
        if on != self.underwater {
            self.underwater = on;
            self.dirty = true;
        }
    }

    /// Whether the underwater muffle is on.
    #[inline]
    pub fn is_underwater(&self) -> bool {
        self.underwater
    }

    // ── routes (one per live sink) ───────────────────────────────────────────

    /// Registers a sink on `bus` at unity, replacing any route under that id. The manager does it
    /// before the sound reaches the device, so its first sample is priced.
    pub(crate) fn route(&mut self, id: u64, bus: &str) {
        self.routes
            .insert(id, Route { bus: bus.to_string(), volume: 1.0, pitch: 1.0, fade: 1.0 });
        self.buses.entry(bus.to_string()).or_default();
    }

    /// The bus a live sink is on, if the mixer knows it.
    pub fn sink_bus(&self, id: u64) -> Option<&str> {
        self.routes.get(&id).map(|r| r.bus.as_str())
    }

    /// Records what the game asked this sink to play at — the distance attenuation, the slider,
    /// the scene's `AudioSource::volume`. This is intent, not the number that reaches the device.
    pub(crate) fn set_sink_volume(&mut self, id: u64, volume: f32) {
        if let Some(route) = self.routes.get_mut(&id) {
            route.volume = sanitize_gain(volume);
        }
    }

    /// Records the pitch the game asked for, Doppler included. See [`Mixer::set_sink_volume`].
    pub(crate) fn set_sink_pitch(&mut self, id: u64, pitch: f32) {
        if let Some(route) = self.routes.get_mut(&id) {
            route.pitch = pitch;
        }
    }

    /// Forgets every route whose sink is gone. Called from the manager's sink garbage collector —
    /// without it a long session leaks one `Route` per sound ever played.
    pub(crate) fn retain_routes(&mut self, keep: impl Fn(u64) -> bool) {
        self.routes.retain(|id, _| keep(*id));
        self.fades.retain(|id, _| keep(*id));
    }

    // ── fades ────────────────────────────────────────────────────────────────

    /// Starts moving a live sink's fade factor — a multiplier on top of its volume, bus and
    /// master — to `to` over `secs` seconds, from wherever it is now (a fade already running is
    /// replaced from its current point, so reversing one does not jump). `stop` stops the sink
    /// when it arrives. `false` for a sink the mixer has never seen.
    ///
    /// `secs <= 0` (or NaN) arrives on the next [`advance_fades`](Self::advance_fades), whatever
    /// its `dt`.
    pub(crate) fn start_fade(&mut self, id: u64, to: f32, secs: f32, stop: bool) -> bool {
        let Some(route) = self.routes.get(&id) else {
            return false;
        };
        let secs = if secs.is_finite() { secs.max(0.0) } else { 0.0 };
        self.fades.insert(id, Fade { from: route.fade, to: sanitize_gain(to), secs, t: 0.0, stop });
        true
    }

    /// Sets a sink's fade factor at once, cancelling any fade — `0.0` before fading a sound in.
    pub(crate) fn set_fade(&mut self, id: u64, factor: f32) {
        self.fades.remove(&id);
        if let Some(route) = self.routes.get_mut(&id) {
            route.fade = sanitize_gain(factor);
        }
    }

    /// A sink's fade factor: `1.0` unless a fade has moved it.
    pub fn fade_of(&self, id: u64) -> f32 {
        self.routes.get(&id).map_or(1.0, |r| r.fade)
    }

    /// Whether a sink is fading.
    pub fn is_fading(&self, id: u64) -> bool {
        self.fades.contains_key(&id)
    }

    /// Advances every fade by `dt` seconds, writing each sink's fade factor, and reports what
    /// moved. A finished fade is forgotten; a finished fade-out is reported for stopping.
    pub(crate) fn advance_fades(&mut self, dt: f32) -> FadeStep {
        let mut step = FadeStep::default();
        if self.fades.is_empty() {
            return step;
        }
        let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
        let mut done = Vec::new();
        for (&id, fade) in &mut self.fades {
            fade.t += dt;
            let k = if fade.secs <= 0.0 { 1.0 } else { (fade.t / fade.secs).min(1.0) };
            if let Some(route) = self.routes.get_mut(&id) {
                route.fade = fade.from + (fade.to - fade.from) * k;
                step.changed.push(id);
            }
            if k >= 1.0 {
                done.push(id);
                if fade.stop {
                    step.stop.push(id);
                }
            }
        }
        for id in done {
            self.fades.remove(&id);
        }
        step
    }

    /// How many live sinks the mixer is pricing. (A leak check, and a debug-HUD number.)
    #[inline]
    pub fn routed_sinks(&self) -> usize {
        self.routes.len()
    }

    // ── the composition ──────────────────────────────────────────────────────

    /// The bus a route is on, or the default one's settings for a route on a bus never mentioned.
    fn bus_of(&self, id: u64) -> Option<Bus> {
        let route = self.routes.get(&id)?;
        Some(self.buses.get(&route.bus).copied().unwrap_or_default())
    }

    /// Whether the listener is underwater as far as this sink is concerned: under water, and not on
    /// a dry bus.
    fn submerged(&self, bus: Bus) -> bool {
        self.underwater && !bus.dry
    }

    /// The volume to write to this sink: intent × fade × bus × master × environment.
    ///
    /// `0.0` for an id with no route — a sound the mixer cannot price is one it will not let
    /// through at an arbitrary volume.
    pub fn volume_for(&self, id: u64) -> f32 {
        let (Some(route), Some(bus)) = (self.routes.get(&id), self.bus_of(id)) else {
            return 0.0;
        };
        let environment = if self.submerged(bus) { UNDERWATER_VOLUME } else { 1.0 };
        route.volume * route.fade * bus.factor() * self.master.factor() * environment
    }

    /// The playback speed to write to this sink: the pitch the game asked for, sanitised.
    ///
    /// No environment term any more — the underwater effect is a filter, not a pitch shift (see
    /// `UNDERWATER_CUTOFF_HZ`). The clamp stays because `pitch` is scene-authored and reaches
    /// `0.0` from a serialised `AudioSource`, which makes rodio's sample-rate converter compute a
    /// rate of zero and assert on the audio callback thread.
    pub fn speed_for(&self, id: u64) -> f32 {
        let Some(route) = self.routes.get(&id) else {
            return 1.0;
        };
        crate::sanitize_playback_speed(route.pitch)
    }

    /// The low-pass corner this sink's source should filter at, `0` for none.
    ///
    /// The manager stores it into the sink's own control, which the filter inside its source reads
    /// on the audio thread: per sink, since a dry bus is not muffled. `0` for an id with no route.
    pub fn cutoff_hz_for(&self, id: u64) -> u32 {
        match self.bus_of(id) {
            Some(bus) if self.submerged(bus) => UNDERWATER_CUTOFF_HZ,
            _ => 0,
        }
    }

    /// Whether the manager's reverb reaches this sink: it does unless the sink is on a dry bus.
    /// `false` for an id with no route.
    pub fn reverb_reaches(&self, id: u64) -> bool {
        self.bus_of(id).is_some_and(|bus| !bus.dry)
    }

    /// Whether anything has moved since this was last asked, clearing the flag.
    ///
    /// The manager calls it once per frame: a mixer nobody touched costs one bool test, and a
    /// mixer somebody did costs two atomic stores per live sink.
    pub(crate) fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The composition, end to end: a bus scales the sounds on it and nothing else.
    #[test]
    fn a_bus_gain_scales_the_sounds_on_it_and_leaves_the_others_alone() {
        let mut mixer = Mixer::new();
        mixer.route(1, Mixer::MUSIC);
        mixer.route(2, Mixer::SFX);

        mixer.set_bus_gain(Mixer::MUSIC, 0.25);
        assert_eq!(mixer.volume_for(1), 0.25, "the music sink follows its bus");
        assert_eq!(mixer.volume_for(2), 1.0, "the sfx sink is on a bus nobody moved");

        // And a sound that starts *after* the slider moved is priced the same as one that was
        // already playing — the reason the gain lives on the bus and not on the sink.
        mixer.route(3, Mixer::MUSIC);
        assert_eq!(mixer.volume_for(3), 0.25);
    }

    /// Master mute must be recoverable: the gains are still there underneath.
    #[test]
    fn muting_silences_without_losing_the_gains() {
        let mut mixer = Mixer::new();
        mixer.route(1, Mixer::SFX);
        mixer.set_master_gain(0.8);
        mixer.set_bus_gain(Mixer::SFX, 0.5);
        assert_eq!(mixer.volume_for(1), 0.4);

        mixer.set_master_muted(true);
        assert_eq!(mixer.volume_for(1), 0.0, "a mute is silence, not a small number");
        mixer.set_master_muted(false);
        assert_eq!(mixer.volume_for(1), 0.4, "and it comes back exactly as it was");

        mixer.set_bus_muted(Mixer::SFX, true);
        assert_eq!(mixer.volume_for(1), 0.0, "a bus mute silences its own sounds");
        mixer.set_bus_muted(Mixer::SFX, false);
        assert_eq!(mixer.volume_for(1), 0.4);
    }

    /// REGRESSION (measured 2026-08-18, real device). `audio_spatial_system` writes a 3D sink's
    /// distance attenuation **every frame**. When that write was the sink's volume itself, it
    /// erased the underwater muffle one frame after `set_underwater(true)` — so the muffle was
    /// unreachable for every 3D sound in the engine. It is intent now, and intent composes.
    #[test]
    fn the_underwater_muffle_survives_the_spatial_update_that_used_to_erase_it() {
        let mut mixer = Mixer::new();
        mixer.route(1, Mixer::SFX);
        mixer.set_underwater(true);
        assert_eq!(mixer.volume_for(1), UNDERWATER_VOLUME);

        for _ in 0..120 {
            // two seconds of frames, each one re-stating the attenuation
            mixer.set_sink_volume(1, 1.0);
            mixer.set_sink_pitch(1, 1.0);
        }
        assert_eq!(
            mixer.volume_for(1),
            UNDERWATER_VOLUME,
            "the muffle must outlive the frames that restate the attenuation"
        );
        assert_eq!(
            mixer.cutoff_hz_for(1),
            UNDERWATER_CUTOFF_HZ,
            "and so must the filter corner, which is the other half of it"
        );
        assert_eq!(mixer.speed_for(1), 1.0, "the muffle is a filter, not a pitch shift");
    }

    /// REGRESSION (measured 2026-08-18, real device). Surfacing multiplied every sink by
    /// `1/0.4 = 2.5`, so a volume set while submerged came back 250 % too loud: 0.22 → 0.55, and a
    /// 3D sink whose muffle had already been erased surfaced at 2.5. Nothing accumulates now.
    #[test]
    fn surfacing_returns_exactly_the_volume_the_game_asked_for() {
        let mut mixer = Mixer::new();
        mixer.route(1, Mixer::SFX);

        mixer.set_underwater(true);
        mixer.set_sink_volume(1, 0.22); // the slider, moved while submerged
        assert_eq!(mixer.volume_for(1), 0.22 * UNDERWATER_VOLUME);

        mixer.set_underwater(false);
        assert_eq!(mixer.volume_for(1), 0.22, "surfacing is not an amplifier");

        // And the order the two arrive in cannot matter, which is the property that was broken.
        let mut other = Mixer::new();
        other.route(1, Mixer::SFX);
        other.set_sink_volume(1, 0.22);
        other.set_underwater(true);
        other.set_underwater(false);
        assert_eq!(other.volume_for(1), mixer.volume_for(1));
    }

    /// A gain that reaches rodio as `NaN` is silence you cannot debug; a negative one is a phase
    /// flip nobody asked for. Both are clamped, and amplification above 1.0 is left alone.
    #[test]
    fn gains_are_clamped_where_they_are_written() {
        let mut mixer = Mixer::new();
        mixer.route(1, Mixer::SFX);

        mixer.set_bus_gain(Mixer::SFX, -1.0);
        assert_eq!(mixer.bus_gain(Mixer::SFX), 0.0);
        mixer.set_bus_gain(Mixer::SFX, f32::NAN);
        assert_eq!(mixer.bus_gain(Mixer::SFX), 0.0);
        mixer.set_master_gain(f32::INFINITY);
        assert_eq!(mixer.master_gain(), 0.0);

        mixer.set_master_gain(2.0);
        mixer.set_bus_gain(Mixer::SFX, 1.0);
        assert_eq!(mixer.volume_for(1), 2.0, "amplification is a legitimate request");

        mixer.set_sink_volume(1, -3.0);
        assert_eq!(mixer.volume_for(1), 0.0, "intent is clamped on the same rule");
    }

    /// A scene may serialise `AudioSource.pitch = 0.0`, and rodio's sample-rate converter asserts
    /// on a rate of zero — on the audio callback thread, taking playback with it.
    #[test]
    fn the_composed_speed_stays_above_rodios_floor() {
        let mut mixer = Mixer::new();
        mixer.route(1, Mixer::SFX);
        mixer.set_sink_pitch(1, 0.0); // e.g. a scene-authored `AudioSource.pitch`
        mixer.set_underwater(true);
        assert!(mixer.speed_for(1) >= 0.01, "got {}", mixer.speed_for(1));
        assert_eq!(mixer.cutoff_hz_for(1), UNDERWATER_CUTOFF_HZ);
    }

    /// An id the mixer does not know is not a sound it will let through at full volume.
    #[test]
    fn an_unrouted_sink_is_silent_and_unpitched() {
        let mixer = Mixer::new();
        assert_eq!(mixer.volume_for(404), 0.0);
        assert_eq!(mixer.speed_for(404), 1.0);
        assert_eq!(mixer.cutoff_hz_for(404), 0);
        assert!(!mixer.reverb_reaches(404));
    }

    /// A dry bus is outside the environment: under water its sounds keep their level and get no
    /// low-pass, and the reverb does not reach them; its gain and mute still apply, and the other
    /// buses are muffled as before. Until 2026-11-26 the muffle applied to every bus, a menu's
    /// music too, with no way out.
    #[test]
    fn a_dry_bus_is_outside_the_environment() {
        let mut mixer = Mixer::new();
        mixer.route(1, Mixer::MUSIC);
        mixer.route(2, Mixer::SFX);
        mixer.set_bus_gain(Mixer::MUSIC, 0.5);
        mixer.set_underwater(true);
        mixer.take_dirty();

        mixer.set_bus_dry(Mixer::MUSIC, true);
        assert!(mixer.take_dirty(), "a sound already playing on the bus has to hear it");
        assert!(mixer.is_bus_dry(Mixer::MUSIC) && !mixer.is_bus_dry(Mixer::SFX));
        assert_eq!(mixer.volume_for(1), 0.5, "the bus's gain, and no underwater level");
        assert_eq!(mixer.cutoff_hz_for(1), 0);
        assert!(!mixer.reverb_reaches(1));
        assert_eq!(mixer.volume_for(2), UNDERWATER_VOLUME, "the other buses are muffled as before");
        assert_eq!(mixer.cutoff_hz_for(2), UNDERWATER_CUTOFF_HZ);
        assert!(mixer.reverb_reaches(2));

        mixer.set_bus_muted(Mixer::MUSIC, true);
        assert_eq!(mixer.volume_for(1), 0.0, "dry is not deaf to its mute");
        mixer.set_bus_muted(Mixer::MUSIC, false);

        // Routed to a bus that is in the world, a sound is muffled there.
        mixer.route(1, Mixer::SFX);
        assert_eq!((mixer.volume_for(1), mixer.cutoff_hz_for(1)), (UNDERWATER_VOLUME, UNDERWATER_CUTOFF_HZ));
        mixer.set_bus_dry(Mixer::MUSIC, false);
        mixer.route(1, Mixer::MUSIC);
        assert_eq!(mixer.cutoff_hz_for(1), UNDERWATER_CUTOFF_HZ, "and back in the world, so is the music");
    }

    /// One `Route` per sound ever played is a leak in anything that runs for an hour.
    #[test]
    fn dead_sinks_take_their_routes_with_them() {
        let mut mixer = Mixer::new();
        for id in 1..=10 {
            mixer.route(id, Mixer::SFX);
        }
        assert_eq!(mixer.routed_sinks(), 10);
        mixer.retain_routes(|id| id > 7);
        assert_eq!(mixer.routed_sinks(), 3);
        assert_eq!(mixer.volume_for(1), 0.0, "a collected sink is unknown again");
    }

    /// The flag is what lets a game turn a gain through `mixer_mut()` and have it heard, without
    /// the manager rewriting every sink every frame.
    #[test]
    fn a_moved_gain_is_reported_once() {
        let mut mixer = Mixer::new();
        assert!(!mixer.take_dirty(), "a fresh mixer has nothing to apply");
        mixer.set_bus_gain(Mixer::MUSIC, 0.5);
        assert!(mixer.take_dirty());
        assert!(!mixer.take_dirty(), "and it is cleared by the reading");

        // Per-sink changes are NOT mixer-wide: the manager writes that sink itself, on the spot.
        mixer.route(1, Mixer::SFX);
        mixer.set_sink_volume(1, 0.5);
        mixer.set_sink_pitch(1, 1.2);
        assert!(!mixer.take_dirty());
    }

    /// A bus becomes visible to a settings panel as soon as anything mentions it.
    #[test]
    fn a_bus_appears_when_it_is_first_mentioned() {
        let mut mixer = Mixer::new();
        assert_eq!(mixer.buses().count(), 0);
        mixer.route(1, Mixer::MUSIC);
        mixer.set_bus_gain(Mixer::VOICE, 0.7);
        let mut names: Vec<&str> = mixer.buses().collect();
        names.sort_unstable();
        assert_eq!(names, vec![Mixer::MUSIC, Mixer::VOICE]);
    }

    /// A fade moves the factor linearly and lands exactly, a fade-out asks for its sink to be
    /// stopped once, and the spatial system rewriting the volume every frame does not undo it.
    #[test]
    fn a_fade_moves_its_factor_and_survives_volume_writes() {
        let mut mixer = Mixer::new();
        mixer.route(1, Mixer::MUSIC);
        mixer.set_sink_volume(1, 0.5);
        assert!(mixer.start_fade(1, 0.0, 2.0, true));
        let step = mixer.advance_fades(0.5);
        assert_eq!(step.changed, vec![1]);
        assert!(step.stop.is_empty());
        assert!((mixer.fade_of(1) - 0.75).abs() < 1e-6);
        // What `update_spatial_sink` does every frame: write the distance volume.
        mixer.set_sink_volume(1, 0.5);
        assert!((mixer.volume_for(1) - 0.5 * 0.75).abs() < 1e-6, "the fade is still applied");
        assert!(mixer.is_fading(1));
        let step = mixer.advance_fades(5.0);
        assert_eq!(step.stop, vec![1], "the fade-out ended: stop it");
        assert_eq!(mixer.fade_of(1), 0.0);
        assert_eq!(mixer.volume_for(1), 0.0);
        assert!(!mixer.is_fading(1));
        assert!(mixer.advance_fades(1.0).stop.is_empty(), "and only once");
        assert!(!mixer.start_fade(99, 1.0, 1.0, false), "no such sink");
    }

    /// Reversing mid-fade starts from where it is, not from the start; a zero-length fade lands
    /// on the next step; a fade-in from silence ends at the target and stops nothing.
    #[test]
    fn a_reversed_or_instant_fade_does_not_jump() {
        let mut mixer = Mixer::new();
        mixer.route(2, Mixer::MUSIC);
        mixer.set_fade(2, 0.0);
        mixer.start_fade(2, 1.0, 1.0, false);
        mixer.advance_fades(0.4);
        mixer.start_fade(2, 0.0, 1.0, false);
        mixer.advance_fades(0.0);
        assert!((mixer.fade_of(2) - 0.4).abs() < 1e-6, "reversed from 0.4: {}", mixer.fade_of(2));
        mixer.start_fade(2, 0.8, 0.0, false);
        let step = mixer.advance_fades(0.0);
        assert_eq!(mixer.fade_of(2), 0.8);
        assert!(step.stop.is_empty() && !mixer.is_fading(2));
        mixer.start_fade(2, 0.3, 1.0, false);
        mixer.retain_routes(|_| false);
        assert!(!mixer.is_fading(2), "a dead sink takes its fade with it");
    }

    /// A send that would come back to where it started is refused, and so is one to itself; a
    /// level of zero takes a send away.
    #[test]
    fn a_send_that_closes_a_loop_is_refused() {
        let mut mixer = Mixer::new();
        mixer.set_send("a", "b", 0.5).unwrap();
        mixer.set_send("b", "c", 0.5).unwrap();
        assert_eq!(
            mixer.set_send("c", "a", 0.5),
            Err(SendError::Loop { from: "c".into(), to: "a".into(), through: vec!["c".into(), "a".into(), "b".into(), "c".into()] })
        );
        assert_eq!(mixer.set_send("a", "a", 0.5), Err(SendError::ToItself("a".into())));
        assert_eq!(mixer.send_level("c", "a"), 0.0, "a refused send changes nothing");
        mixer.set_send("a", "c", 0.25).unwrap();
        assert_eq!(mixer.sends().collect::<Vec<_>>(), vec![("a", "b", 0.5), ("a", "c", 0.25), ("b", "c", 0.5)]);
        mixer.set_send("b", "c", 0.0).unwrap();
        assert_eq!(mixer.send_level("b", "c"), 0.0);
        assert!(mixer.set_send("c", "b", 0.5).is_ok(), "with b → c gone, c → b closes nothing");
    }

}
