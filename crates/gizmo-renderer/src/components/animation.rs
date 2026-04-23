use std::sync::Arc;

#[derive(Clone)]
pub struct Skeleton {
    pub bind_group: Arc<wgpu::BindGroup>,
    pub buffer: Arc<wgpu::Buffer>,
    pub hierarchy: Arc<crate::animation::SkeletonHierarchy>,
    pub local_poses: Vec<gizmo_math::Mat4>,
}

#[derive(Clone)]
pub struct AnimationPlayer {
    pub current_time: f32,
    pub active_animation: usize,
    pub loop_anim: bool,
    pub animations: Arc<Vec<crate::animation::AnimationClip>>,
}
