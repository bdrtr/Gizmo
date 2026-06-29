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
                self.prev_animation = Some(self.active_animation);
                self.prev_time = self.current_time;
                self.active_animation = idx;
                self.current_time = 0.0;
                self.blend_duration = blend;
                self.blend_time = 0.0;
                self.loop_anim = loop_anim;
            }
            true
        } else {
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
