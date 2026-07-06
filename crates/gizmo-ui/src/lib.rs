//! A small flexbox/grid UI layer for the Gizmo engine.
//!
//! `gizmo-ui` builds on [`taffy`] for layout and integrates with the Gizmo ECS.
//! UI elements are entities carrying components such as [`Style`] (layout),
//! [`Node`] (computed geometry), [`BackgroundColor`] and [`Interaction`].
//! Spawn them via the [`NodeBundle`] and [`ButtonBundle`] bundles.
//!
//! Add [`UiPlugin`] to an `App` to register the components and run the layout
//! and interaction systems each frame. Common types are re-exported from the
//! [`prelude`] module.
pub mod components;
pub mod layout;
pub mod system;
pub mod interaction;
pub mod bundles;

use gizmo_core::system::{IntoSystemConfig, Schedule};
use gizmo_core::world::World;
pub use components::*;
pub use bundles::*;
pub use layout::*;

/// Registers the UI components + [`UiContext`] resource and schedules the layout
/// and interaction systems on a [`World`]/[`Schedule`] directly.
///
/// This is the **dependency-light** entry point — it needs only `gizmo-core`, so
/// `gizmo-ui` works as a pure ECS-UI layer without `gizmo-app`. The `app`-feature
/// [`UiPlugin`] is a thin wrapper over this.
pub fn register(world: &mut World, schedule: &mut Schedule) {
    world.register_component_type::<Style>();
    world.register_component_type::<Node>();
    world.register_component_type::<Interaction>();
    world.register_component_type::<BackgroundColor>();
    world.register_component_type::<UiRoot>();

    world.insert_resource(UiContext::new());
    // Ensure a WindowInfo exists so `ui_layout_system`'s `Res<WindowInfo>` always
    // resolves (a missing resource would skip the whole system). Under gizmo-app the
    // resize handler keeps this up to date; standalone users can set it directly.
    let _ = world.get_resource_mut_or_default::<gizmo_core::window::WindowInfo>();

    schedule.add_di_system(
        system::ui_layout_system
            .into_config()
            .label("ui_layout"),
    );
    schedule.add_di_system(
        interaction::ui_interaction_system
            .into_config()
            .label("ui_interaction")
            .after("ui_layout"),
    );
}

/// Plugin that registers the UI components and schedules the layout and
/// interaction systems (via [`register`]). Requires the `app` feature.
#[cfg(feature = "app")]
pub struct UiPlugin;

#[cfg(feature = "app")]
impl<State: 'static> gizmo_app::Plugin<State> for UiPlugin {
    fn build(&self, app: &mut gizmo_app::App<State>) {
        register(&mut app.world, &mut app.schedule);
    }
}

/// Re-exports of the most commonly used UI types, including the relevant
/// `taffy` style and geometry items.
pub mod prelude {
    pub use crate::{
        components::{Style, Node, Interaction, BackgroundColor, UiRoot},
        bundles::{NodeBundle, ButtonBundle},
    };
    #[cfg(feature = "app")]
    pub use crate::UiPlugin;
    pub use taffy::style::*;
    pub use taffy::geometry::*;
}
