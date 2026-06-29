//! GPU-skinning skeletal-animation **data model**, moved out of `gizmo-renderer`
//! so it carries no `wgpu` dependency and can be shared by scripting/editor crates
//! without pulling in the renderer.
//!
//! These types are the pure-data counterpart to the renderer's GPU `Skeleton`
//! component and GPU update systems (which stay in `gizmo-renderer` and import the
//! moved types from here). They are intentionally reachable **only** via the
//! `skeletal::` path — the crate root must not glob-re-export them, to avoid
//! ambiguity with the existing transform-track [`crate::clip`]/[`crate::player`]
//! animation types of the same name.

pub mod clip;
pub mod component;
pub mod keyframe;
pub mod sample;
pub mod skeleton;
pub mod state_machine;

pub use clip::AnimationClip;
pub use component::{AnimationPlayer, BoneAttachment};
pub use keyframe::{InterpolationMode, Keyframe, Track};
pub use sample::{blend_poses, decompose_mat4, evaluate_clip};
pub use skeleton::{SkeletonHierarchy, SkeletonJoint};
pub use state_machine::{
    ActiveBlend, AnimationState, AnimationStateMachine, AnimationTransition,
};
