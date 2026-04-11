use gizmo::prelude::*;
use gizmo::editor::EditorState;

pub mod state;
pub mod setup;
pub mod update;
pub mod render;
pub mod render_pipeline;
pub mod studio_input;

pub use studio_input::*;
pub use state::{StudioState, DebugAssets};

fn main() {
    let mut app = App::<StudioState>::new("Gizmo Studio", 1600, 900)
        .with_icon(include_bytes!("../../../media/logo.png"));

    app = app.set_setup(|world, renderer| {
        setup::setup_studio_scene(world, renderer)
    });

    app = app.set_update(|world, state, dt, input| {
        update::update_studio(world, state, dt, input);
    });

    app = app.set_ui(|world, _state, ctx| {
        // Draw the editor filling the screen
        if let Some(mut editor_state) = world.get_resource_mut::<EditorState>() {
            egui::CentralPanel::default().show(ctx, |_ui| {
                gizmo::editor::draw_editor(ctx, world, &mut editor_state);
            });
        }
    });

    app = app.set_render(|world, state, encoder, view, renderer, light_time| {
        render::render_studio(world, state, encoder, view, renderer, light_time);
    });

    app.run();
}
