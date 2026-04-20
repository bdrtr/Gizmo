use crate::state::StudioState;
use gizmo::editor::EditorState;
use gizmo::physics::components::Transform;
use gizmo::prelude::*;

pub fn handle_editor_shortcuts(world: &mut World, editor_state: &mut EditorState, state: &StudioState, input: &Input) {
        // --- EDITOR KISAYOLLARI (SHORTCUTS) ---
        let ctrl_pressed = input.is_key_pressed(gizmo::winit::keyboard::KeyCode::ControlLeft as u32)
            || input.is_key_pressed(gizmo::winit::keyboard::KeyCode::ControlRight as u32);
        let shift_pressed = input.is_key_pressed(gizmo::winit::keyboard::KeyCode::ShiftLeft as u32)
            || input.is_key_pressed(gizmo::winit::keyboard::KeyCode::ShiftRight as u32);

        // Kısayol: Undo / Redo
        if ctrl_pressed {
            if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::KeyZ as u32) {
                if shift_pressed {
                    editor_state.history.redo(world);
                } else {
                    editor_state.history.undo(world);
                }
            } else if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::KeyY as u32) {
                editor_state.history.redo(world);
            }

            // Kısayol: Ctrl + D (Çoğalt)
            if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::KeyD as u32) {
                for &entity in editor_state.selected_entities.iter() {
                    editor_state.duplicate_requests.push(entity);
                }
            }
        }

        // Kısayol: Delete (Sil) (Ctrl durumundan bağımsız tetiklenmeli)
        if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::Delete as u32) {
            for &entity in editor_state.selected_entities.iter() {
                editor_state.despawn_requests.push(entity);
            }
            editor_state.clear_selection();
        }

        // Kısayol: F (Seçili Objeye Odaklan) (Yine Ctrl'den bağımsız tetiklenmeli)
        if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::KeyF as u32) {
            if !editor_state.selected_entities.is_empty() {
                    if let Some(transforms) = world.borrow::<Transform>().expect("ECS Aliasing Error") {
                        let mut center_pos = gizmo::math::Vec3::ZERO;
                        let mut count = 0.0;
                        for &target_id in editor_state.selected_entities.iter() {
                            if let Some(target) = transforms.get(target_id) {
                                center_pos += target.position;
                                count += 1.0;
                            }
                        }

                        if count > 0.0 {
                            let target_pos = center_pos / count;
                            drop(transforms); // Ödünç almayı bırak

                            if let (Some(mut t_mut), Some(mut cam_mut)) = (
                                world.borrow_mut::<Transform>().expect("ECS Aliasing Error"),
                                world.borrow_mut::<gizmo::renderer::components::Camera>().expect("ECS Aliasing Error"),
                            ) {
                                if let (Some(cam_t), Some(cam)) = (
                                    t_mut.get_mut(state.editor_camera),
                                    cam_mut.get_mut(state.editor_camera),
                                ) {
                                    // Hedef odak mesafesi dinamik. Şimdilik 10.0 varsayılan.
                                    editor_state.prefs.camera_focus_distance = 10.0;
                                    let offset = cam.get_front() * -editor_state.prefs.camera_focus_distance;
                                    cam_t.position = target_pos + offset;
                                    cam_t.update_local_matrix();
                                }
                            }
                        }
                }
            }
        }
}
