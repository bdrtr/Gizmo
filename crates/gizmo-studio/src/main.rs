//! Gizmo Studio's entry point: boots an [`gizmo::App`] window, wires the engine's
//! setup/update/UI/render hooks, and renders the egui-based editor on top of the live scene.
//! Run it with `cargo run -p gizmo-studio`; the `editor`, `scene`, `audio` and `scripting`
//! engine features are enabled by default.
//!
//! Everything below `main` lives in the library target (`lib.rs`) so that the editor's render
//! path is reachable from a test — see that file for why an application has one.

use gizmo::editor::EditorState;
use gizmo::prelude::*;
use gizmo_studio::{render, setup, update, StudioState};

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() {
    gizmo::core::logger::init_tracing();
    let mut app = App::<StudioState>::new("Gizmo Studio", 1600, 900)
        .with_icon(include_bytes!("../media/logo.png"))
        .add_event::<gizmo_studio::state::ShaderReloadEvent>()
        .add_plugin(gizmo::asset_server::AssetServerPlugin);

    app = app.set_setup(setup::setup_studio_scene);

    app = app.set_update(|world, state, dt, input| {
        gizmo::systems::physics::cpu_physics_step_system(world, dt);
        gizmo::ai::system::ai_navmesh_rebuild_system(world, dt);
        gizmo::ai::system::ai_navigation_system(world, dt);
        update::update_studio(world, state, dt, input);
    });

    app = app.set_ui(|world, _state, ctx| {
        // Draw the editor filling the screen
        if let Some(mut editor_state) = world.get_resource_mut::<EditorState>() {
            // egui 0.34 root-`Ui` composition (replaces the deprecated top-level
            // `CentralPanel::show(ctx)`): build a full-viewport background `Ui` and
            // let the editor compose its panels into it via `show_inside`.
            let mut root = gizmo::egui::Ui::new(
                ctx.clone(),
                gizmo::egui::Id::new("gizmo_editor_root"),
                gizmo::egui::UiBuilder::new()
                    .layer_id(gizmo::egui::LayerId::background())
                    .max_rect(ctx.content_rect()),
            );
            root.set_clip_rect(ctx.content_rect());
            gizmo::editor::draw_editor(&mut root, world, &mut editor_state);
        }
    });

    app = app.set_render(|world, state, encoder, view, renderer, light_time| {
        render::render_studio(world, state, encoder, view, renderer, light_time);
    });

    app.run().expect("uygulama çalıştırılamadı");
}
