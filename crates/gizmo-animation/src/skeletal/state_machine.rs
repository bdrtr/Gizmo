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
//! - The engine's frame driver runs it (in the game's and the editor's draw paths alike), after
//!   the [`super::AnimationPlayer`] driver; an entity carrying both is a caller error.
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
use gizmo_math::Vec2;
use std::sync::Arc;

/// A single state in the animation state machine — names one clip, or blends several by a
/// parameter ([`blend`](Self::blend)).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
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

    /// Several clips blended by one of the machine's parameters
    /// ([`AnimationStateMachine::set_parameter`]) in place of [`clip_index`](Self::clip_index): a
    /// walk and a run by how fast the character goes, eight directions of a strafe by where it
    /// heads. See [`BlendSpace`]. `None`: the state plays its one clip.
    #[serde(default)]
    pub blend: Option<BlendSpace>,
}

impl AnimationState {
    /// A state that plays clip `clip_index`, looping, at its authored rate.
    #[must_use]
    pub fn clip(name: &str, clip_index: usize) -> Self {
        Self { name: name.into(), clip_index, looped: true, speed: 1.0, blend: None }
    }

    /// A state that blends clips by a parameter ([`BlendSpace`]), looping, at their authored rate.
    #[must_use]
    pub fn blended(name: &str, blend: BlendSpace) -> Self {
        Self { name: name.into(), clip_index: 0, looped: true, speed: 1.0, blend: Some(blend) }
    }

    /// The same state, holding its last frame instead of looping.
    #[must_use]
    pub fn once(mut self) -> Self {
        self.looped = false;
        self
    }

    /// The same state at `speed` times its authored rate.
    #[must_use]
    pub fn at_speed(mut self, speed: f32) -> Self {
        self.speed = speed;
        self
    }
}

/// **Clips placed at points of a parameter's space, blended by where the parameter is** — Godot's
/// `BlendSpace1D`/`BlendSpace2D`, Unity's blend trees.
///
/// Each [`BlendPoint`] puts a clip at a value of the parameter: a walk at 1.5 m/s and a run at
/// 4.6 m/s on a speed, or a forward, a back and two sideways steps on a direction. At any value the
/// clips nearest it are weighted by how close it is — between two points along a line, linearly;
/// past the last, all the last — with **gradient band** interpolation, which needs no triangulation
/// and takes points anywhere (one dimension is a space whose points all have `y = 0`).
///
/// **The clips keep step.** A blended state's playhead is a phase, 0 to 1 through the cycle, and
/// every clip is sampled at the same phase of its own length: a 1 s walk and a 0.7 s run at a half
/// blend put both feet down together, where sampling both at the same second would have one foot of
/// each down at once. The phase advances at the weighted average of the clips' lengths, so a
/// half-and-half cycle lasts halfway between the two.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BlendSpace {
    /// The machine parameter that places the blend ([`AnimationStateMachine::set_parameter`]).
    pub parameter: String,
    /// The clips and where each sits.
    pub points: Vec<BlendPoint>,
}

/// A clip at a point of a [`BlendSpace`].
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BlendPoint {
    /// Index into [`AnimationStateMachine::clips`].
    pub clip_index: usize,
    /// Where it sits: `[x, 0]` in one dimension.
    pub at: [f32; 2],
}

impl BlendSpace {
    /// A one-dimensional space: each clip at a value of `parameter`.
    #[must_use]
    pub fn one_d(parameter: &str, points: &[(usize, f32)]) -> Self {
        Self {
            parameter: parameter.into(),
            points: points.iter().map(|&(clip_index, x)| BlendPoint { clip_index, at: [x, 0.0] }).collect(),
        }
    }

    /// A two-dimensional space: each clip at a point of `parameter`.
    #[must_use]
    pub fn two_d(parameter: &str, points: &[(usize, [f32; 2])]) -> Self {
        Self { parameter: parameter.into(), points: points.iter().map(|&(clip_index, at)| BlendPoint { clip_index, at }).collect() }
    }

    /// Each point's weight at `at`, in the order of [`points`](Self::points), summing to 1 (empty
    /// for a space with no point). Gradient band: a point's weight is the least, over every other
    /// point, of how far `at` still is from crossing over to it — at a point, that point alone;
    /// along the line between two, linear; beyond the outermost, the outermost.
    #[must_use]
    pub fn weights(&self, at: [f32; 2]) -> Vec<f32> {
        let n = self.points.len();
        if n <= 1 {
            return vec![1.0; n];
        }
        let p = Vec2::from(at);
        let mut weights: Vec<f32> = (0..n)
            .map(|i| {
                let pi = Vec2::from(self.points[i].at);
                let mut least = f32::INFINITY;
                for (j, other) in self.points.iter().enumerate() {
                    let d = Vec2::from(other.at) - pi;
                    let length_squared = d.length_squared();
                    if j == i || length_squared <= 1.0e-12 {
                        continue;
                    }
                    least = least.min(1.0 - (p - pi).dot(d) / length_squared);
                }
                if least.is_finite() { least.max(0.0) } else { 1.0 }
            })
            .collect();
        let total: f32 = weights.iter().sum();
        if total > 1.0e-6 {
            for w in &mut weights {
                *w /= total;
            }
        } else {
            // Every point at the same place: the first speaks for them.
            weights.iter_mut().enumerate().for_each(|(i, w)| *w = if i == 0 { 1.0 } else { 0.0 });
        }
        weights
    }
}

/// A directed transition between two named states.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
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
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ActiveBlend {
    /// The outgoing state, sampled at the frozen [`from_time`](Self::from_time) — a clip or a
    /// blend space alike ([`AnimationStateMachine::state_clips`]). An unknown state contributes
    /// nothing, and the incoming one is drawn alone.
    pub from_state: String,

    /// Time into the source state at the moment the blend started (its phase, for a blended one).
    ///
    /// **Never advanced** while the blend runs: the outgoing clip is a still frame that the
    /// incoming one fades over. That is why there is no `from_speed` counterpart to
    /// [`to_speed`](Self::to_speed).
    pub from_time: f32,
    /// Time into the destination state (advances each frame; its phase, for a blended one).
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
///         AnimationState::clip("idle", 0),
///         AnimationState::clip("run", 1).at_speed(1.2),
///         AnimationState::clip("jump", 2).once(),
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
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct AnimationStateMachine {
    /// The clip pool every [`AnimationState::clip_index`] indexes into.
    ///
    /// Shared behind an `Arc` because keyframe data is large and one rig's clips are normally
    /// used by every entity playing that rig: cloning this component (which the ECS does)
    /// copies a refcount, not the animation data. An empty pool makes the driver skip the
    /// entity entirely — the playhead does not even advance.
    ///
    /// Not saved with a scene: the clips are the model's data, given back by what made the
    /// machine's model — for a glTF model, `gizmo_renderer::gltf_model::resolve_gltf_models`, from
    /// the file (with the scene's clip edits, if it has some), as for an `AnimationPlayer`.
    /// Everything else here — the graph, the state, the playhead, a blend in flight, triggers not
    /// yet read — is saved.
    #[serde(skip)]
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
    #[serde(default)]
    pending_triggers: Vec<String>,
    /// The values the blend spaces read, by name ([`set_parameter`](Self::set_parameter)); one not
    /// set reads `[0, 0]`.
    #[serde(default)]
    parameters: Vec<(String, [f32; 2])>,
    /// How often its skeleton is posed: by how big it is drawn, by default
    /// ([`AnimationUpdate`](super::AnimationUpdate)). Transitions and the playhead advance whatever it says.
    #[serde(default)]
    pub update: super::AnimationUpdate,
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
            parameters: Vec::new(),
            update: super::AnimationUpdate::default(),
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
        std::mem::take(&mut self.pending_triggers)
    }

    /// Sets a one-dimensional parameter the blend spaces read — a speed, a lean.
    pub fn set_parameter(&mut self, name: &str, value: f32) {
        self.set_parameter_2d(name, [value, 0.0]);
    }

    /// Sets a two-dimensional parameter the blend spaces read — a direction of travel.
    pub fn set_parameter_2d(&mut self, name: &str, value: [f32; 2]) {
        match self.parameters.iter_mut().find(|(n, _)| n == name) {
            Some((_, v)) => *v = value,
            None => self.parameters.push((name.to_string(), value)),
        }
    }

    /// A parameter's value; `[0, 0]` for one never set.
    #[must_use]
    pub fn parameter(&self, name: &str) -> [f32; 2] {
        self.parameters.iter().find(|(n, _)| n == name).map_or([0.0; 2], |(_, v)| *v)
    }

    /// The clips `state` plays at `time` — `(clip index, time into that clip, weight)`, the weights
    /// summing to 1: its one clip at `time`, or each clip of its blend space at the phase `time`
    /// of its own length, weighted by where the space's parameter is now. Points whose clip is
    /// missing, and weights of nothing, are left out.
    #[must_use]
    pub fn state_clips(&self, state: &AnimationState, time: f32) -> Vec<(usize, f32, f32)> {
        match &state.blend {
            None => match self.clips.get(state.clip_index) {
                Some(_) => vec![(state.clip_index, time, 1.0)],
                None => Vec::new(),
            },
            Some(space) => space
                .points
                .iter()
                .zip(space.weights(self.parameter(&space.parameter)))
                .filter(|(_, w)| *w > 1.0e-4)
                .filter_map(|(point, w)| self.clips.get(point.clip_index).map(|clip| (point.clip_index, time * clip.duration, w)))
                .collect(),
        }
    }

    /// How long `state`'s playhead runs before it wraps or stops: its clip's length (1.0 s when the
    /// clip is missing), or 1 — a whole cycle of phase — for a blended state.
    #[must_use]
    pub fn state_duration(&self, state: &AnimationState) -> f32 {
        match &state.blend {
            Some(_) => 1.0,
            None => self.clips.get(state.clip_index).map_or(1.0, |c| c.duration),
        }
    }

    /// How fast `state`'s playhead moves per second of `dt` at playback rate `speed`: `speed` for a
    /// clip; for a blended state, `speed` over the weighted average of its clips' lengths, so the
    /// phase goes round once in the time the blended cycle takes.
    #[must_use]
    pub fn state_rate(&self, state: &AnimationState, speed: f32) -> f32 {
        match &state.blend {
            None => speed,
            Some(_) => {
                let cycle: f32 = self
                    .state_clips(state, 0.0)
                    .iter()
                    .map(|&(clip, _, w)| self.clips.get(clip).map_or(0.0, |c| c.duration) * w)
                    .sum();
                if cycle > 1.0e-6 { speed / cycle } else { 0.0 }
            }
        }
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
    /// Reports the *outgoing* clip for the whole of a cross-fade; the incoming state is
    /// [`ActiveBlend::to_state`]. For a blended state, the clip that weighs most where its
    /// parameter is now.
    pub fn current_clip_index(&self) -> Option<usize> {
        let state = self.find_state(&self.current_state)?;
        match &state.blend {
            None => Some(state.clip_index),
            // The clip weighing most where the parameter is now.
            Some(_) => self
                .state_clips(state, self.current_time)
                .into_iter()
                .max_by(|a, b| a.2.total_cmp(&b.2))
                .map(|(clip, _, _)| clip),
        }
    }

    /// Length of the current state's clip, in seconds.
    ///
    /// Falls back to **1.0 s** when the state or the clip is missing. That the fallback is
    /// non-zero is load-bearing: the driver detects clip completion with
    /// `current_time >= duration` and normalizes the playhead against the same value, so a
    /// `0.0` here would report the clip finished on every single frame.
    ///
    /// This is the clip's authored length and is unaffected by [`AnimationState::speed`] — at
    /// speed `2.0` a clip reporting 2.0 s here is done after 1 s of wall clock. For a blended state
    /// it is 1: its playhead is a phase ([`BlendSpace`]).
    pub fn current_clip_duration(&self) -> f32 {
        self.find_state(&self.current_state).map_or(1.0, |s| self.state_duration(s))
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
            from_state: "idle".into(),
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
        let states = vec![AnimationState::clip("run", 0).once().at_speed(1.5)];
        let m = AnimationStateMachine::new("run", clips, states, vec![]);
        assert_eq!(m.current_clip_index(), Some(0));
        assert!((m.current_clip_duration() - 2.5).abs() < 1e-6);
        assert!((m.current_speed() - 1.5).abs() < 1e-6);
        assert!(!m.is_current_looped());
    }

    fn clip(name: &str, duration: f32) -> AnimationClip {
        AnimationClip { name: name.into(), duration, translations: vec![], rotations: vec![], scales: vec![] }
    }

    /// Along a line: a point's clip alone at the point, linear between two, the outermost beyond
    /// the ends; the weights always sum to one.
    #[test]
    fn a_one_dimensional_space_blends_linearly_and_holds_past_its_ends() {
        let space = BlendSpace::one_d("speed", &[(0, 1.5), (1, 4.6)]);
        let w = |x: f32| space.weights([x, 0.0]);
        assert_eq!(w(1.5), vec![1.0, 0.0]);
        assert_eq!(w(4.6), vec![0.0, 1.0]);
        let half = w(3.05);
        assert!((half[0] - 0.5).abs() < 1e-5 && (half[1] - 0.5).abs() < 1e-5, "{half:?}");
        assert_eq!(w(0.0), vec![1.0, 0.0], "below the first point, the first");
        assert_eq!(w(9.0), vec![0.0, 1.0], "past the last, the last");
        // Three along the line: the middle one only between its neighbours.
        let three = BlendSpace::one_d("speed", &[(0, 0.0), (1, 1.0), (2, 2.0)]);
        let at = three.weights([0.5, 0.0]);
        assert!((at[0] - 0.5).abs() < 1e-5 && (at[1] - 0.5).abs() < 1e-5 && at[2].abs() < 1e-6, "{at:?}");
    }

    /// In a plane: a point alone at itself, the weights summing to one anywhere, and the centre of
    /// four points around it shared between them evenly.
    #[test]
    fn a_two_dimensional_space_weighs_the_points_around_the_parameter() {
        let space = BlendSpace::two_d("heading", &[(0, [0.0, 1.0]), (1, [1.0, 0.0]), (2, [0.0, -1.0]), (3, [-1.0, 0.0])]);
        assert_eq!(space.weights([1.0, 0.0]), vec![0.0, 1.0, 0.0, 0.0]);
        for at in [[0.3, 0.4], [-0.9, 0.2], [2.0, 2.0], [0.0, 0.0]] {
            let sum: f32 = space.weights(at).iter().sum();
            assert!((sum - 1.0).abs() < 1e-5, "weights at {at:?} sum to {sum}");
        }
        let centre = space.weights([0.0, 0.0]);
        assert!(centre.iter().all(|w| (w - 0.25).abs() < 1e-5), "{centre:?}");
        // Between forward and right, those two.
        let between = space.weights([0.5, 0.5]);
        assert!(between[0] > 0.3 && between[1] > 0.3 && between[2] < 1e-5 && between[3] < 1e-5, "{between:?}");
    }

    /// **The clips keep step.** A 1 s walk and a 0.5 s run half and half: both are sampled at the
    /// same phase of their own length, and the phase goes round in the 0.75 s the blended cycle
    /// takes — so after 0.375 s both are half-way through.
    #[test]
    fn a_blended_state_samples_every_clip_at_one_phase() {
        let clips: Arc<[AnimationClip]> = Arc::from(vec![clip("walk", 1.0), clip("run", 0.5)]);
        let state = AnimationState::blended("move", BlendSpace::one_d("speed", &[(0, 1.0), (1, 3.0)]));
        let mut m = AnimationStateMachine::new("move", clips, vec![state.clone()], vec![]);
        m.set_parameter("speed", 2.0);
        let rate = m.state_rate(&state, 1.0);
        assert!((rate - 1.0 / 0.75).abs() < 1e-5, "the phase goes round in 0.75 s: {rate}");
        let at = m.state_clips(&state, 0.375 * rate);
        assert_eq!(at.len(), 2);
        assert!((at[0].1 - 0.5).abs() < 1e-5, "the walk half-way: {:?}", at[0]);
        assert!((at[1].1 - 0.25).abs() < 1e-5, "the run half-way: {:?}", at[1]);
        assert!((at[0].2 - 0.5).abs() < 1e-5 && (at[1].2 - 0.5).abs() < 1e-5);
        assert_eq!(m.current_clip_duration(), 1.0, "a blended state's playhead is a phase");
        // At the run's end of the space the run alone, and the walk weighs nothing.
        m.set_parameter("speed", 3.0);
        assert_eq!(m.state_clips(&state, 0.0).iter().map(|c| c.0).collect::<Vec<_>>(), vec![1]);
        assert_eq!(m.current_clip_index(), Some(1));
    }

    #[test]
    fn a_parameter_never_set_reads_zero() {
        let mut m = machine(vec![]);
        assert_eq!(m.parameter("speed"), [0.0, 0.0]);
        m.set_parameter("speed", 2.0);
        m.set_parameter_2d("heading", [0.5, -1.0]);
        m.set_parameter("speed", 3.0);
        assert_eq!((m.parameter("speed"), m.parameter("heading")), ([3.0, 0.0], [0.5, -1.0]));
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
