use std::sync::Arc;

pub use crate::animation_state_machine::{
    ActiveBlend, AnimationState, AnimationStateMachine, AnimationTransition,
};

#[derive(Clone)]
pub struct Skeleton {
    pub bind_group: Arc<wgpu::BindGroup>,
    pub buffer: Arc<wgpu::Buffer>,
    pub hierarchy: Arc<crate::animation::SkeletonHierarchy>,
    pub local_poses: Vec<gizmo_math::Mat4>,
}

impl Skeleton {
    pub fn new(
        bind_group: Arc<wgpu::BindGroup>,
        buffer: Arc<wgpu::Buffer>,
        hierarchy: Arc<crate::animation::SkeletonHierarchy>,
        local_poses: Vec<gizmo_math::Mat4>,
    ) -> Self {
        assert_eq!(
            hierarchy.joints.len(),
            local_poses.len(),
            "Skeleton joints uzunlugu ile local_poses esit olmali"
        );
        Self {
            bind_group,
            buffer,
            hierarchy,
            local_poses,
        }
    }
}

#[derive(Clone)]
pub struct AnimationPlayer {
    pub current_time: f32,
    pub active_animation: usize,
    pub loop_anim: bool,
    pub speed: f32,
    pub animations: Arc<[crate::animation::AnimationClip]>,
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
    pub fn current_clip(&self) -> Option<&crate::animation::AnimationClip> {
        self.animations.get(self.active_animation)
    }
}
