
use crate::editor_state::EditorState;
use egui;
use gizmo_core::World;
use gizmo_physics_core::Transform;


pub fn draw_transform_section(
    ui: &mut egui::Ui,
    world: &World,
    entity_id: gizmo_core::entity::Entity,
    state: &mut EditorState,
) {
    // SAFETY: editor UI runs single-threaded in the egui draw; no concurrent World access.
    let mut transforms = unsafe { world.borrow_mut_unchecked::<Transform>() };
    let old_t = transforms.get_mut(entity_id.id()).map(|t| *t);
    
    // Pointer status check for Drag operations
    let pointer_down = ui.input(|i| i.pointer.any_down());
    let mut changed = false;

    if let Some(mut t) = transforms.get_mut(entity_id.id()) {
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
                if let Some(original_t) = transforms.get_mut(ent.id()).map(|t| *t) {
                    state.inspector_drag_original_transforms.insert(ent, original_t);
                }
            }
        } else if !pointer_down && !state.inspector_drag_original_transforms.is_empty() {
            // Fare bırakıldı, değişiklikleri kaydet
            let mut undo_changes = Vec::new();
            for (ent, old_transform) in state.inspector_drag_original_transforms.drain() {
                if let Some(new_transform) = transforms.get_mut(ent.id()).map(|t| *t) {
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
            if let Some(new_t) = transforms.get_mut(entity_id.id()).map(|t| *t) {
                let delta_pos = new_t.position - old.position;
                let delta_rot = new_t.rotation * old.rotation.inverse();
                let delta_scale = gizmo_math::Vec3::new(
                    if old.scale.x != 0.0 { new_t.scale.x / old.scale.x } else { 1.0 },
                    if old.scale.y != 0.0 { new_t.scale.y / old.scale.y } else { 1.0 },
                    if old.scale.z != 0.0 { new_t.scale.z / old.scale.z } else { 1.0 },
                );

                let others: Vec<_> = state.selection.entities.iter().copied().filter(|&e| e != entity_id).collect();
                for e in others {
                    if let Some(mut other_t) = transforms.get_mut(e.id()) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_math::{Quat, Vec3};

    /// İki quaternion'un AYNI rotasyonu temsil edip etmediğini, üç temel
    /// vektörü döndürüp sonuçları karşılaştırarak doğrular. Bu, `q` ile `-q`nin
    /// aynı rotasyon olması (çift-örtü) ve Euler ayrıştırmasının farklı ama
    /// eşdeğer üçlüler üretebilmesi sorununu doğal olarak aşar.
    fn assert_same_rotation(a: Quat, b: Quat, eps: f32) {
        for v in [Vec3::X, Vec3::Y, Vec3::Z, Vec3::new(1.0, 2.0, 3.0)] {
            let ra = a.mul_vec3(v);
            let rb = b.mul_vec3(v);
            let d = (ra - rb).length();
            assert!(
                d <= eps,
                "rotasyonlar eşdeğer değil: v={:?} a·v={:?} b·v={:?} fark={}",
                v,
                ra,
                rb,
                d
            );
        }
    }

    #[test]
    fn identity_quat_maps_to_zero_euler() {
        let (x, y, z) = quat_to_euler_deg(Quat::IDENTITY);
        assert!(x.abs() < 1e-4, "x={}", x);
        assert!(y.abs() < 1e-4, "y={}", y);
        assert!(z.abs() < 1e-4, "z={}", z);
    }

    #[test]
    fn zero_euler_maps_to_identity_quat() {
        let q = euler_deg_to_quat(0.0, 0.0, 0.0);
        assert_same_rotation(q, Quat::IDENTITY, 1e-5);
    }

    /// Derece → radyan dönüşümü doğru olmalı: 90° X ekseni etrafında dönüş,
    /// glam'in `from_rotation_x(PI/2)`si ile aynı rotasyonu vermeli.
    #[test]
    fn degrees_are_converted_to_radians() {
        let q = euler_deg_to_quat(90.0, 0.0, 0.0);
        assert_same_rotation(q, Quat::from_rotation_x(std::f32::consts::FRAC_PI_2), 1e-5);

        let qy = euler_deg_to_quat(0.0, 45.0, 0.0);
        assert_same_rotation(qy, Quat::from_rotation_y(45.0_f32.to_radians()), 1e-5);
    }

    /// Tek eksen dönüşleri Euler'e temiz geri okunmalı (gimbal-lock yok).
    #[test]
    fn single_axis_round_trips_to_expected_degrees() {
        let (x, y, z) = quat_to_euler_deg(Quat::from_rotation_y(45.0_f32.to_radians()));
        assert!(x.abs() < 1e-3, "x={}", x);
        assert!((y - 45.0).abs() < 1e-3, "y={}", y);
        assert!(z.abs() < 1e-3, "z={}", z);
    }

    /// Asıl invariant: quat → euler → quat, girişle AYNI rotasyonu vermeli.
    /// Gimbal-lock (|pitch|≈90°) dışındaki temsili açı üçlüleri için taranır.
    #[test]
    fn quat_euler_quat_round_trip_preserves_rotation() {
        let samples = [
            (10.0, 20.0, 30.0),
            (-45.0, 15.0, 80.0),
            (120.0, -30.0, 5.0),
            (0.0, 0.0, 179.0),
            (-170.0, 60.0, -60.0),
            (33.0, -33.0, 33.0),
        ];
        for (rx, ry, rz) in samples {
            let q = euler_deg_to_quat(rx, ry, rz);
            let (bx, by, bz) = quat_to_euler_deg(q);
            let q2 = euler_deg_to_quat(bx, by, bz);
            assert_same_rotation(q, q2, 1e-4);
        }
    }

    /// `euler_deg_to_quat` daima birim (normalize) quaternion üretmeli.
    #[test]
    fn produced_quaternion_is_unit_length() {
        for (rx, ry, rz) in [(0.0, 0.0, 0.0), (90.0, 45.0, 30.0), (-123.0, 47.0, 200.0)] {
            let q = euler_deg_to_quat(rx, ry, rz);
            assert!((q.length() - 1.0).abs() < 1e-5, "|q|={}", q.length());
        }
    }

    /// 360°'lik tam tur, sıfır dönüş ile aynı rotasyonu vermeli (periyodiklik).
    #[test]
    fn full_turn_is_equivalent_to_no_rotation() {
        let q_full = euler_deg_to_quat(360.0, 0.0, 0.0);
        assert_same_rotation(q_full, Quat::IDENTITY, 1e-4);
    }
}


