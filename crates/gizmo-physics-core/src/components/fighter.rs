use serde::{Deserialize, Serialize};
use gizmo_core::input::FighterInputBuffer;

/// Timing of a single fighting-game move, measured in **frames** rather than seconds.
///
/// The three phase lengths partition a move into `startup` → `active` → `recovery`, giving a
/// total of `startup + active + recovery` frames that
/// [`FighterController::current_move_frame`] walks from 0. Nothing here converts to seconds:
/// stepped at a fixed 60 Hz the defaults below describe a 30-frame (half-second) move, and the
/// same numbers mean a different duration at any other step rate. Being integers, the counts
/// are exact — they never accumulate the rounding a `dt`-summing timer would.
///
/// `#[non_exhaustive]`, so other crates build one from [`FrameData::default`] and overwrite
/// the fields they care about; struct literals (including `..Default::default()`) are not
/// available to them.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct FrameData {
    /// Wind-up frames before the move can hit — frames `0 .. startup` of the move.
    ///
    /// `0` means the attack is already hitting on its first frame.
    pub startup: u32,
    /// Length of the hitting window in frames: the move connects on frames
    /// `startup .. startup + active`, the half-open range
    /// [`FighterController::is_in_active_window`] tests.
    ///
    /// `0` is accepted and yields a move with no hitting window at all, which can never
    /// connect however long it is held.
    pub active: u32,
    /// Frames of lock-out after the hitting window closes, during which the fighter is still
    /// committed to the move and vulnerable.
    ///
    /// Descriptive only: no method on [`FrameData`] or [`FighterController`] reads it. The
    /// code that advances `current_move_frame` is what decides when a move ends, so recovery
    /// is honoured only if that code sums the three phases itself.
    pub recovery: u32,
    /// Damage a clean hit with this move deals, in the same health points as
    /// [`FighterController::health`] (whose default full bar is 100).
    ///
    /// Nothing on this type subtracts it, and it is neither clamped nor sign-checked. When a
    /// move drives a hitbox it also duplicates
    /// [`Hitbox::damage`](crate::components::hitbox::Hitbox::damage); the two are independent
    /// fields and nothing keeps them in agreement.
    pub damage: f32,
    /// Frames of stun this move inflicts on the fighter it hits — the value to pass to that
    /// fighter's [`FighterController::apply_hitstun`].
    ///
    /// Stored on the attacking move rather than on the victim, and nothing here transfers it.
    pub hitstun: u32,
    /// Frames of freeze on connect, to pass to [`FighterController::apply_hitstop`].
    ///
    /// Who gets frozen is not decided here: this is only a duration, and `apply_hitstop`
    /// freezes whichever controller it is called on.
    pub hitstop: u32,
}

impl Default for FrameData {
    fn default() -> Self {
        Self {
            startup: 10,
            active: 5,
            recovery: 15,
            damage: 10.0,
            hitstun: 20,
            hitstop: 5,
        }
    }
}

/// A named attack: an identifier plus its [`FrameData`] timing, and nothing else.
///
/// That is the complete move description — no animation reference, hitbox list or cancel
/// table hangs off it, so everything past the timing lives in the game's own data. Its
/// derived `Default` inherits [`FrameData`]'s hand-written one rather than a zeroed struct,
/// so `CombatMove::default()` is an unnamed but immediately usable 10/5/15 attack.
///
/// `#[non_exhaustive]` — other crates construct it via `default()` and then assign fields.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[non_exhaustive]
pub struct CombatMove {
    /// Free-form move identifier, e.g. `"Hadouken"`; empty for `CombatMove::default()`.
    ///
    /// Never parsed or matched against anything in this crate, and not required to be unique
    /// — it is a label for the game's own move table.
    pub name: String,
    /// Phase timing for this move, owned rather than shared.
    ///
    /// Moving a `CombatMove` into [`FighterController::active_move`] therefore copies the
    /// timing, so retuning one fighter's in-flight move cannot disturb the table it came from
    /// — nor will edits to the table reach a move already in progress.
    pub frame_data: FrameData,
}

/// Per-fighter combat state for a 2D-style fighting game: health, stance flags, the move
/// currently being executed, and the frame counters that freeze or stun the character.
///
/// Pure data with no scheduler behind it. Nothing in this crate advances
/// `current_move_frame`, feeds `input_buffer`, or counts `hitstop_frames`/`hitstun_frames`
/// down — the game (or a script) must tick them once per fixed frame. That is also why every
/// duration here is a frame count rather than seconds: the component has no `dt` to work
/// with, so it cannot express time any other way.
///
/// Three fields are `#[serde(skip)]` (`input_buffer`, `hitstop_frames`, `hitstun_frames`), so
/// a save/load round trip yields a fighter with a fresh 60-frame input buffer and neither
/// freeze nor stun pending — [`FighterController::is_locked`] reports `false` immediately
/// after loading whatever it reported when saved. The authored state (health, stance, the
/// active move and its frame index) does survive.
///
/// `#[non_exhaustive]`: other crates construct it with [`FighterController::new`] or
/// `default()` and then assign fields.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct FighterController {
    /// Player slot this fighter occupies; `1` by default.
    ///
    /// An identity tag only — no method on this type reads it, nothing enforces that two
    /// fighters carry different ids, and `u8` admits values far outside the 1/2 a versus
    /// match needs.
    pub player_id: u8,
    /// Current health, in arbitrary points on the same scale as `max_health` (default bar:
    /// 100).
    ///
    /// Never touched by this type: it is not clamped to `0..=max_health`, hitting `0` or going
    /// negative triggers nothing here, and no code in this crate subtracts damage from it.
    pub health: f32,
    /// Full-bar reference value, `100.0` by default — the denominator for a health fraction.
    ///
    /// Nothing enforces `health <= max_health` and `0.0` is accepted, so code computing
    /// `health / max_health` has to guard against a zero or undersized bar itself.
    pub max_health: f32,
    /// Whether the fighter is holding guard this frame.
    ///
    /// [`FighterController::apply_hitstun`] clears it — being stunned drops the guard — but
    /// nothing here ever raises it; that is the input handler's job.
    pub is_blocking: bool,
    /// Whether the fighter is crouching this frame.
    ///
    /// Unlike `is_blocking`, no method here ever changes it: a stunned fighter keeps whatever
    /// crouch state it had.
    pub is_crouching: bool,
    
    // Aktif saldırı durumu ve Frame Data takibi
    /// The move being executed, or `None` when the fighter is neutral.
    ///
    /// This and `current_move_frame` are one unit of state — the frame index means nothing
    /// while this is `None`, and [`FighterController::apply_hitstun`] clears both together to
    /// cancel an attack outright.
    pub active_move: Option<CombatMove>,
    /// Frame index within `active_move`, `0` on the move's first startup frame.
    ///
    /// Never advanced here; the driving code increments it once per fixed frame and is also
    /// what decides the move is over, since this type only ever clears `active_move` through
    /// [`FighterController::apply_hitstun`]. Indices past the move's total length are not
    /// rejected — they simply fall outside the active window.
    pub current_move_frame: u32,
    
    // Combo / Input handling
    /// Rolling motion/command history that combo recognition reads, 60 frames deep as
    /// constructed by [`FighterController::new`] and `default()`.
    ///
    /// `#[serde(skip)]`: a saved fighter comes back with a default-constructed buffer, so a
    /// depth configured at spawn is silently lost across a round trip and any motion input in
    /// flight is forgotten.
    #[serde(skip)]
    pub input_buffer: FighterInputBuffer,
    
    // Hitstop / Hitstun (Kare cinsinden bekleme süresi)
    /// Frames of hit-freeze still pending; `0` when the fighter is not frozen.
    ///
    /// Written wholesale by [`FighterController::apply_hitstop`] and read by
    /// [`FighterController::is_locked`], but never decremented here — the game counts it down.
    /// `#[serde(skip)]`, hence `0` after a load.
    #[serde(skip)]
    pub hitstop_frames: u32,
    /// Frames of stun still pending; `0` when the fighter is free to act.
    ///
    /// Differs from `hitstop_frames` in what entering it costs, not in how it is spent:
    /// [`FighterController::apply_hitstun`] additionally cancels the active move and drops the
    /// guard, whereas a hitstop leaves the move intact to resume where it froze. Equally
    /// `#[serde(skip)]` and equally never decremented here.
    #[serde(skip)]
    pub hitstun_frames: u32,

    /// Ground walking speed in metres per second (`3.0` by default).
    ///
    /// A tunable this type never uses: the component moves nothing, so turning it into an
    /// actual velocity is the game's job.
    pub walk_speed: f32,
    /// Dash speed in metres per second (`10.0` by default) — the same kind of unused tunable
    /// as `walk_speed`. Nothing requires it to exceed the walking speed.
    pub dash_speed: f32,
}

impl Default for FighterController {
    fn default() -> Self {
        Self {
            player_id: 1,
            health: 100.0,
            max_health: 100.0,
            is_blocking: false,
            is_crouching: false,
            active_move: None,
            current_move_frame: 0,
            input_buffer: FighterInputBuffer::new(60), // 1 saniyelik buffer (60fps)
            hitstop_frames: 0,
            hitstun_frames: 0,
            walk_speed: 3.0,
            dash_speed: 10.0,
        }
    }
}

impl FighterController {
    /// Creates a fighter for the given player slot with every other field left at its
    /// [`Default`] value: a full 100-point bar, neutral stance, no active move, a 60-frame
    /// input buffer, and 3 m/s walk / 10 m/s dash speeds.
    ///
    /// `player_id` is stored verbatim — it is not validated against the 1/2 a versus match
    /// uses, and no registration or uniqueness check happens anywhere.
    pub fn new(player_id: u8) -> Self {
        Self {
            player_id,
            ..Default::default()
        }
    }
    
    /// Karakter hasar yediğinde veya blokladığında hitstop (donma) uygula
    pub fn apply_hitstop(&mut self, frames: u32) {
        self.hitstop_frames = frames;
    }

    /// Sersemletme uygula
    pub fn apply_hitstun(&mut self, frames: u32) {
        self.hitstun_frames = frames;
        self.active_move = None;
        self.current_move_frame = 0;
        self.is_blocking = false;
    }

    /// Karakter şu an kilitli mi (animasyon donmuş veya sersemlemiş)
    pub fn is_locked(&self) -> bool {
        self.hitstop_frames > 0 || self.hitstun_frames > 0
    }

    /// Aktif saldırının 'Hasar Veren' (Active) kareleri içinde miyiz?
    pub fn is_in_active_window(&self) -> bool {
        if let Some(move_data) = &self.active_move {
            let fd = &move_data.frame_data;
            self.current_move_frame >= fd.startup && self.current_move_frame < (fd.startup + fd.active)
        } else {
            false
        }
    }
}

#[cfg(feature = "ecs")]
gizmo_core::impl_component!(FighterController);
