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
//! Add [`AnimationPlugin`] to an [`gizmo_app::App`] to register the components
//! and schedule the system (it runs before transform propagation).

pub mod clip;
pub mod player;
pub mod system;

use gizmo_core::system::IntoSystemConfig;

/// [`gizmo_app::Plugin`] that registers the animation components and schedules
/// [`system::animation_system`] to run before transform propagation.
pub struct AnimationPlugin;

impl<State: 'static> gizmo_app::Plugin<State> for AnimationPlugin {
    fn build(&self, app: &mut gizmo_app::App<State>) {
        app.world.register_component_type::<player::AnimationPlayer>();
        app.world.register_component_type::<player::Animated>();
        app.schedule.add_di_system(
            system::animation_system
                .into_config()
                .label("animation_update")
                // Animated local transforms must be updated before they are propagated to global space.
                .before("transform_propagate"),
        );
    }
}
