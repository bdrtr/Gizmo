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
//! dependency-light entry point), or — with the `app` feature — add
//! [`AnimationPlugin`] to a [`gizmo_app::App`].

pub mod clip;
pub mod player;
pub mod system;

use gizmo_core::system::{IntoSystemConfig, Schedule};
use gizmo_core::world::World;

/// Registers the animation components ([`player::AnimationPlayer`],
/// [`player::Animated`]) and schedules [`system::animation_system`] to run before
/// transform propagation, on a [`World`]/[`Schedule`] directly.
///
/// This is the **dependency-light** entry point — it needs only `gizmo-core`, so
/// it works without `gizmo-app`. The `app`-feature [`AnimationPlugin`] is a thin
/// wrapper over this.
pub fn register(world: &mut World, schedule: &mut Schedule) {
    world.register_component_type::<player::AnimationPlayer>();
    world.register_component_type::<player::Animated>();
    schedule.add_di_system(
        system::animation_system
            .into_config()
            .label("animation_update")
            // Animated local transforms must be updated before they are propagated to global space.
            .before("transform_propagate"),
    );
}

/// [`gizmo_app::Plugin`] that registers the animation components and system (via
/// [`register`]). Requires the `app` feature.
#[cfg(feature = "app")]
pub struct AnimationPlugin;

#[cfg(feature = "app")]
impl<State: 'static> gizmo_app::Plugin<State> for AnimationPlugin {
    fn build(&self, app: &mut gizmo_app::App<State>) {
        register(&mut app.world, &mut app.schedule);
    }
}
