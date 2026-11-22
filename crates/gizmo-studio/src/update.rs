use crate::state::StudioState;
use gizmo::editor::EditorState;
use gizmo::prelude::*;

/// The studio's per-frame update: input and picking, camera, shortcuts, scene operations, play
/// mode, gizmos and the housekeeping timers, in that order.
pub fn update_studio(world: &mut World, state: &mut StudioState, dt: f32, input: &Input) {
    state.current_fps = 1.0 / dt;
    state.actual_dt = dt;

    // ▶ / ⏹ first, before anything borrows the editor's state out of the world: they swap the
    // world itself (`crate::pie`).
    let (start, stop) = world.get_resource_mut::<EditorState>().map_or((false, false), |mut ed| {
        (std::mem::take(&mut ed.play_start_request), std::mem::take(&mut ed.play_stop_request))
    });
    if stop {
        crate::pie::stop_play(world, state);
        if let Some(mut audio) = world.get_resource_mut::<gizmo::audio::AudioManager>() {
            audio.stop_all();
        }
        if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
            ed.log_info("⏹ Stop: düzenlenen sahneye dönüldü (oyun dünyası atıldı).");
        }
    }
    if start && state.session.is_none() {
        crate::pie::start_play(world, state);
        let count = world.iter_alive_entities().len();
        if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
            ed.log_info(&format!("▶ Play: oyun kendi dünyasında başladı ({count} varlık)."));
        }
    }

    let mut look_delta = None;
    let mut pan_delta = None;
    let mut orbit_delta = None;
    let mut scroll_delta = None;
    world.resource_scope(|world, editor_state: &mut EditorState| {
        look_delta = editor_state.camera.look_delta;
        pan_delta = editor_state.camera.pan_delta;
        orbit_delta = editor_state.camera.orbit_delta;
        scroll_delta = editor_state.camera.scroll_delta;

        let win_info = world
            .get_resource::<WindowInfo>()
            .map(|w| *w)
            .unwrap_or_default();
        crate::systems::input::handle_input_and_scene_view(
            world,
            editor_state,
            state,
            dt,
            input,
            &win_info,
        );
        crate::systems::build::handle_build_requests(world, editor_state);
        crate::systems::shortcuts::handle_editor_shortcuts(world, editor_state, state, input);
        crate::systems::simulation::handle_simulation(world, editor_state, state, dt, input);
        crate::systems::scene_ops::handle_scene_operations(world, editor_state, state);

        // Garbage Collection & Auto-Save (her frame kontrol, belirli aralıklarla çalışır)
        crate::systems::gc::garbage_collection_system(world, state, editor_state, dt);
    });
    // İşletim sistemleri (Async Asset Server ve Transform senkronizasyonları)
    crate::systems::gizmos::update_editor_gizmos(world, state);

    // Resolve all Transform hierarchy
    let mut transform_sync = gizmo::systems::transform::TransformSyncSystem;
    let mut transform_propagate = gizmo::systems::transform::TransformPropagateSystem;
    gizmo::core::system::System::run(&mut transform_sync, world, dt);
    gizmo::core::system::System::run(&mut transform_propagate, world, dt);

    // The selection's boxes, the cameras' and lights' icons and the collider overlay are the
    // editor's own lines, drawn into the Scene panel alone: the render pipeline makes them each frame
    // (`render_pipeline::prepare_editor_lines`). They were written into the game's `Gizmos` here,
    // with its depth test switched off, and the game path draws those into the Game panel too.

    // Kamera sistemine editor state'e geri dönmüş delta'yı gönder
    crate::systems::camera::handle_camera(
        world,
        state,
        dt,
        input,
        look_delta,
        pan_delta,
        orbit_delta,
        scroll_delta.unwrap_or(0.0),
    );
}

/// A helper for copying a directory
pub fn copy_dir_all(
    src: impl AsRef<std::path::Path>,
    dst: impl AsRef<std::path::Path>,
) -> std::io::Result<()> {
    std::fs::create_dir_all(&dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_dir() {
            copy_dir_all(entry.path(), dst.as_ref().join(entry.file_name()))?;
        } else {
            std::fs::copy(entry.path(), dst.as_ref().join(entry.file_name()))?;
        }
    }
    Ok(())
}
