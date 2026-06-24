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

use gizmo_app::{App, Plugin};
use gizmo_core::system::IntoSystemConfig;
pub use components::*;
pub use bundles::*;
pub use layout::*;

/// Plugin that registers the UI components and schedules the layout and
/// interaction systems.
pub struct UiPlugin;

impl<State: 'static> Plugin<State> for UiPlugin {
    fn build(&self, app: &mut App<State>) {
        app.world.register_component_type::<Style>();
        app.world.register_component_type::<Node>();
        app.world.register_component_type::<Interaction>();
        app.world.register_component_type::<BackgroundColor>();
        app.world.register_component_type::<UiRoot>();

        app.world.insert_resource(UiContext::new());

        app.schedule.add_di_system(
            system::ui_layout_system
                .into_config()
                .label("ui_layout"),
        );
        app.schedule.add_di_system(
            interaction::ui_interaction_system
                .into_config()
                .label("ui_interaction")
                .after("ui_layout"),
        );
    }
}

/// Re-exports of the most commonly used UI types, including the relevant
/// `taffy` style and geometry items.
pub mod prelude {
    pub use crate::{
        components::{Style, Node, Interaction, BackgroundColor, UiRoot},
        bundles::{NodeBundle, ButtonBundle},
        UiPlugin,
    };
    pub use taffy::style::*;
    pub use taffy::geometry::*;
}
