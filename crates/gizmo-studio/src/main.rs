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

/// The window icon, embedded from the workspace's canonical logo.
///
/// Pulled out of the call so a test can decode the same bytes the window gets: the icon path is
/// two nested `if let Ok(..)`, so a logo that has been moved, renamed or re-encoded into something
/// the `image` crate cannot read costs you the icon and says nothing at all.
const LOGO: &[u8] = include_bytes!("../../../media/logo.png");

fn main() {
    gizmo::core::logger::init_tracing();
    let mut app = App::<StudioState>::new("Gizmo Studio", 1600, 900)
        // The workspace's one logo, not a copy of it. `gizmo-studio` is `publish = false`
        // (see publish_all.sh), so reaching outside the crate directory is safe here — a
        // published crate could not, because `cargo package` only ships files under its own root.
        .with_icon(LOGO)
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

#[cfg(test)]
mod icon_tests {
    /// The embedded logo must be something `winit` can actually take as a window icon.
    ///
    /// The runtime path is `if let Ok(image) = image::load_from_memory(..)` inside
    /// `if let Ok(icon) = Icon::from_rgba(..)` — two silent failures. A logo re-encoded into a
    /// format the `image` crate does not read, or one whose dimensions `winit` rejects, costs you
    /// the icon and logs nothing. This walks the same two steps on the same bytes.
    #[test]
    fn the_embedded_logo_decodes_into_a_window_icon() {
        let image = image::load_from_memory(super::LOGO).expect("the logo must decode");
        let rgba = image.into_rgba8();
        let (w, h) = rgba.dimensions();
        assert_eq!(w, h, "a window icon is square; {w}x{h} is not");
        assert!(w >= 64, "an icon smaller than 64 px has nothing left after scaling");
        // Transparent corners, not a white card: the icon sits on whatever the OS puts behind it.
        assert_eq!(rgba.get_pixel(0, 0)[3], 0, "the logo must keep its transparent background");
        gizmo::winit::window::Icon::from_rgba(rgba.into_raw(), w, h)
            .expect("winit must accept it");
    }
}
