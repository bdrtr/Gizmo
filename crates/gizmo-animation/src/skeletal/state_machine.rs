use super::clip::AnimationClip;
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
}
