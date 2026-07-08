//! Name-based keyframe animation for the Gizmo engine.
//!
//! This crate provides a small ECS animation system built around three pieces:
//!
//! - [`clip`]: the data model. An [`clip::AnimationClip`] is a collection of
//!   [`clip::Track`]s, each targeting an entity by name and holding keyframe
//!   timestamps plus per-channel [`clip::Keyframes`] (translation, rotation, scale).
//! - [`player`]: the [`player::AnimationPlayer`] component that tracks playback
//!   state for a clip, and the [`player::Animated`] marker component.
//! - [`system`]: the [`system::animation_system`] that advances players, resolves
//!   each track's target name to an entity within the player's hierarchy, and
//!   writes sampled values to the targeted transforms.
//!
//! Register the animation components + system with [`register`] (the
//! dependency-light entry point) on a [`World`]/[`Schedule`] directly.

pub mod clip;
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
