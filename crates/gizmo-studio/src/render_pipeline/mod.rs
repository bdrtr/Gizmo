//! The studio's two panels, both drawn through the game path.
//!
//! The Scene panel is the editor camera's view (the game camera's in play mode) with the editor's
//! furniture, toolbar shading and lines; the Game panel is the game camera's own picture, HUD
//! included. Both are `gizmo::systems::render::draw_camera_into`, so neither can draw what the game
//! does not: until 2026-11-22 they were the studio's own forward renderer — its own batching, cull
//! loop, shadow, main and transparent passes, some 1 800 lines — which drifted from the game's the
//! way a copy does, and `tests/render_parity.rs` existed to catch the drift. Routed through the
//! game path as an experiment, 27 of the 30 pixel tests passed as they were; the other three asked
//! for what an editor's view adds (`ViewOptions`), which the game path now has.

use crate::StudioState;
use gizmo::prelude::*;
use gizmo::systems::render::{ViewOptions, ViewShading};

mod viewpoint;

/// The toolbar's shading chips, as `EditorState::shading_mode` numbers them (`gizmo_editor::toolbar`).
fn toolbar_shading(mode: u32) -> ViewShading {
    match mode {
        1 => ViewShading::Normals,
        2 => ViewShading::Albedo,
        3 => ViewShading::Wire,
        _ => ViewShading::Lit,
    }
}

/// Draws the studio's panels for one frame: the Scene panel into `EditorRenderTarget` (into `view`
/// when there is none) and, when it is on screen, the Game panel into `GameRenderTarget`.
pub fn execute_render_pipeline(
    world: &mut World,
    state: &StudioState,
    encoder: &mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    renderer: &mut gizmo::renderer::Renderer,
    _light_time: f32,
) {
    // **The frame's preparation is the game's own.** `prepare_frame` is everything a frame writes
    // that the game can see — terrain and cloth meshes, the frame clocks (tweens, typewriters,
    // camera shakes, the screen fade), transforms, materials both ways, skeletal and sprite
    // animation, bone attachments, the CPU side of GPU physics, particle emitters, the water, the
    // end of the debug-line frame. ⏸ reaches it through the clock (`hold_clock`). What the last
    // draw cost goes to the world first, as the engine's render stage does.
    gizmo::systems::render::publish_render_stats(world, renderer);
    gizmo::systems::render::prepare_frame(world, renderer);
    prepare_editor_lines(world, state);
    // From here on the world is only read.
    draw_studio_frame(world, state, encoder, view, renderer);
}

/// **The editor's own lines for this frame** (`EditorLines`), drawn into the Scene panel alone: the
/// game camera's frustum, in green, while editing, and the collider overlay when the toolbar asks
/// for it. They went into the game's `Gizmos` (the overlay) or a copy of its frame (the frustum)
/// while the Scene panel had a renderer of its own; the game path draws the game's lines into every
/// view, the Game panel's included, so the editor's have a resource of their own.
fn prepare_editor_lines(world: &mut World, state: &StudioState) {
    let mut lines = gizmo::renderer::Gizmos::default();
    let show_colliders =
        world.get_resource::<gizmo::editor::EditorState>().is_some_and(|ed| ed.show_colliders);
    if !viewpoint::is_playing(world) {
        // At the Game panel's shape: the frustum outlines what that panel shows.
        let aspect = world
            .get_resource::<gizmo::renderer::components::GameRenderTarget>()
            .map(|t| t.0.width as f32 / t.0.height.max(1) as f32)
            .unwrap_or(16.0 / 9.0);
        if let Some(view_proj) = viewpoint::game_view_proj(world, state.game_camera, aspect) {
            lines.draw_frustum(view_proj, [0.0, 1.0, 0.0, 1.0]);
        }
    }
    if show_colliders {
        gizmo::systems::physics::physics_debug_lines(world, &mut lines);
    }
    world.insert_resource(gizmo::renderer::EditorLines(lines));
}

/// What the editor's toolbar and panels ask of this frame's draw.
struct EditorView {
    shading: ViewShading,
    show_colliders: bool,
    show_grid: bool,
    game_view_visible: bool,
}

/// Mirrors the editor's state into the renderer (FXAA, which is the editor's own toggle) and reads
/// the rest of what the draw needs. Everything about the LOOK is the camera's, read by the game
/// path: grade, exposure, depth of field.
fn sync_editor_settings(world: &World, renderer: &mut gizmo::renderer::Renderer) -> EditorView {
    let ed = world.get_resource::<gizmo::editor::EditorState>();
    // A missing `EditorState` is a headless render test: the grid on, as the app starts.
    let editor = EditorView {
        shading: toolbar_shading(ed.as_ref().map_or(0, |ed| ed.shading_mode)),
        show_colliders: ed.as_ref().is_some_and(|ed| ed.show_colliders),
        show_grid: ed.as_ref().is_none_or(|ed| ed.prefs.show_grid),
        game_view_visible: ed.as_ref().is_some_and(|ed| ed.game_view_visible),
    };
    let fxaa_on = ed.as_ref().is_none_or(|ed| ed.post_process.fxaa_enabled);
    drop(ed);
    if let Some(ref mut fxaa) = renderer.fxaa {
        if fxaa.enabled != fxaa_on {
            fxaa.enabled = fxaa_on;
            fxaa.set_enabled(&renderer.queue, fxaa_on);
        }
    }
    editor
}

/// The editor's draw: the Game panel, then the Scene panel, from a prepared world, which it only
/// reads.
fn draw_studio_frame(
    world: &World,
    state: &StudioState,
    encoder: &mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    renderer: &mut gizmo::renderer::Renderer,
) {
    // **Everything a frame does before its views is the game's own** (`draw_ahead_of_views`): the
    // reflection probes that ask for a capture (drawn without furniture, so the editor's probes are
    // the game's), the GPU simulations stepped once for every view, and every camera that draws
    // into a texture (`CameraTarget`), so a monitor in the scene shows in the editor what it shows
    // in the game. At full detail, where the game scales the simulations by camera distance: an
    // editor shows what is there. The collider overlay's switch goes to the GPU bodies first, so
    // the debug lines the step makes are this frame's.
    let editor = sync_editor_settings(world, renderer);
    if let Some(physics) = &renderer.gpu_physics {
        physics.set_debug_flags(&renderer.queue, u32::from(editor.show_colliders));
    }
    gizmo::systems::render::draw_ahead_of_views(world, renderer, gizmo::systems::render::SimulationDetail::Full);

    // **The Game panel is the game's own picture**: the game camera through the game path's whole
    // pipeline, in play mode and out of it, HUD included. Only for a panel someone can see: in the
    // default layout Scene and Game are tabs of ONE dock leaf, so at most one of them is on screen,
    // and a second scene draw every frame to fill a texture behind another tab cost the studio's
    // frame about 40% (2.63/3.02 ms → 1.65/1.69 ms when the gate went in).
    if editor.game_view_visible {
        draw_game_view(world, renderer, state.game_camera);
    }

    // **The Scene panel**: the editor camera with the furniture, the toolbar's shading and the
    // editor's lines; in play mode the game camera, as the game draws it. Culled by the camera it
    // draws from: until 2026-11-22 it culled against the game camera even while editing, so what the
    // game would not draw vanished from the editor (decided against, by the owner, for the scene
    // view every other editor has).
    let playing = viewpoint::is_playing(world);
    let camera = viewpoint::scene_camera(world, state.editor_camera, state.game_camera);
    let options = if playing {
        ViewOptions::default()
    } else {
        let options = ViewOptions::default().with_furniture().with_shading(editor.shading);
        if editor.show_grid { options } else { options.without_grid() }
    };
    let target = world.get_resource::<gizmo::renderer::components::EditorRenderTarget>().map(|t| t.0.clone());
    match target {
        Some(target) => {
            // The window's own surface only frames the panels: cleared, so a frame never presents
            // whatever the swapchain image held.
            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Clear Swapchain Background Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            gizmo::systems::render::draw_camera_into(world, renderer, camera, &target.view, (target.width, target.height), options);
        }
        None => {
            let size = (renderer.size.width, renderer.size.height);
            gizmo::systems::render::draw_camera_into(world, renderer, camera, view, size, options);
        }
    }
}

/// Draws the game camera into the Game panel's texture through the game path
/// (`draw_camera_into`). Nothing when there is no panel texture or no game camera to draw from.
fn draw_game_view(world: &World, renderer: &mut gizmo::renderer::Renderer, game_camera: Entity) {
    let Some(target) = world.get_resource::<gizmo::renderer::components::GameRenderTarget>().map(|t| t.0.clone()) else {
        return;
    };
    if world.query::<&gizmo::renderer::components::Camera>().get(game_camera).is_none() {
        return;
    }
    gizmo::systems::render::draw_camera_into(
        world,
        renderer,
        game_camera,
        &target.view,
        (target.width, target.height),
        ViewOptions::default().with_hud(),
    );
}
