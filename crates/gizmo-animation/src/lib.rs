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

pub mod clip;
/// Shared cubic-Hermite sampling core used by both the transform-track and
/// skeletal animation subsystems (crate-internal).
mod hermite;
pub mod ik;
pub mod player;
pub mod skeletal;
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
