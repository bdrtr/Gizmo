//! Gizmo Studio as a library call: [`run`] opens the editor, with a game hooked in or without one.
//!
//! The `gizmo-studio` binary is `run(None)`: the editor on its own, a scene played by the engine's
//! loop. A game written in Rust opens the same editor on itself with `run(Some(game))` — its
//! components in the inspector, its setup and its frame under ▶ ([`crate::StudioGame`], [`crate::pie`]) —
//! from a binary of its own, since an application cannot see the types of a crate it was not built
//! with.

use gizmo::editor::EditorState;
use gizmo::prelude::*;
use crate::{render, setup, update, StudioGame, StudioState};

/// The window icon, embedded from the workspace's canonical logo.
///
/// Pulled out of the call so a test can decode the same bytes the window gets: the icon path is
/// two nested `if let Ok(..)`, so a logo that has been moved, renamed or re-encoded into something
/// the `image` crate cannot read costs you the icon and says nothing at all.
pub const LOGO: &[u8] = include_bytes!("../../../media/logo.png");

/// Opens Gizmo Studio, on `game` when there is one (see the [module](self)), and runs it until the
/// window closes.
///
/// # Errors
///
/// What the windowed app's `run` reports: no window, no GPU.
pub fn run(game: Option<StudioGame>) -> Result<(), gizmo::app::AppError> {
    let title = game.as_ref().map_or_else(|| "Gizmo Studio".to_string(), |g| format!("Gizmo Studio — {}", g.name));
    let mut game = game;
    let extend = game.as_mut().and_then(|g| g.app.take());
    let game = game.map(std::sync::Arc::new);
    let mut app = App::<StudioState>::new(&title, 1600, 900)
        // The workspace's one logo, not a copy of it. `gizmo-studio` is `publish = false`
        // (see publish_all.sh), so reaching outside the crate directory is safe here — a
        // published crate could not, because `cargo package` only ships files under its own root.
        .with_icon(LOGO)
        .add_plugin(gizmo::asset_server::AssetServerPlugin);
    if let Some(extend) = extend {
        app = extend(app);
    }

    app = app.set_setup(move |world, renderer| {
        let mut state = setup::setup_studio_scene(world, renderer);
        if let Some(game) = &game {
            (game.register)(world);
            state.game = Some(game.clone());
        }
        state
    });

    app = app.set_update(|world, state, dt, input| {
        // Not unconditional: while ▶ is down `PlayLoop` owns the frame and stepping here as well
        // ran the simulation at ~2x wall-clock (and left ⏸ painting a pause overlay over falling
        // bodies). See `systems::simulation::editor_owns_the_physics_step`.
        //
        // AI rides the same gate for the same reason, now that `PlayLoop::step` runs `ai_frame`
        // itself — and that call is what gives an exported game navigation at all: until then
        // these two lines were the ONLY place in the workspace where an agent was ever steered.
        if crate::systems::simulation::editor_owns_the_physics_step(world) {
            gizmo::systems::physics::cpu_physics_step_system(world, dt);
            gizmo::ai::system::ai_frame(world, dt);
        }
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

    app.run()
}

// Two things the studio decides before any of its frame runs: the window's icon, and who owns the
// frame while ▶ is down.
#[cfg(test)]
mod main_tests {
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

    /// **The editor's own physics step must stay gated.**
    ///
    /// It was not, and the two consequences were invisible from inside the editor: with ▶ down the
    /// world was stepped twice per rendered frame — once here with the frame delta, once by
    /// `PlayLoop`'s 60 Hz accumulator — so the editor ran the simulation at roughly twice the rate
    /// an exported game does, which is exactly the drift `PlayLoop` was extracted to make
    /// impossible. And ⏸ stopped only `PlayLoop`, so the pause overlay was painted over bodies
    /// that were still falling.
    ///
    /// A source-shape guard because the offending line lives in `main`'s update closure, which no
    /// test can drive. Comments are cut first: a negative `contains` is satisfied — wrongly — by
    /// prose that merely names the call, and the paragraph above does exactly that.
    #[test]
    fn the_editors_physics_step_is_gated_on_the_play_session() {
        fn code_only(src: &str) -> String {
            src.lines()
                .map(|line| {
                    let bytes = line.as_bytes();
                    let mut end = line.len();
                    let mut i = 0;
                    while i + 1 < bytes.len() {
                        if bytes[i] == b'/'
                            && bytes[i + 1] == b'/'
                            && (i == 0 || bytes[i - 1] != b':')
                        {
                            end = i;
                            break;
                        }
                        i += 1;
                    }
                    &line[..end]
                })
                .collect::<Vec<_>>()
                .join("\n")
        }

        let code = code_only(include_str!("app.rs").split("#[cfg(test)]").next().unwrap_or(""));
        let call = code
            .find("cpu_physics_step_system(")
            .expect("the editor still steps physics in edit mode; if that changed, so must this");
        let gate = code
            .find("editor_owns_the_physics_step(")
            .expect("the editor's physics step must be gated on whether it owns the frame");
        assert!(
            gate < call,
            "the gate must come BEFORE the step — an ungated call runs the simulation twice \
             under ▶ and keeps it running under ⏸"
        );

        // Same gate, same reason, for AI: `PlayLoop::step` runs `ai_frame` too, so an ungated
        // call here steers every agent twice per frame while ▶ is down.
        let ai = code
            .find("ai_frame(")
            .expect("the editor still runs AI in edit mode; if that changed, so must this");
        assert!(gate < ai, "the editor's AI frame must be gated on the play session too");
        let close = code[gate..]
            .find("\n        }")
            .map(|o| gate + o)
            .expect("the gated block must still be a block");
        assert!(
            ai < close,
            "the AI call drifted out of the gated block — it reads as gated and is not"
        );
    }
}
