use super::clip::AnimationClip;
use super::state_machine::AnimationStateMachine;
use std::sync::Arc;

#[derive(Clone)]
pub struct AnimationPlayer {
    pub current_time: f32,
    pub active_animation: usize,
    pub loop_anim: bool,
    pub speed: f32,
    pub animations: Arc<[AnimationClip]>,
    // Blending support
    pub blend_time: f32,
    pub blend_duration: f32,
    pub prev_animation: Option<usize>,
    pub prev_time: f32,
}

impl Default for AnimationPlayer {
    fn default() -> Self {
        Self {
            current_time: 0.0,
            active_animation: 0,
            loop_anim: true,
            speed: 1.0,
            animations: Arc::new([]),
            blend_time: 0.0,
            blend_duration: 0.0,
            prev_animation: None,
            prev_time: 0.0,
        }
    }
}

impl AnimationPlayer {
    pub fn current_clip(&self) -> Option<&AnimationClip> {
        self.animations.get(self.active_animation)
    }

    pub fn play_animation_by_name(&mut self, name: &str, blend: f32, loop_anim: bool) -> bool {
        if let Some(idx) = self.animations.iter().position(|a| a.name == name) {
            if self.active_animation != idx {
                // Gameplay-driven clip switch (arms a cross-fade). Frequent enough
                // in normal play that it belongs at debug!, not info!.
                tracing::debug!(
                    from = self.active_animation,
                    to = idx,
                    name = %name,
                    blend,
                    loop_anim,
                    "[Animation] skeletal clip switch (cross-fade armed)"
                );
                self.prev_animation = Some(self.active_animation);
                self.prev_time = self.current_time;
                self.active_animation = idx;
                self.current_time = 0.0;
                self.blend_duration = blend;
                self.blend_time = 0.0;
                self.loop_anim = loop_anim;
            } else {
                tracing::trace!(name = %name, "[Animation] skeletal clip already active; play is a no-op");
            }
            true
        } else {
            // Previously a silent `false`: a caller that ignores the return value
            // (a typo'd clip name) would just see nothing animate. Surface it.
            tracing::warn!(
                requested = %name,
                available = self.animations.len(),
                "[Animation] play_animation_by_name: no clip with that name; request ignored"
            );
            false
        }
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct BoneAttachment {
    pub target_entity: gizmo_core::entity::Entity,
    pub bone_index: usize,
    pub offset: gizmo_math::Mat4,
}

impl Default for BoneAttachment {
    fn default() -> Self {
        Self {
            target_entity: gizmo_core::entity::Entity::new(0, 0),
            bone_index: 0,
            offset: gizmo_math::Mat4::IDENTITY,
        }
    }
}

gizmo_core::impl_component!(AnimationPlayer, AnimationStateMachine, BoneAttachment);

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(name: &str) -> AnimationClip {
        AnimationClip {
            name: name.into(),
            duration: 1.0,
            translations: vec![],
            rotations: vec![],
            scales: vec![],
        }
    }

    fn player_with(names: &[&str]) -> AnimationPlayer {
        let anims: Vec<AnimationClip> = names.iter().map(|n| clip(n)).collect();
        AnimationPlayer {
            animations: Arc::from(anims),
            ..Default::default()
        }
    }

    #[test]
    fn play_by_name_switches_and_arms_the_crossfade() {
        let mut p = player_with(&["idle", "run"]);
        p.current_time = 5.0; // pretend we were mid-idle
        let ok = p.play_animation_by_name("run", 0.3, false);
        assert!(ok, "known animation must report success");
        assert_eq!(p.active_animation, 1, "active switches to run");
        assert_eq!(p.prev_animation, Some(0), "previous animation captured for cross-fade");
        assert_eq!(p.prev_time, 5.0, "previous playhead captured");
        assert_eq!(p.current_time, 0.0, "new animation restarts from 0");
        assert_eq!(p.blend_duration, 0.3);
        assert_eq!(p.blend_time, 0.0);
        assert!(!p.loop_anim, "loop flag taken from the call");
    }

    #[test]
    fn play_by_name_same_animation_is_noop_but_succeeds() {
        let mut p = player_with(&["idle", "run"]);
        p.current_time = 5.0;
        let ok = p.play_animation_by_name("idle", 0.5, true);
        assert!(ok, "re-selecting the active clip still reports success");
        assert_eq!(p.active_animation, 0, "no switch");
        assert_eq!(p.prev_animation, None, "no cross-fade armed");
        assert_eq!(p.current_time, 5.0, "playhead must NOT be reset when already playing it");
        assert_eq!(p.blend_duration, 0.0, "no blend armed for a no-op");
    }

    #[test]
    fn play_by_name_unknown_returns_false_and_changes_nothing() {
        let mut p = player_with(&["idle", "run"]);
        p.current_time = 2.0;
        let ok = p.play_animation_by_name("fly", 0.1, false);
        assert!(!ok, "unknown animation must fail");
        assert_eq!(p.active_animation, 0, "state untouched on failure");
        assert_eq!(p.prev_animation, None);
        assert_eq!(p.current_time, 2.0);
    }

    #[test]
    fn current_clip_indexes_active_and_guards_bounds() {
        let mut p = player_with(&["idle", "run"]);
        p.active_animation = 1;
        assert_eq!(p.current_clip().map(|c| c.name.as_str()), Some("run"));
        // An out-of-range active index must yield None, not panic.
        p.active_animation = 99;
        assert!(p.current_clip().is_none());
    }
}
