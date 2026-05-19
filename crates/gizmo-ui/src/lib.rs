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

pub mod prelude {
    pub use crate::{
        components::{Style, Node, Interaction, BackgroundColor, UiRoot},
        bundles::{NodeBundle, ButtonBundle},
        UiPlugin,
    };
    pub use taffy::style::*;
    pub use taffy::geometry::*;
}
