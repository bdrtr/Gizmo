//! Clip-driven finite state machine for skeletal animation, with cross-fade blending.
//!
//! This module is **pure data plus lookup logic**: it stores the state/transition graph and
//! the in-flight blend, but never advances time and never touches a skeleton itself. All
//! motion comes from the driver, `gizmo_renderer::animation_state_machine_update_system`,
//! which once per frame advances [`AnimationStateMachine::current_time`], evaluates queued
//! triggers and exit-time transitions, samples the clip (or the two blended clips) and writes
//! the resulting bone poses into the entity's `Skeleton` component.
//!
//! Two consequences of that split are easy to trip over:
//!
//! - An [`AnimationStateMachine`] on an entity that has **no** `Skeleton` component is inert.
//!   The driver bails out before advancing anything, so the playhead never moves and queued
//!   triggers are never evaluated.
//! - No engine-side plugin schedules that driver. `gizmo-studio` runs only the simple
//!   [`super::AnimationPlayer`] system, so an app that wants the FSM has to call
//!   `animation_state_machine_update_system` itself each frame with the frame's `dt`.
//!
//! Nothing here validates the graph, and nothing panics. An unknown
//! [`AnimationStateMachine::current_state`] or an out-of-range [`AnimationState::clip_index`]
//! degrade to "this entity is not posed this frame", leaving the skeleton at its last pose. A
//! transition whose `to` names no known state behaves differently: the driver simply does not
//! start the blend, so the machine stays where it is and keeps animating the current state —
//! the transition is dead configuration, not a freeze. Either way a typo surfaces as a
//! character that does not move as intended rather than as an error. Each accessor documents
//! the exact fallback it takes.

use super::clip::AnimationClip;
use std::sync::Arc;

/// A single state in the animation state machine — names one clip.
#[derive(Clone, Debug)]
pub struct AnimationState {
    /// The key this state is addressed by: [`AnimationTransition::from`],
    /// [`AnimationTransition::to`] and [`AnimationStateMachine::current_state`] all hold this
    /// string.
    ///
    /// Resolved by exact string equality in a linear scan
    /// ([`AnimationStateMachine::find_state`]), so a duplicate name is not an error — the
    /// first state wins and the later one is simply unreachable. `"*"` carries no special
    /// meaning here; the wildcard exists only on [`AnimationTransition::from`].
    pub name: String,

    /// Index into [`AnimationStateMachine::clips`] of the clip this state plays.
    ///
    /// Never bounds-checked at construction. An out-of-range index makes the driver skip the
    /// entity for that frame — the skeleton keeps whatever pose it last had — and
    /// [`AnimationStateMachine::current_clip_duration`] falls back to 1.0 s.
    pub clip_index: usize,

    /// Whether the playhead wraps at the end of the clip (`rem_euclid` into `[0, duration)`)
    /// or clamps to `duration` and stops there.
    ///
    /// This also decides how long an exit-time transition stays armed. A looped state reports
    /// "clip finished" only on the frame it wraps; a non-looped state has its playhead pinned
    /// at `duration`, so it reports finished on *every* frame after the clip ends, until some
    /// transition moves it elsewhere.
    pub looped: bool,

    /// Playback rate as a dimensionless multiplier on the frame `dt` — `1.0` is the clip's
    /// authored rate, `2.0` plays a 2 s clip in 1 s of wall clock. Not a speed in metres per
    /// second.
    ///
    /// Negative values play the clip backwards; the driver normalizes with `rem_euclid`, so a
    /// looped state wraps around to the clip's end instead of sampling at a negative time.
    /// `0.0` freezes the playhead where it is.
    pub speed: f32,
}

/// A directed transition between two named states.
#[derive(Clone, Debug)]
pub struct AnimationTransition {
    /// Source state name (`"*"` matches any state).
    pub from: String,
    /// Destination state name.
    pub to: String,
    /// Cross-fade duration in seconds.
    pub blend_duration: f32,
    /// Optional trigger string that activates this transition.
    /// If `None` the transition fires automatically when the source clip ends
    /// (only meaningful when `has_exit_time` is `true`).
    pub trigger: Option<String>,
    /// When `true` the transition may only start once the source clip has
    /// finished at least one full play-through.
    pub has_exit_time: bool,
}

/// Per-entity state tracked while a cross-fade blend is in progress.
#[derive(Clone, Debug)]
pub struct ActiveBlend {
    /// Index into [`AnimationStateMachine::clips`] of the outgoing clip, captured from
    /// [`AnimationStateMachine::current_clip_index`] when the blend started (falling back to
    /// `0` if the state was unknown). Sampled at the frozen [`from_time`](Self::from_time).
    pub from_clip: usize,

    /// Index into [`AnimationStateMachine::clips`] of the incoming clip, copied from the
    /// destination state's [`AnimationState::clip_index`].
    ///
    /// If either this or [`from_clip`](Self::from_clip) is out of range the driver skips the
    /// entity for the rest of the frame, so a mis-configured transition freezes the character
    /// mid-blend rather than panicking.
    pub to_clip: usize,

    /// Time into the source clip at the moment the blend started.
    ///
    /// **Never advanced** while the blend runs: the outgoing clip is a still frame that the
    /// incoming one fades over. That is why there is no `from_speed` counterpart to
    /// [`to_speed`](Self::to_speed).
    pub from_time: f32,
    /// Time into the destination clip (advances each frame).
    pub to_time: f32,
    /// Seconds elapsed since blend began.
    pub elapsed: f32,

    /// Cross-fade length in seconds, copied from [`AnimationTransition::blend_duration`].
    ///
    /// [`elapsed`](Self::elapsed) accumulates the raw frame `dt`, never scaled by any state's
    /// `speed`, so a blend always takes this many seconds of wall clock however fast the clips
    /// themselves play. `<= 0.0` makes [`alpha`](Self::alpha) return `1.0` straight away, so
    /// the transition completes on the frame it starts — an instant snap with no visible
    /// cross-fade.
    pub duration: f32,

    /// Name of the state being blended into; moved into
    /// [`AnimationStateMachine::current_state`] once [`alpha`](Self::alpha) reaches `1.0`.
    ///
    /// Because that switch happens only at the end, the machine reports the *outgoing* state
    /// for the whole cross-fade — [`AnimationStateMachine::current_state`],
    /// [`current_speed`](AnimationStateMachine::current_speed) and
    /// [`is_current_looped`](AnimationStateMachine::is_current_looped) all still describe
    /// where you came from.
    pub to_state: String,

    /// Copy of the destination [`AnimationState::looped`], used to normalize
    /// [`to_time`](Self::to_time) while the blend is running — before
    /// [`AnimationStateMachine::current_state`] has switched over.
    ///
    /// Snapshotted rather than re-resolved each frame, so editing or removing the destination
    /// state mid-blend cannot change how the incoming clip is sampled.
    pub to_looped: bool,

    /// Copy of the destination [`AnimationState::speed`]; multiplies the frame `dt` when
    /// advancing [`to_time`](Self::to_time), so the incoming clip already runs at its own rate
    /// while it fades in. Snapshotted for the same reason as [`to_looped`](Self::to_looped).
    pub to_speed: f32,
}

impl ActiveBlend {
    /// Blend weight: 0.0 = fully source, 1.0 = fully destination.
    #[inline]
    pub fn alpha(&self) -> f32 {
        if self.duration <= 0.0 {
            1.0
        } else {
            (self.elapsed / self.duration).clamp(0.0, 1.0)
        }
    }
}

/// ECS component — full animation state machine with cross-fade blending.
///
/// # Usage
/// ```
/// use std::sync::Arc;
/// use gizmo_animation::skeletal::{
///     AnimationClip, AnimationState, AnimationStateMachine, AnimationTransition,
/// };
/// # fn clip(name: &str, duration: f32) -> AnimationClip {
/// #     AnimationClip {
/// #         name: name.into(),
/// #         duration,
/// #         translations: Vec::new(),
/// #         rotations: Vec::new(),
/// #         scales: Vec::new(),
/// #     }
/// # }
/// # let clips: Arc<[AnimationClip]> =
/// #     Arc::from(vec![clip("idle", 1.0), clip("run", 0.8), clip("jump", 0.6)]);
/// let mut fsm = AnimationStateMachine::new(
///     "idle",
///     clips,
///     vec![
///         AnimationState { name: "idle".into(), clip_index: 0, looped: true, speed: 1.0 },
///         AnimationState { name: "run".into(),  clip_index: 1, looped: true, speed: 1.2 },
///         AnimationState { name: "jump".into(), clip_index: 2, looped: false, speed: 1.0 },
///     ],
///     vec![
///         AnimationTransition { from: "idle".into(), to: "run".into(),  blend_duration: 0.2, trigger: Some("run".into()),  has_exit_time: false },
///         AnimationTransition { from: "run".into(),  to: "idle".into(), blend_duration: 0.3, trigger: Some("stop".into()), has_exit_time: false },
///         AnimationTransition { from: "*".into(),    to: "jump".into(), blend_duration: 0.1, trigger: Some("jump".into()), has_exit_time: false },
///     ],
/// );
/// fsm.trigger("run");
///
/// // Queued, not applied: only the driver moves the machine, so the state, the
/// // (absent) cross-fade and the playhead are all still where `new` left them.
/// assert_eq!(fsm.current_state, "idle");
/// assert!(fsm.active_blend.is_none());
/// assert_eq!(fsm.current_time, 0.0);
/// assert_eq!(fsm.drain_triggers(), ["run"]);
///
/// // What the driver will do with that trigger — first matching rule, in order.
/// assert_eq!(
///     fsm.find_transition("idle", Some("run"), false).map(|t| t.to.as_str()),
///     Some("run")
/// );
/// ```
#[derive(Clone)]
pub struct AnimationStateMachine {
    /// The clip pool every [`AnimationState::clip_index`] indexes into.
    ///
    /// Shared behind an `Arc` because keyframe data is large and one rig's clips are normally
    /// used by every entity playing that rig: cloning this component (which the ECS does)
    /// copies a refcount, not the animation data. An empty pool makes the driver skip the
    /// entity entirely — the playhead does not even advance.
    pub clips: Arc<[AnimationClip]>,

    /// Every state the machine can occupy.
    ///
    /// Resolved by name, never by position, so the order here is free — but two states sharing
    /// a name leave the later one unreachable. An empty list makes the driver skip the entity
    /// entirely.
    pub states: Vec<AnimationState>,

    /// Candidate transitions, **in priority order**: [`find_transition`](Self::find_transition)
    /// returns the first match.
    ///
    /// Shadowing is per query, not global — a rule can only hide later rules that answer the
    /// *same* query (see [`find_transition`](Self::find_transition) for the two query modes),
    /// so an early `from: "*"` rule pre-empts only the rules carrying the same trigger, or,
    /// for a triggerless exit-time rule, the other exit-time rules. Where two rules do compete
    /// like that, list the specific one first.
    pub transitions: Vec<AnimationTransition>,

    /// Name of the state currently driving the pose.
    ///
    /// During a cross-fade this still names the *outgoing* state; it is replaced by
    /// [`ActiveBlend::to_state`] only once the blend completes. If it matches no entry in
    /// [`states`](Self::states) nothing panics — every accessor falls back (see each one) and
    /// the driver leaves the skeleton at its previous pose.
    pub current_state: String,

    /// Playhead in seconds into the current state's clip.
    ///
    /// The driver advances it by `dt` scaled by the state's [`speed`](AnimationState::speed),
    /// then normalizes: wrapped into `[0, duration)` for a looped state, clamped to
    /// `[0, duration]` otherwise. Starting
    /// a cross-fade does **not** reset it — it is captured into [`ActiveBlend::from_time`] and
    /// later overwritten with [`ActiveBlend::to_time`], so when the blend ends the incoming
    /// clip's playhead carries over instead of snapping back to zero.
    pub current_time: f32,

    /// The in-flight cross-fade, or `None` when a single clip is driving the pose.
    ///
    /// While this is `Some` the machine is **uninterruptible**: the driver evaluates no
    /// transitions at all, and the triggers it drained that frame are discarded rather than
    /// held over. A long [`AnimationTransition::blend_duration`] therefore swallows player
    /// input for its whole length, which is why gameplay-driven blends are usually kept to
    /// 0.1–0.3 s.
    pub active_blend: Option<ActiveBlend>,

    /// Triggers queued by [`trigger`](Self::trigger) since the last update, in call order.
    /// Neither merged nor de-duplicated — [`drain_triggers`](Self::drain_triggers) empties the
    /// whole batch once per update.
    pending_triggers: Vec<String>,
}

impl AnimationStateMachine {
    /// Builds a machine parked in `initial_state`, playhead at 0, no blend in progress — so
    /// the initial state pops in on the first frame without a cross-fade.
    ///
    /// Nothing is validated. `initial_state` need not name an entry in `states`, `states` may
    /// reference clip indices outside `clips`, and `transitions` may point at states that do
    /// not exist. The first two degrade to "this entity is not posed this frame" at update
    /// time; a transition into an unknown state is skipped instead, leaving the machine in
    /// the state it was already playing. Nothing fails here, which keeps data-authored graphs
    /// from aborting a load but also means a typo'd name shows up as a character that does
    /// not move as intended. The counts are logged at `debug` level on construction to make
    /// that diagnosable.
    pub fn new(
        initial_state: &str,
        clips: Arc<[AnimationClip]>,
        states: Vec<AnimationState>,
        transitions: Vec<AnimationTransition>,
    ) -> Self {
        tracing::debug!(
            initial = %initial_state,
            states = states.len(),
            transitions = transitions.len(),
            clips = clips.len(),
            "[Animation] state machine created"
        );
        Self {
            clips,
            states,
            transitions,
            current_state: initial_state.to_string(),
            current_time: 0.0,
            active_blend: None,
            pending_triggers: Vec::new(),
        }
    }

    /// Queue a trigger to be evaluated on the next
    /// `gizmo_renderer::animation_state_machine_update_system` pass.
    ///
    /// Queued, not applied — nothing changes until the driver runs, and the queue is lossier
    /// than a mailbox in two ways worth knowing:
    ///
    /// - Only the **first** queued trigger that matches a transition is taken; the rest of the
    ///   batch is drained and dropped along with it, so queuing `"jump"` and `"run"` in one
    ///   frame never fires both.
    /// - Triggers queued while a cross-fade is running are drained and **discarded**, because
    ///   the driver evaluates no transitions mid-blend. Re-queue on a later frame if the input
    ///   must not be lost.
    pub fn trigger(&mut self, name: &str) {
        tracing::debug!(
            trigger = %name,
            current = %self.current_state,
            "[Animation] FSM trigger queued"
        );
        self.pending_triggers.push(name.to_string());
    }

    /// Drain and return all pending triggers (consumed by the update system).
    pub fn drain_triggers(&mut self) -> Vec<String> {
        // Called every frame by the update system; only speak up when there is
        // actually something to drain to keep the per-frame path quiet.
        if !self.pending_triggers.is_empty() {
            tracing::trace!(
                count = self.pending_triggers.len(),
                current = %self.current_state,
                "[Animation] draining FSM triggers"
            );
        }
        self.pending_triggers.drain(..).collect()
    }

    // ── helpers ──────────────────────────────────────────────────────────────

    /// Looks a state up by exact name — linear scan over [`states`](Self::states), first match
    /// wins.
    ///
    /// `"*"` is *not* interpreted here: the wildcard is only meaningful as an
    /// [`AnimationTransition::from`], so a state literally named `"*"` is just an ordinary
    /// state. `None` means "unknown state", which every caller treats as a fallback condition
    /// rather than an error — it is what a typo in [`current_state`](Self::current_state) or
    /// in a transition's `to` looks like from here.
    pub fn find_state(&self, name: &str) -> Option<&AnimationState> {
        self.states.iter().find(|s| s.name == name)
    }

    /// Index into [`clips`](Self::clips) of the clip the current state plays, or `None` when
    /// [`current_state`](Self::current_state) names no known state — in which case the driver
    /// skips posing the entity and the skeleton holds its last pose.
    ///
    /// Reports the *outgoing* clip for the whole of a cross-fade; the incoming one is
    /// [`ActiveBlend::to_clip`].
    pub fn current_clip_index(&self) -> Option<usize> {
        self.find_state(&self.current_state).map(|s| s.clip_index)
    }

    /// Length of the current state's clip, in seconds.
    ///
    /// Falls back to **1.0 s** when the state or the clip is missing. That the fallback is
    /// non-zero is load-bearing: the driver detects clip completion with
    /// `current_time >= duration` and normalizes the playhead against the same value, so a
    /// `0.0` here would report the clip finished on every single frame.
    ///
    /// This is the clip's authored length and is unaffected by [`AnimationState::speed`] — at
    /// speed `2.0` a clip reporting 2.0 s here is done after 1 s of wall clock.
    pub fn current_clip_duration(&self) -> f32 {
        self.current_clip_index()
            .and_then(|i| self.clips.get(i))
            .map(|c| c.duration)
            .unwrap_or(1.0)
    }

    /// Playback rate of the current state: a dimensionless multiplier on the frame `dt`, not a
    /// speed in units per second. Negative values play the clip backwards.
    ///
    /// Falls back to `1.0` for an unknown state, so a mis-named state still advances time at
    /// the normal rate instead of freezing. Reports the *outgoing* state's rate during a
    /// cross-fade — the incoming clip is advanced with [`ActiveBlend::to_speed`].
    pub fn current_speed(&self) -> f32 {
        self.find_state(&self.current_state)
            .map(|s| s.speed)
            .unwrap_or(1.0)
    }

    /// Whether the current state's clip wraps at its end instead of clamping there.
    ///
    /// Falls back to **`true`** for an unknown state, which is the safe direction: the playhead
    /// keeps wrapping inside the 1.0 s fallback duration instead of pinning at the end, where
    /// `current_time >= duration` would hold on every subsequent frame and re-arm any wildcard
    /// exit-time transition, frame after frame.
    pub fn is_current_looped(&self) -> bool {
        self.find_state(&self.current_state)
            .map(|s| s.looped)
            .unwrap_or(true)
    }

    /// First transition that matches, scanning [`transitions`](Self::transitions) in order —
    /// declaration order *is* priority order.
    ///
    /// `from` is matched literally or by the `"*"` wildcard. The two query modes are
    /// deliberately disjoint:
    ///
    /// - **Trigger query** (`trigger` is `Some`) — only a transition carrying exactly that
    ///   trigger string can match. `clip_finished` is ignored, so a trigger fires mid-clip.
    /// - **Exit-time query** (`trigger` is `None`) — only a transition with no trigger *and*
    ///   `has_exit_time` can match, and only when `clip_finished` is `true`.
    ///
    /// Keeping them disjoint is a bug fix, not an accident. An auto transition listed *before*
    /// a trigger transition used to satisfy a specific trigger query on the frame the source
    /// clip happened to end, silently swallowing the player's input and jumping to the wrong
    /// state on clip-boundary frames (regression test
    /// `specific_trigger_not_hijacked_by_exit_time_transition`).
    ///
    /// Corollary worth knowing when authoring a graph: a transition with `trigger: None` and
    /// `has_exit_time: false` can satisfy neither query — it is dead configuration.
    pub fn find_transition(
        &self,
        from: &str,
        trigger: Option<&str>,
        clip_finished: bool,
    ) -> Option<&AnimationTransition> {
        self.transitions.iter().find(|tr| {
            // Source must match current state or wildcard
            let from_matches = tr.from == from || tr.from == "*";
            if !from_matches {
                return false;
            }

            // Trigger-based transition
            if let Some(ref req) = tr.trigger {
                if let Some(t) = trigger {
                    return t == req;
                }
                return false;
            }
            // Auto / exit-time transition (this transition has no trigger).
            // Only consider it on an exit-time query (caller passed no trigger);
            // a *specific* trigger query must never be satisfied by an unrelated
            // auto-transition just because the clip happens to have finished
            // (that would silently swallow the player's input and jump to the
            // wrong state on clip-boundary frames).
            if trigger.is_none() && tr.has_exit_time {
                clip_finished
            } else {
                false
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn machine(transitions: Vec<AnimationTransition>) -> AnimationStateMachine {
        AnimationStateMachine::new(
            "run",
            Arc::from(Vec::<AnimationClip>::new()),
            vec![],
            transitions,
        )
    }

    fn tr(from: &str, to: &str, trigger: Option<&str>, exit: bool) -> AnimationTransition {
        AnimationTransition {
            from: from.into(),
            to: to.into(),
            blend_duration: 0.1,
            trigger: trigger.map(Into::into),
            has_exit_time: exit,
        }
    }

    #[test]
    fn specific_trigger_not_hijacked_by_exit_time_transition() {
        // An auto (exit-time) transition ordered BEFORE the trigger transition
        // must not satisfy a specific trigger query on the frame the clip ends —
        // otherwise the "jump" input is swallowed and the FSM goes to idle.
        let m = machine(vec![
            tr("run", "idle", None, true),           // auto-return when run finishes
            tr("run", "jump", Some("jump"), false),  // jump on trigger
        ]);
        let hit = m
            .find_transition("run", Some("jump"), true)
            .expect("jump trigger must resolve");
        assert_eq!(
            hit.to, "jump",
            "specific trigger must win over an unrelated exit-time transition"
        );
    }

    #[test]
    fn exit_time_query_still_matches_auto_transition() {
        let m = machine(vec![tr("run", "idle", None, true)]);
        assert_eq!(
            m.find_transition("run", None, true).map(|t| t.to.as_str()),
            Some("idle"),
            "exit-time query fires the auto transition when the clip finished"
        );
        assert!(
            m.find_transition("run", None, false).is_none(),
            "auto transition must not fire before the clip finishes"
        );
    }

    #[test]
    fn trigger_transitions_ignore_clip_finished_and_unknown_triggers() {
        let m = machine(vec![tr("run", "jump", Some("jump"), false)]);
        // A trigger transition never fires on a bare exit-time query...
        assert!(m.find_transition("run", None, true).is_none());
        // ...nor for a non-matching trigger.
        assert!(m.find_transition("run", Some("crouch"), true).is_none());
        // ...and matches its own trigger regardless of clip_finished.
        assert_eq!(
            m.find_transition("run", Some("jump"), false).map(|t| t.to.as_str()),
            Some("jump")
        );
    }

    #[test]
    fn wildcard_source_matches_any_state() {
        let m = machine(vec![tr("*", "jump", Some("jump"), false)]);
        assert_eq!(
            m.find_transition("anything", Some("jump"), false)
                .map(|t| t.to.as_str()),
            Some("jump")
        );
    }

    // ── Cross-fade blend weight ────────────────────────────────────────

    fn active_blend(elapsed: f32, duration: f32) -> ActiveBlend {
        ActiveBlend {
            from_clip: 0,
            to_clip: 1,
            from_time: 0.0,
            to_time: 0.0,
            elapsed,
            duration,
            to_state: "run".into(),
            to_looped: true,
            to_speed: 1.0,
        }
    }

    #[test]
    fn active_blend_alpha_ramps_and_clamps() {
        assert_eq!(active_blend(0.0, 0.4).alpha(), 0.0, "start fully on the source");
        assert!((active_blend(0.2, 0.4).alpha() - 0.5).abs() < 1e-6, "linear ramp midpoint");
        assert_eq!(active_blend(0.4, 0.4).alpha(), 1.0, "end fully on the destination");
        // Overshooting the blend duration must clamp, not exceed 1.0.
        assert_eq!(active_blend(10.0, 0.4).alpha(), 1.0, "alpha clamps at 1.0");
    }

    #[test]
    fn active_blend_zero_duration_is_instant() {
        // A zero (or negative) blend duration snaps straight to the destination.
        assert_eq!(active_blend(0.0, 0.0).alpha(), 1.0);
        assert_eq!(active_blend(0.0, -1.0).alpha(), 1.0);
    }

    // ── Trigger queue ──────────────────────────────────────────────────

    #[test]
    fn triggers_drain_in_order_then_empty() {
        let mut m = machine(vec![]);
        m.trigger("run");
        m.trigger("jump");
        assert_eq!(m.drain_triggers(), vec!["run".to_string(), "jump".to_string()]);
        assert!(m.drain_triggers().is_empty(), "queue must be empty after draining");
    }

    // ── Current-state metadata accessors ───────────────────────────────

    #[test]
    fn metadata_reads_from_state_and_clip_when_present() {
        let clips: Arc<[AnimationClip]> = Arc::from(vec![AnimationClip {
            name: "run".into(),
            duration: 2.5,
            translations: vec![],
            rotations: vec![],
            scales: vec![],
        }]);
        let states = vec![AnimationState {
            name: "run".into(),
            clip_index: 0,
            looped: false,
            speed: 1.5,
        }];
        let m = AnimationStateMachine::new("run", clips, states, vec![]);
        assert_eq!(m.current_clip_index(), Some(0));
        assert!((m.current_clip_duration() - 2.5).abs() < 1e-6);
        assert!((m.current_speed() - 1.5).abs() < 1e-6);
        assert!(!m.is_current_looped());
    }

    #[test]
    fn metadata_falls_back_when_state_is_unknown() {
        // `machine` sets current_state = "run" but supplies no states/clips, so every
        // accessor must return its documented default rather than panic/index.
        let m = machine(vec![]);
        assert_eq!(m.current_clip_index(), None);
        assert_eq!(m.current_clip_duration(), 1.0, "missing clip → 1.0");
        assert_eq!(m.current_speed(), 1.0, "missing state → speed 1.0");
        assert!(m.is_current_looped(), "missing state → looped true");
    }
}
