
use crate::editor_state::EditorState;
use egui;
use gizmo_core::World;
use gizmo_physics::components::Transform;


pub fn draw_transform_section(
    ui: &mut egui::Ui,
    world: &World,
    entity_id: gizmo_core::entity::Entity,
    state: &mut EditorState,
) {
    let mut transforms = world.borrow_mut::<Transform>();
    let old_t = transforms.get(entity_id.id()).copied();
    
    // Pointer status check for Drag operations
    let pointer_down = ui.input(|i| i.pointer.any_down());
    let mut changed = false;

    if let Some(t) = transforms.get_mut(entity_id.id()) {
        let _is_interacting_with_transform = egui::CollapsingHeader::new("🚀 Transform")
            .default_open(true)
            .show(ui, |ui| {
                ui.label("Pozisyon:");
                ui.horizontal(|ui| {
                    ui.label("X");
                    if ui.add(egui::DragValue::new(&mut t.position.x).speed(0.1)).changed() { changed = true; }
                    ui.label("Y");
                    if ui.add(egui::DragValue::new(&mut t.position.y).speed(0.1)).changed() { changed = true; }
                    ui.label("Z");
                    if ui.add(egui::DragValue::new(&mut t.position.z).speed(0.1)).changed() { changed = true; }
                });

                ui.label("Rotasyon (Euler°):");
                let (mut rx, mut ry, mut rz) = quat_to_euler_deg(t.rotation);
                let old_euler = (rx, ry, rz);
                ui.horizontal(|ui| {
                    ui.label("X");
                    if ui.add(egui::DragValue::new(&mut rx).speed(1.0).suffix("°")).changed() { changed = true; }
                    ui.label("Y");
                    if ui.add(egui::DragValue::new(&mut ry).speed(1.0).suffix("°")).changed() { changed = true; }
                    ui.label("Z");
                    if ui.add(egui::DragValue::new(&mut rz).speed(1.0).suffix("°")).changed() { changed = true; }
                });
                if (rx, ry, rz) != old_euler {
                    t.rotation = euler_deg_to_quat(rx, ry, rz);
                    changed = true;
                }

                ui.label("Ölçek:");
                ui.horizontal(|ui| {
                    ui.label("X");
                    if ui.add(egui::DragValue::new(&mut t.scale.x).speed(0.05).range(0.01..=100.0)).changed() { changed = true; }
                    ui.label("Y");
                    if ui.add(egui::DragValue::new(&mut t.scale.y).speed(0.05).range(0.01..=100.0)).changed() { changed = true; }
                    ui.label("Z");
                    if ui.add(egui::DragValue::new(&mut t.scale.z).speed(0.05).range(0.01..=100.0)).changed() { changed = true; }
                });
            });

        if changed {
            t.update_local_matrix();
        }
        
        // --- UNDO/REDO TRACKING (INSPECTOR) ---
        if pointer_down && changed && state.inspector_drag_original_transforms.is_empty() {
            // Sürükleme (drag) işlemi başladı, asıl transformları yedekle
            for &ent in state.selection.entities.iter() {
                if let Some(&original_t) = transforms.get(ent.id()) {
                    state.inspector_drag_original_transforms.insert(ent, original_t);
                }
            }
        } else if !pointer_down && !state.inspector_drag_original_transforms.is_empty() {
            // Fare bırakıldı, değişiklikleri kaydet
            let mut undo_changes = Vec::new();
            for (ent, old_transform) in state.inspector_drag_original_transforms.drain() {
                if let Some(&new_transform) = transforms.get(ent.id()) {
                    if old_transform != new_transform {
                        undo_changes.push((ent, old_transform, new_transform));
                    }
                }
            }
            if !undo_changes.is_empty() {
                let count = undo_changes.len();
                state.history.push(crate::history::EditorAction::TransformsChanged { changes: undo_changes });
                state.status_message = format!("💾 {} obje Inspector'dan değiştirildi (Undo: Ctrl+Z)", count);
            }
        }
        // Klavye ile tekil girişler (text box edit) için:
        // Eğer pointer down değilse ama değer changed ise, direkt geçmişe kaydet
        else if !pointer_down && changed && state.inspector_drag_original_transforms.is_empty() {
            if let Some(old_transform) = old_t {
                let undo_changes = vec![(entity_id, old_transform, *t)];
                
                // Çoklu seçim varsa diğerlerinin de son halini kaydetmeliyiz.
                // Not: Delta hesaplaması hemen aşağıda olduğu için, onların yeni t'sini bir sonraki kareye bırakamayız.
                // Bu yüzden keyboard input için basit tutuyoruz.
                state.history.push(crate::history::EditorAction::TransformsChanged { changes: undo_changes });
                state.status_message = "💾 Obje Inspector'dan değiştirildi (Undo: Ctrl+Z)".to_string();
            }
        }
        
        ui.separator();
    }

    // Çoklu seçim (Multi-Object Editing) Delta Uygulaması
    if changed && state.selection.entities.len() > 1 {
        if let Some(old) = old_t {
            if let Some(new_t) = transforms.get(entity_id.id()).copied() {
                let delta_pos = new_t.position - old.position;
                let delta_rot = new_t.rotation * old.rotation.inverse();
                let delta_scale = gizmo_math::Vec3::new(
                    if old.scale.x != 0.0 { new_t.scale.x / old.scale.x } else { 1.0 },
                    if old.scale.y != 0.0 { new_t.scale.y / old.scale.y } else { 1.0 },
                    if old.scale.z != 0.0 { new_t.scale.z / old.scale.z } else { 1.0 },
                );

                let others: Vec<_> = state.selection.entities.iter().copied().filter(|&e| e != entity_id).collect();
                for e in others {
                    if let Some(other_t) = transforms.get_mut(e.id()) {
                        other_t.position += delta_pos;
                        other_t.rotation = delta_rot * other_t.rotation;
                        other_t.scale *= delta_scale;
                        other_t.update_local_matrix();
                    }
                }
            }
        }
    }
}


pub fn quat_to_euler_deg(q: gizmo_math::Quat) -> (f32, f32, f32) {
    let (x, y, z) = q.to_euler(gizmo_math::EulerRot::XYZ);
    (x.to_degrees(), y.to_degrees(), z.to_degrees())
}


pub fn euler_deg_to_quat(rx: f32, ry: f32, rz: f32) -> gizmo_math::Quat {
    gizmo_math::Quat::from_euler(
        gizmo_math::EulerRot::XYZ,
        rx.to_radians(),
        ry.to_radians(),
        rz.to_radians(),
    )
}


