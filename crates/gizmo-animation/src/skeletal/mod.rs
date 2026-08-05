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

/// ECS components that carry skeletal playback state:
/// [`component::AnimationPlayer`] (one active clip plus a single cross-fade slot) and
/// [`component::BoneAttachment`] (an entity whose `Transform` is overwritten each
/// frame with `skeleton.global_poses[bone_index] * offset`, so props can ride a bone).
///
/// This module is where all three skeletal components — including
/// [`state_machine::AnimationStateMachine`], which is declared elsewhere — get their
/// `gizmo-core` `Component` impls. Nothing in this crate advances them: the systems
/// that do live in `gizmo-renderer` (`animation_update_system`) and the `gizmo`
/// facade (`BoneAttachmentSystem`).
pub mod component;

/// The keyframe primitives every skeletal track is built from:
/// [`keyframe::Keyframe`] (a timestamped value plus the optional glTF cubic
/// in/out tangents), [`keyframe::Track`] (one channel targeting one node) and
/// [`keyframe::InterpolationMode`].
///
/// [`keyframe::Track`] owns only the *time* half of sampling — locating the segment
/// around a query time by binary search — and hands the actual blend to a
/// caller-supplied closure. That is why it is generic over the value type and free of
/// any `Vec3`/`Quat` maths, and why a `CubicSpline` track whose tangents were lost
/// can cleanly decline and let the caller fall back to a linear blend.
pub mod keyframe;

/// Pose evaluation: [`sample::evaluate_clip`] (clip + time + skeleton → one TRS
/// triple per joint, falling back to the joint's bind value per channel),
/// [`sample::blend_poses`] (the cross-fade: lerp translation/scale, slerp rotation)
/// and [`sample::decompose_mat4`].
///
/// This is where clip tracks are matched against skeleton joints, including the loose
/// `mixamorig:` / `/RootNode/` name cleanup used for retargeting, and where the
/// Hips-only root-motion rule is enforced.
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
