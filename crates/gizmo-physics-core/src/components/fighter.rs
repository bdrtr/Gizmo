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
    /// Counted by [`FrameData::total_frames`] and therefore honoured by
    /// [`FighterController::tick`], which is what ends the move: a fighter stays committed
    /// through its recovery frames and only returns to neutral after them. No other method
    /// reads it — nothing here makes a recovering fighter more vulnerable, that is the game's
    /// to model.
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

impl FrameData {
    /// How many frames the move lasts in total: `startup + active + recovery`.
    ///
    /// The one definition of a move's length — [`FighterController::tick`] ends a move by this
    /// number, so a game that sums the three phases itself and a game that calls this cannot
    /// disagree. Saturating rather than wrapping: three phase lengths near `u32::MAX` are
    /// nonsense either way, and a wrapped total would end the move on its first frame.
    ///
    /// `0` (all three phases empty) describes a move with no frames at all; `tick` ends such a
    /// move immediately rather than leaving the fighter committed to it forever.
    #[inline]
    pub fn total_frames(&self) -> u32 {
        self.startup
            .saturating_add(self.active)
            .saturating_add(self.recovery)
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
/// [`FighterController::tick`] is the clock: it counts `hitstop_frames`/`hitstun_frames` down
/// and advances `current_move_frame`, and something must call it once per fixed frame.
/// `gizmo-physics-dynamics`' `fighter_frame_system` is that caller for an ECS game — the engine
/// registers it with the other gameplay systems — and a game stepping by hand calls `tick`
/// itself. Before that clock existed nothing called anything at all: the counters stood still,
/// so a hitstop applied from Lua froze its fighter permanently and no move ever reached its
/// active window.
///
/// `input_buffer` is the one part still on the game: nothing here feeds it, because what to
/// record is the game's action names. Every duration is a frame count rather than seconds
/// because the component has no `dt` to work with — it counts ticks, and cannot express time
/// any other way.
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
    /// Advanced by [`FighterController::tick`], one frame per call, which is also what ends
    /// the move (at [`FrameData::total_frames`]) and resets this to `0`. Written directly it is
    /// not validated: an index past the move's total length is not rejected, it simply falls
    /// outside the active window until the next tick ends the move.
    pub current_move_frame: u32,
    
    // Combo / Input handling
    /// Rolling motion/command history that combo recognition reads, 60 frames deep as
    /// constructed by [`FighterController::new`] and `default()`.
    ///
    /// Nothing in the engine fills it — not even [`FighterController::tick`], which is a clock
    /// and has no input to read. Feeding it is the game's, via
    /// [`FighterInputBuffer::update`](gizmo_core::input::FighterInputBuffer::update) with the
    /// action names that game binds. Left alone it stays empty, and every combo query over it
    /// answers `false`.
    ///
    /// `#[serde(skip)]`: a saved fighter comes back with a default-constructed buffer, so a
    /// depth configured at spawn is silently lost across a round trip and any motion input in
    /// flight is forgotten.
    #[serde(skip)]
    pub input_buffer: FighterInputBuffer,
    
    // Hitstop / Hitstun (Kare cinsinden bekleme süresi)
    /// Frames of hit-freeze still pending; `0` when the fighter is not frozen.
    ///
    /// Written wholesale by [`FighterController::apply_hitstop`], read by
    /// [`FighterController::is_locked`] and counted down one per [`FighterController::tick`].
    /// `#[serde(skip)]`, hence `0` after a load.
    #[serde(skip)]
    pub hitstop_frames: u32,
    /// Frames of stun still pending; `0` when the fighter is free to act.
    ///
    /// Differs from `hitstop_frames` in what entering it costs, not in how it is spent:
    /// [`FighterController::apply_hitstun`] additionally cancels the active move and drops the
    /// guard, whereas a hitstop leaves the move intact to resume where it froze. Equally
    /// `#[serde(skip)]`, and counted down by the same [`FighterController::tick`].
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
    
    /// Apply hitstop (freeze) when the character takes damage or blocks
    pub fn apply_hitstop(&mut self, frames: u32) {
        self.hitstop_frames = frames;
    }

    /// Apply stun
    pub fn apply_hitstun(&mut self, frames: u32) {
        self.hitstun_frames = frames;
        self.active_move = None;
        self.current_move_frame = 0;
        self.is_blocking = false;
    }

    /// Is the character currently locked (animation frozen or stunned)
    pub fn is_locked(&self) -> bool {
        self.hitstop_frames > 0 || self.hitstun_frames > 0
    }

    /// Are we inside the active attack's 'Damage-Dealing' (Active) frames?
    pub fn is_in_active_window(&self) -> bool {
        if let Some(move_data) = &self.active_move {
            let fd = &move_data.frame_data;
            self.current_move_frame >= fd.startup && self.current_move_frame < (fd.startup + fd.active)
        } else {
            false
        }
    }

    /// Advances this fighter by **one fixed frame**: counts hitstop and hitstun down, and moves
    /// the active move to its next frame unless the fighter is frozen or stunned.
    ///
    /// This is the clock the rest of the type is written against — `apply_hitstop` sets a
    /// duration, `is_locked` reports one is pending, `is_in_active_window` reads a frame index,
    /// and every one of them stands still until something calls this. Call it exactly once per
    /// fixed step; `gizmo_physics_dynamics::fighter_frame_system` is that caller for an ECS
    /// game, and a game stepping by hand calls it directly.
    ///
    /// **The lock is read before it is spent.** A fighter that enters the frame with
    /// `hitstop_frames == 1` still spends this frame frozen and only advances on the next one,
    /// so `apply_hitstop(n)` freezes for exactly `n` frames rather than `n - 1`.
    ///
    /// **Hitstop freezes the move, hitstun has already cancelled it.** Both stop the frame
    /// index, but a hitstop leaves `active_move` intact so the move resumes where it froze,
    /// which is what [`FighterController::apply_hitstun`] deliberately does not do.
    ///
    /// **What `current_move_frame` counts.** Index `0` is the frame the move was authored on —
    /// the value [`FighterController::apply_hitstun`] and the scripting API's `set_move` leave
    /// behind — and the first tick moves it to `1`. So a game reading the index *after* a fixed
    /// step sees the number of frames the move has run, index `0` exists only in between, and a
    /// move of `total_frames()` frames occupies exactly that many ticks. The active window is
    /// open on exactly `active` of them.
    ///
    /// A move whose [`FrameData::total_frames`] is `0` ends on its first tick instead of
    /// pinning the fighter to a move it can never finish.
    pub fn tick(&mut self) {
        let was_locked = self.is_locked();

        self.hitstop_frames = self.hitstop_frames.saturating_sub(1);
        self.hitstun_frames = self.hitstun_frames.saturating_sub(1);

        if was_locked {
            return;
        }

        let Some(move_data) = &self.active_move else {
            return;
        };
        let total = move_data.frame_data.total_frames();
        let next = self.current_move_frame.saturating_add(1);
        if total == 0 || next >= total {
            self.active_move = None;
            self.current_move_frame = 0;
        } else {
            self.current_move_frame = next;
        }
    }
}

#[cfg(feature = "ecs")]
gizmo_core::impl_component!(FighterController);

#[cfg(test)]
mod tests {
    use super::*;

    /// A fighter with a `startup`/`active`/`recovery` move already running from frame 0.
    fn with_move(startup: u32, active: u32, recovery: u32) -> FighterController {
        let frame_data = FrameData {
            startup,
            active,
            recovery,
            ..Default::default()
        };
        FighterController {
            active_move: Some(CombatMove {
                name: "test".to_string(),
                frame_data,
            }),
            current_move_frame: 0,
            ..Default::default()
        }
    }

    /// The defect this clock exists for: a hitstop applied and never counted down.
    ///
    /// `apply_hitstop(3)` used to be permanent — `is_locked` answered `true` for the rest of
    /// the process because nothing in the engine subtracted from the counter. Three frames must
    /// cost exactly three frames: locked after the first and second tick, free after the third.
    #[test]
    fn hitstop_lasts_exactly_the_frames_it_was_given() {
        let mut f = FighterController::default();
        f.apply_hitstop(3);

        for spent in 1..=3 {
            assert!(f.is_locked(), "tick {spent}: still inside the freeze");
            f.tick();
        }
        assert!(!f.is_locked(), "three frames of hitstop must end after three ticks");
        assert_eq!(f.hitstop_frames, 0);

        // And it stays free — the saturating subtraction must not wrap into a fresh freeze.
        f.tick();
        assert!(!f.is_locked(), "a spent counter must not wrap around");
        assert_eq!(f.hitstop_frames, 0);
    }

    /// Hitstun counts down the same way, and its extra cost (the cancelled move) is applied at
    /// `apply_hitstun` time, not at tick time.
    #[test]
    fn hitstun_counts_down_and_leaves_the_move_cancelled() {
        let mut f = with_move(5, 3, 2);
        f.apply_hitstun(2);
        assert!(f.active_move.is_none(), "stun cancels the move on the spot");

        f.tick();
        assert!(f.is_locked(), "second stun frame");
        f.tick();
        assert!(!f.is_locked(), "two frames of stun must end after two ticks");
    }

    /// A frozen fighter's move does not advance, and resumes on exactly the frame it froze on.
    #[test]
    fn hitstop_freezes_the_move_and_it_resumes_where_it_stopped() {
        let mut f = with_move(5, 3, 2);
        f.tick();
        f.tick();
        assert_eq!(f.current_move_frame, 2);

        f.apply_hitstop(2);
        f.tick();
        f.tick();
        assert_eq!(
            f.current_move_frame, 2,
            "the move must not advance while the fighter is frozen"
        );
        assert!(f.active_move.is_some(), "a hitstop does not cancel the move");

        f.tick();
        assert_eq!(f.current_move_frame, 3, "and it resumes from where it froze");
    }

    /// The active window opens for exactly `active` frames, and the move ends after exactly
    /// `total_frames()` of them — recovery included, which is the half nothing used to honour.
    #[test]
    fn a_move_runs_for_its_total_length_and_hits_for_its_active_window() {
        let (startup, active, recovery) = (5, 3, 2);
        let mut f = with_move(startup, active, recovery);
        assert_eq!(f.active_move.as_ref().unwrap().frame_data.total_frames(), 10);

        let mut hitting = Vec::new();
        for tick in 1..=10 {
            f.tick();
            if f.is_in_active_window() {
                hitting.push(tick);
            }
        }

        assert_eq!(
            hitting,
            vec![5, 6, 7],
            "the window must open on the startup-th frame and stay open for `active` frames"
        );
        assert!(
            f.active_move.is_none(),
            "after startup+active+recovery frames the fighter is back to neutral"
        );
        assert_eq!(f.current_move_frame, 0, "and the index resets with it");
    }

    /// A move with no frames at all must not pin the fighter to it forever.
    #[test]
    fn a_zero_length_move_ends_on_its_first_tick() {
        let mut f = with_move(0, 0, 0);
        f.tick();
        assert!(f.active_move.is_none());
        assert_eq!(f.current_move_frame, 0);
    }

    /// Ticking a neutral fighter is a no-op — the system runs over every entity every frame, so
    /// the common case must cost nothing and change nothing.
    #[test]
    fn ticking_a_neutral_fighter_changes_nothing() {
        let mut f = FighterController {
            health: 73.0,
            is_blocking: true,
            ..Default::default()
        };
        f.tick();
        assert!(f.active_move.is_none());
        assert_eq!(f.current_move_frame, 0);
        assert_eq!(f.hitstop_frames, 0);
        assert_eq!(f.hitstun_frames, 0);
        assert!(f.is_blocking, "the clock does not touch stance");
        assert_eq!(f.health, 73.0, "nor health");
    }

    /// `total_frames` saturates rather than wrapping: a wrapped total would read as a move that
    /// ends immediately, which is the opposite of what absurd phase lengths describe.
    #[test]
    fn total_frames_saturates() {
        let fd = FrameData {
            startup: u32::MAX,
            active: 10,
            recovery: 10,
            ..Default::default()
        };
        assert_eq!(fd.total_frames(), u32::MAX);
    }
}
