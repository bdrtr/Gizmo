#![warn(missing_docs)]
//! (`missing_docs` is a RATCHET, not a suggestion. The CI lint gate runs with `-D warnings`,
//! so every public item in this crate must carry a doc comment or the build fails. This crate
//! is Stage A — the dependency-light core that goes to 1.x first — and its documented surface
//! is part of that promise. Do not silence this with `#[allow]`; write the doc.)

//! Keyframe animation for the Gizmo engine.
//!
//! The crate hosts **two intentionally separate animation subsystems** — they
//! animate different things and are kept apart so their same-named types do not
//! collide (see [`skeletal`]) — but they share one keyframe-sampling core so
//! their interpolation can never silently diverge:
//!
//! 1. **Transform-track animation** ([`clip`] / [`player`] / [`system`]) — a
//!    small ECS system that animates named entity `Transform`s:
//!    - [`clip`]: the data model. An [`clip::AnimationClip`] is a collection of
//!      [`clip::Track`]s, each targeting an entity by name and holding keyframe
//!      timestamps plus per-channel [`clip::Keyframes`] (translation/rotation/scale).
//!    - [`player`]: the [`player::AnimationPlayer`] component tracking playback
//!      state, and the [`player::Animated`] marker component.
//!    - [`system`]: the [`system::animation_system`] that advances players,
//!      resolves each track's target name to an entity in the player's
//!      hierarchy, and writes sampled values to the targeted transforms.
//!
//! 2. **Skeletal (GPU-skinning) animation** ([`skeletal`]) — the pure-data bone
//!    model the renderer drives to produce per-bone TRS and skinning matrices.
//!
//! Both sample keyframes through the same cubic-Hermite core (`crate::hermite`),
//! so a fix in one interpolation path reaches the other. [`ik`] adds analytic /
//! FABRIK inverse kinematics usable by either.
//!
//! Register the transform-track components + system with [`register`] (the
//! dependency-light entry point) on a [`World`]/[`Schedule`] directly.

/// Transform-track data model: [`clip::AnimationClip`], the [`clip::Track`]s it is
/// made of, and the per-channel [`clip::Keyframes`] / [`clip::InterpolatedValue`]
/// they sample into.
///
/// Tracks here are validated up front — [`clip::Track::new`] returns
/// [`clip::TrackError`] for mismatched, unsorted or non-finite timestamps. The fields
/// stay public afterwards, so [`clip::Track::sample`] bounds-checks every keyframe
/// access: a track edited back into an inconsistent state generally samples to
/// [`clip::InterpolatedValue::None`], and a malformed cubic track degrades to a linear
/// blend.
///
/// That is not a blanket no-panic guarantee. `sample` clamps its segment index with
/// `idx.clamp(1, len - 1)`, and `Ord::clamp` itself panics when `min > max` — so a track
/// whose public fields were edited down to a SINGLE keyframe panics if either that
/// keyframe's timestamp or the sampled time is `NaN` (the NaN skips both early-return
/// guards, leaving `0.clamp(1, 0)`). Use [`clip::Track::new`], which rejects non-finite
/// timestamps, rather than writing the fields directly. Note that a clip's length is the
/// method [`clip::AnimationClip::duration`], recomputed from the longest track on
/// every call, unlike the skeletal clip's stored `duration` field.
pub mod clip;
/// Shared cubic-Hermite sampling core used by both the transform-track and
/// skeletal animation subsystems (crate-internal).
mod hermite;
pub mod ik;
/// Playback state for transform-track animation: the [`player::AnimationPlayer`]
/// component and the [`player::Animated`] marker that [`system::animation_system`]
/// stamps onto every transform it has claimed.
///
/// The player owns the playhead, the loop/speed flags and a name→entity resolution
/// cache; it samples nothing itself. [`player::AnimationPlayer::play`] clears that
/// cache, because a new clip's target names have to be resolved against the hierarchy
/// again.
pub mod player;
pub mod skeletal;
/// The ECS half of transform-track animation.
///
/// [`system::animation_system`] advances every *playing* player, then — once per
/// clip, cached — walks that entity's subtree (the player's own entity included) to
/// map each track's `target_name` onto a named entity, and writes the sampled TRS
/// values to those entities' `Transform`s. It is scheduled
/// `before("transform_propagate")` by [`register`] so the animated locals reach global
/// space in the same frame.
///
/// Failure is quiet by design: a track naming nothing in the hierarchy resolves to
/// no entity and animates nothing, rather than erroring.
pub mod system;

use gizmo_core::system::{IntoSystemConfig, Schedule};
use gizmo_core::world::World;

/// Registers the animation components ([`player::AnimationPlayer`],
/// [`player::Animated`], [`ik::TwoBoneIkChain`]) and schedules
/// [`system::animation_system`] to run before transform propagation, on a
/// [`World`]/[`Schedule`] directly.
///
/// This is the **dependency-light** entry point — it needs only `gizmo-core`, so
/// it works without `gizmo-app`.
pub fn register(world: &mut World, schedule: &mut Schedule) {
    world.register_component_type::<player::AnimationPlayer>();
    world.register_component_type::<player::Animated>();
    world.register_component_type::<ik::TwoBoneIkChain>();
    schedule.add_di_system(
        system::animation_system
            .into_config()
            .label("animation_update")
            // Animated local transforms must be updated before they are propagated to global space.
            .before("transform_propagate"),
    );
}
