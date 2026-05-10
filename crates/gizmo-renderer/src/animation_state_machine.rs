use crate::animation::AnimationClip;
use std::sync::Arc;

/// A single state in the animation state machine — names one clip.
#[derive(Clone, Debug)]
pub struct AnimationState {
    pub name: String,
    pub clip_index: usize,
    pub looped: bool,
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
    pub from_clip: usize,
    pub to_clip: usize,
    /// Time into the source clip at the moment the blend started.
    pub from_time: f32,
    /// Time into the destination clip (advances each frame).
    pub to_time: f32,
    /// Seconds elapsed since blend began.
    pub elapsed: f32,
    pub duration: f32,
    pub to_state: String,
    pub to_looped: bool,
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
/// ```ignore
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
/// ```
#[derive(Clone)]
pub struct AnimationStateMachine {
    pub clips: Arc<[AnimationClip]>,
    pub states: Vec<AnimationState>,
    pub transitions: Vec<AnimationTransition>,
    pub current_state: String,
    pub current_time: f32,
    pub active_blend: Option<ActiveBlend>,
    pending_triggers: Vec<String>,
}

impl AnimationStateMachine {
    pub fn new(
        initial_state: &str,
        clips: Arc<[AnimationClip]>,
        states: Vec<AnimationState>,
        transitions: Vec<AnimationTransition>,
    ) -> Self {
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

    /// Queue a trigger to be evaluated on the next `animation_state_machine_update`.
    pub fn trigger(&mut self, name: &str) {
        self.pending_triggers.push(name.to_string());
    }

    /// Drain and return all pending triggers (consumed by the update system).
    pub fn drain_triggers(&mut self) -> Vec<String> {
        self.pending_triggers.drain(..).collect()
    }

    // ── helpers ──────────────────────────────────────────────────────────────

    pub fn find_state(&self, name: &str) -> Option<&AnimationState> {
        self.states.iter().find(|s| s.name == name)
    }

    pub fn current_clip_index(&self) -> Option<usize> {
        self.find_state(&self.current_state).map(|s| s.clip_index)
    }

    pub fn current_clip_duration(&self) -> f32 {
        self.current_clip_index()
            .and_then(|i| self.clips.get(i))
            .map(|c| c.duration)
            .unwrap_or(1.0)
    }

    pub fn current_speed(&self) -> f32 {
        self.find_state(&self.current_state)
            .map(|s| s.speed)
            .unwrap_or(1.0)
    }

    pub fn is_current_looped(&self) -> bool {
        self.find_state(&self.current_state)
            .map(|s| s.looped)
            .unwrap_or(true)
    }

    /// Find the first matching transition (trigger or exit-time based).
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
            // Auto / exit-time transition
            if tr.has_exit_time {
                clip_finished
            } else {
                false
            }
        })
    }
}
