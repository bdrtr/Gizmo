use std::sync::Arc;

// Back-compat re-exports: the pure-data skeletal-animation types now live in
// `gizmo_animation::skeletal`. Re-export them here so existing consumers of
// `gizmo_renderer::components::{AnimationPlayer, ...}` keep resolving unchanged.
pub use gizmo_animation::skeletal::{
    ActiveBlend, AnimationClip, AnimationPlayer, AnimationState, AnimationStateMachine,
    AnimationLayer, AnimationLayers, AnimationTransition, AnimationUpdate, BlendPoint, BlendSpace, BoneAttachment, BoneMask,
    LayerBlend, RootMotion, SkeletonHierarchy,
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
    /// Each clip's tracks matched to this skeleton's joints, kept: matching is a search by name
    /// that the animation systems used to repeat for every clip every frame.
    pub(crate) bindings: ClipBindings,
    /// Where an animation driver last left this skeleton's player or machine: a key for its clip or
    /// state, and its playhead. A frame that finds them otherwise — the game switched the clip, the
    /// editor scrubbed a paused playhead, a scene was reopened — poses it whatever its size on screen
    /// says (`crate::animation_lod`); `None` until a driver has posed it.
    pub(crate) driven: Option<(u64, f32)>,
}

/// The [`ClipBinding`](gizmo_animation::skeletal::ClipBinding)s a skeleton has made, by joint
/// hierarchy, clip table and index. Each entry holds the hierarchy and the table it was made for,
/// so neither address it is keyed by can be reused by another while the entry lives — and a
/// skeleton given another hierarchy matches its clips again.
#[derive(Clone, Default)]
pub(crate) struct ClipBindings {
    entries: Vec<BindingEntry>,
}

type BindingEntry = (
    Arc<gizmo_animation::skeletal::SkeletonHierarchy>,
    Arc<[gizmo_animation::skeletal::AnimationClip]>,
    usize,
    Arc<gizmo_animation::skeletal::ClipBinding>,
);

impl ClipBindings {
    /// The binding of clip `index` of `clips` to `hierarchy`, made the first time it is asked for.
    pub(crate) fn get(
        &mut self,
        clips: &Arc<[gizmo_animation::skeletal::AnimationClip]>,
        index: usize,
        hierarchy: &Arc<gizmo_animation::skeletal::SkeletonHierarchy>,
    ) -> Option<Arc<gizmo_animation::skeletal::ClipBinding>> {
        let clip = clips.get(index)?;
        if let Some((.., b)) =
            self.entries.iter().find(|(h, t, i, _)| Arc::ptr_eq(h, hierarchy) && Arc::ptr_eq(t, clips) && *i == index)
        {
            return Some(b.clone());
        }
        // What no one uses any more is let go: a table no player holds, a hierarchy replaced.
        self.entries.retain(|(h, t, ..)| Arc::strong_count(t) > 1 && Arc::ptr_eq(h, hierarchy));
        let binding = Arc::new(gizmo_animation::skeletal::ClipBinding::new(clip, hierarchy));
        self.entries.push((hierarchy.clone(), clips.clone(), index, binding.clone()));
        Some(binding)
    }
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
            bindings: ClipBindings::default(),
            driven: None,
        }
    }
}

#[cfg(test)]
mod binding_tests {
    use super::*;
    use gizmo_animation::skeletal::{AnimationClip, SkeletonHierarchy, SkeletonJoint};

    fn hierarchy() -> Arc<SkeletonHierarchy> {
        Arc::new(SkeletonHierarchy {
            joints: vec![SkeletonJoint {
                name: "Hips".into(),
                node_index: 0,
                inverse_bind_matrix: gizmo_math::Mat4::IDENTITY,
                parent_index: None,
                local_bind_transform: gizmo_math::Mat4::IDENTITY,
                bind_translation: gizmo_math::Vec3::ZERO,
                bind_rotation: gizmo_math::Quat::IDENTITY,
                bind_scale: gizmo_math::Vec3::ONE,
            }],
            root_transform: gizmo_math::Mat4::IDENTITY,
        })
    }

    /// A skeleton keeps each clip's binding and hands the same one back; another clip, another
    /// table or another hierarchy gets its own — a skeleton given a new hierarchy does not reuse the
    /// old one's matching.
    #[test]
    fn bindings_are_kept_per_clip_table_and_hierarchy() {
        let clip = || AnimationClip { name: "a".into(), duration: 1.0, translations: vec![], rotations: vec![], scales: vec![] };
        let clips: Arc<[AnimationClip]> = Arc::from(vec![clip(), clip()].into_boxed_slice());
        let (h, other_h) = (hierarchy(), hierarchy());
        let mut cache = ClipBindings::default();
        let first = cache.get(&clips, 0, &h).expect("clip 0");
        assert!(Arc::ptr_eq(&first, &cache.get(&clips, 0, &h).unwrap()), "kept, not matched again");
        assert!(!Arc::ptr_eq(&first, &cache.get(&clips, 1, &h).unwrap()), "another clip, another binding");
        let again: Arc<[AnimationClip]> = Arc::from(vec![clip(), clip()].into_boxed_slice());
        assert!(!Arc::ptr_eq(&first, &cache.get(&again, 0, &h).unwrap()), "another table, another binding");
        assert!(!Arc::ptr_eq(&first, &cache.get(&clips, 0, &other_h).unwrap()), "another hierarchy, matched again");
        assert!(cache.get(&clips, 7, &h).is_none(), "no such clip");
    }
}
