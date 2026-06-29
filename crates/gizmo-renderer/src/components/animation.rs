use std::sync::Arc;

// Back-compat re-exports: the pure-data skeletal-animation types now live in
// `gizmo_animation::skeletal`. Re-export them here so existing consumers of
// `gizmo_renderer::components::{AnimationPlayer, ...}` keep resolving unchanged.
pub use gizmo_animation::skeletal::{
    ActiveBlend, AnimationClip, AnimationPlayer, AnimationState, AnimationStateMachine,
    AnimationTransition, BoneAttachment, SkeletonHierarchy,
};

#[derive(Clone)]
pub struct Skeleton {
    pub bind_group: Arc<wgpu::BindGroup>,
    pub buffer: Arc<wgpu::Buffer>,
    pub hierarchy: Arc<gizmo_animation::skeletal::SkeletonHierarchy>,
    pub local_poses: Vec<gizmo_math::Mat4>,
    pub global_poses: Vec<gizmo_math::Mat4>,
}

impl Skeleton {
    pub fn new(
        bind_group: Arc<wgpu::BindGroup>,
        buffer: Arc<wgpu::Buffer>,
        hierarchy: Arc<gizmo_animation::skeletal::SkeletonHierarchy>,
        local_poses: Vec<gizmo_math::Mat4>,
    ) -> Self {
        assert_eq!(
            hierarchy.joints.len(),
            local_poses.len(),
            "Skeleton joints uzunlugu ile local_poses esit olmali"
        );
        let global_poses = local_poses.clone(); // Initial
        Self {
            bind_group,
            buffer,
            hierarchy,
            local_poses,
            global_poses,
        }
    }
}
