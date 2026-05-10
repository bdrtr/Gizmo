use crate::state::StudioState;
use gizmo::editor::EditorState;
use gizmo::prelude::*;

pub fn update_studio(world: &mut World, state: &mut StudioState, dt: f32, input: &Input) {
    state.current_fps = 1.0 / dt;
    state.actual_dt = dt;

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
        crate::systems::build::handle_build_requests(editor_state);
        crate::systems::shortcuts::handle_editor_shortcuts(world, editor_state, state, input);
        crate::systems::simulation::handle_simulation(world, editor_state, state, dt, input);
        crate::systems::scene_ops::handle_scene_operations(world, editor_state, state);
    });

    // Resolve all Transform hierarchy

    // Kamera sistemine editor state'e geri dönmüş haliyle pas atıyoruz (scroll delta Optional'ı unwrap_or(0.0) diye verdik, orijinal kodda Optional idi ama sistem f32 bekliyor. Bizim argüman f32, Option verilmiş. Düzeltilecekti). Wait, let's fix it properly.
    if scroll_delta.is_none() {
        scroll_delta = Some(0.0);
    }
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

/// Dizin kopyalama yardımcı fonksiyonu
pub fn copy_dir_all(
    src: impl AsRef<std::path::Path>,
    dst: impl AsRef<std::path::Path>,
    log: &dyn Fn(&str),
) -> std::io::Result<()> {
    std::fs::create_dir_all(&dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_dir() {
            copy_dir_all(entry.path(), dst.as_ref().join(entry.file_name()), log)?;
        } else {
            std::fs::copy(entry.path(), dst.as_ref().join(entry.file_name()))?;
        }
    }
    Ok(())
}
