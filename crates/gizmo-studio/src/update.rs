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

        // Garbage Collection & Auto-Save (her frame kontrol, belirli aralıklarla çalışır)
        crate::systems::gc::garbage_collection_system(world, state, editor_state, dt);
    });

    // İşletim sistemleri (Async Asset Server ve Transform senkronizasyonları)

    // Resolve all Transform hierarchy
    let mut transform_sync = gizmo::systems::transform::TransformSyncSystem;
    let mut transform_propagate = gizmo::systems::transform::TransformPropagateSystem;
    gizmo::core::system::System::run(&mut transform_sync, world, dt);
    gizmo::core::system::System::run(&mut transform_propagate, world, dt);

    // OBB Highlight for Selected Entities
    if let Some(mut gizmos) = world.get_resource_mut::<gizmo::renderer::Gizmos>() {
        let meshes = world.borrow::<gizmo::renderer::components::Mesh>();
        let global_transforms = world.borrow::<gizmo::physics::components::GlobalTransform>();
        let children_comp = world.borrow::<gizmo::core::component::Children>();
        
        let editor_state = world.get_resource::<gizmo::editor::EditorState>().unwrap();
        let selected_entities = editor_state.selection.entities.iter().copied().collect::<Vec<gizmo::core::entity::Entity>>();
        let color = [1.0, 0.5, 0.0, 1.0]; // Blender Orange

        // BFS to collect all descendants
        let mut to_draw = selected_entities.clone();
        let mut i = 0;
        while i < to_draw.len() {
            let ent = to_draw[i];
            if let Some(children) = children_comp.get(ent.id()) {
                for &child_id in &children.0 {
                    // Generation is not strictly checked here for highlight rendering
                    to_draw.push(gizmo::core::entity::Entity::new(child_id, 0));
                }
            }
            i += 1;
        }

        for entity in to_draw {
            if let (Some(mesh), Some(gt)) = (meshes.get(entity.id()), global_transforms.get(entity.id())) {
                let min = mesh.bounds.min;
                let max = mesh.bounds.max;
                let c = [
                    gizmo::math::Vec3::new(min.x, min.y, min.z),
                    gizmo::math::Vec3::new(max.x, min.y, min.z),
                    gizmo::math::Vec3::new(max.x, max.y, min.z),
                    gizmo::math::Vec3::new(min.x, max.y, min.z),
                    gizmo::math::Vec3::new(min.x, min.y, max.z),
                    gizmo::math::Vec3::new(max.x, min.y, max.z),
                    gizmo::math::Vec3::new(max.x, max.y, max.z),
                    gizmo::math::Vec3::new(min.x, max.y, max.z),
                ];
                
                // Transform to global space
                let mut tc = [gizmo::math::Vec3::ZERO; 8];
                for j in 0..8 {
                    let v4 = gt.matrix * gizmo::math::Vec4::new(c[j].x, c[j].y, c[j].z, 1.0);
                    tc[j] = gizmo::math::Vec3::new(v4.x, v4.y, v4.z);
                }

                // Draw 12 lines
                // Bottom face
                gizmos.draw_line(tc[0], tc[1], color);
                gizmos.draw_line(tc[1], tc[2], color);
                gizmos.draw_line(tc[2], tc[3], color);
                gizmos.draw_line(tc[3], tc[0], color);
                // Top face
                gizmos.draw_line(tc[4], tc[5], color);
                gizmos.draw_line(tc[5], tc[6], color);
                gizmos.draw_line(tc[6], tc[7], color);
                gizmos.draw_line(tc[7], tc[4], color);
                // Connecting edges
                gizmos.draw_line(tc[0], tc[4], color);
                gizmos.draw_line(tc[1], tc[5], color);
                gizmos.draw_line(tc[2], tc[6], color);
                gizmos.draw_line(tc[3], tc[7], color);
            }
        }
    }

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
