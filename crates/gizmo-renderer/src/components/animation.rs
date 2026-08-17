use std::sync::Arc;

// Back-compat re-exports: the pure-data skeletal-animation types now live in
// `gizmo_animation::skeletal`. Re-export them here so existing consumers of
// `gizmo_renderer::components::{AnimationPlayer, ...}` keep resolving unchanged.
pub use gizmo_animation::skeletal::{
    ActiveBlend, AnimationClip, AnimationPlayer, AnimationState, AnimationStateMachine,
    AnimationTransition, BoneAttachment, SkeletonHierarchy,
};

/// A skinned mesh's skeleton: the joint hierarchy, the current pose, and the GPU buffer the
/// vertex shader reads its skinning matrices from.
///
/// Both poses are kept because they are different things: animation writes **local** poses, one
/// per joint relative to its parent, and the skinning pass needs **global** ones, which are the
/// local poses composed down the hierarchy. Storing only locals would mean re-walking the tree per
/// vertex; storing only globals would make it impossible to blend two animations.
#[derive(Clone)]
pub struct Skeleton {
    /// The bind group holding [`Self::buffer`].
    pub bind_group: Arc<wgpu::BindGroup>,
    /// The GPU buffer of skinning matrices, uploaded from [`Self::global_poses`] each frame the
    /// pose changes.
    pub buffer: Arc<wgpu::Buffer>,
    /// The joint hierarchy — parent links and inverse bind matrices. Shared, because every entity
    /// using one skinned mesh has the same skeleton.
    pub hierarchy: Arc<gizmo_animation::skeletal::SkeletonHierarchy>,
    /// The current pose: one matrix per joint, relative to its parent. This is what animation
    /// playback writes.
    pub local_poses: Vec<gizmo_math::Mat4>,
    /// The same pose composed down the hierarchy, in model space. This is what reaches the GPU.
    pub global_poses: Vec<gizmo_math::Mat4>,
}

impl Skeleton {
    /// A skeleton in its initial pose, with the global poses seeded from the local ones.
    ///
    /// # Panics
    ///
    /// If `local_poses` has a different length than the hierarchy's joint list — a pose array
    /// that does not match its skeleton indexes out of bounds or skins to the wrong joint, and
    /// both are far harder to trace from the resulting picture than from here.
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
