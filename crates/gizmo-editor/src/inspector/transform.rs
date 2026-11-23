
use crate::editor_state::EditorState;
use egui;
use gizmo_core::World;
use gizmo_physics_core::Transform;


/// The transform rows: position, rotation in degrees, and scale.
pub fn draw_transform_section(
    ui: &mut egui::Ui,
    world: &World,
    entity_id: gizmo_core::entity::Entity,
    state: &mut EditorState,
) {
    // SAFETY: editor UI runs single-threaded in the egui draw; no concurrent World access.
    let mut transforms = unsafe { world.query_unchecked::<&mut Transform>() };
    let old_t = transforms.get_mut(entity_id).map(|t| *t);
    
    // Pointer status check for Drag operations
    let pointer_down = ui.input(|i| i.pointer.any_down());
    let mut changed = false;

    if let Some(mut t) = transforms.get_mut(entity_id) {
        let _is_interacting_with_transform = egui::CollapsingHeader::new(crate::theme::section_title("Transform"))
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
        // The rigid bodies under the edited entity go with it (`carried_bodies`): written once the
        // undo bookkeeping below has read them as they were.
        let carried = match (changed, old_t) {
            (true, Some(before)) => {
                let selection: Vec<_> = state.selection.entities.iter().copied().collect();
                crate::carried_bodies::carried(world, entity_id, &before, &t, &selection)
            }
            _ => Vec::new(),
        };
        
        // --- UNDO/REDO TRACKING (INSPECTOR) ---
        if pointer_down && changed && state.inspector_drag_original_transforms.is_empty() {
            // Sürükleme (drag) işlemi başladı, asıl transformları yedekle — the edited entity's as it was
            // before this frame's change (`t` already holds it), and the bodies the selection carries.
            let selection: Vec<_> = state.selection.entities.iter().copied().collect();
            let mut recorded = selection.clone();
            for &ent in &selection {
                recorded.extend(crate::carried_bodies::bodies_under(world, ent, &selection));
            }
            for ent in recorded {
                let original_t = if ent == entity_id { old_t } else { transforms.get_mut(ent).map(|t| *t) };
                if let Some(original_t) = original_t {
                    state.inspector_drag_original_transforms.entry(ent).or_insert(original_t);
                }
            }
        } else if !pointer_down && !state.inspector_drag_original_transforms.is_empty() {
            // Fare bırakıldı, değişiklikleri kaydet
            let mut undo_changes = Vec::new();
            for (ent, old_transform) in state.inspector_drag_original_transforms.drain() {
                if let Some(new_transform) = transforms.get_mut(ent).map(|t| *t) {
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
                let mut undo_changes = vec![(entity_id, old_transform, *t)];
                undo_changes.extend(carried.iter().copied());
                
                // Çoklu seçim varsa diğerlerinin de son halini kaydetmeliyiz.
                // Not: Delta hesaplaması hemen aşağıda olduğu için, onların yeni t'sini bir sonraki kareye bırakamayız.
                // Bu yüzden keyboard input için basit tutuyoruz.
                state.history.push(crate::history::EditorAction::TransformsChanged { changes: undo_changes });
                state.status_message = "💾 Obje Inspector'dan değiştirildi (Undo: Ctrl+Z)".to_string();
            }
        }
        
        ui.separator();
        for (body, _, next) in carried {
            if let Some(mut b) = transforms.get_mut(body) {
                *b = next;
            }
        }
    }

    // Çoklu seçim (Multi-Object Editing) Delta Uygulaması
    if changed && state.selection.entities.len() > 1 {
        if let Some(old) = old_t {
            if let Some(new_t) = transforms.get_mut(entity_id).map(|t| *t) {
                let delta_pos = new_t.position - old.position;
                let delta_rot = new_t.rotation * old.rotation.inverse();
                let delta_scale = gizmo_math::Vec3::new(
                    if old.scale.x != 0.0 { new_t.scale.x / old.scale.x } else { 1.0 },
                    if old.scale.y != 0.0 { new_t.scale.y / old.scale.y } else { 1.0 },
                    if old.scale.z != 0.0 { new_t.scale.z / old.scale.z } else { 1.0 },
                );

                let others: Vec<_> = state.selection.entities.iter().copied().filter(|&e| e != entity_id).collect();
                let selection: Vec<_> = state.selection.entities.iter().copied().collect();
                for e in others {
                    let Some(before) = transforms.get_mut(e).map(|t| *t) else { continue };
                    let mut after = before;
                    after.position += delta_pos;
                    after.rotation = delta_rot * after.rotation;
                    after.scale *= delta_scale;
                    after.update_local_matrix();
                    let carried = crate::carried_bodies::carried(world, e, &before, &after, &selection);
                    if let Some(mut other_t) = transforms.get_mut(e) {
                        *other_t = after;
                    }
                    for (body, _, next) in carried {
                        if let Some(mut b) = transforms.get_mut(body) {
                            *b = next;
                        }
                    }
                }
            }
        }
    }
}


/// A quaternion as Euler angles in degrees, for display.
///
/// The decomposition is not unique — several triples name the same rotation, and at gimbal lock
/// (|pitch| ≈ 90°) yaw and roll trade off — so a round trip through here preserves the *rotation*
/// rather than the numbers.
pub fn quat_to_euler_deg(q: gizmo_math::Quat) -> (f32, f32, f32) {
    let (x, y, z) = q.to_euler(gizmo_math::EulerRot::XYZ);
    (x.to_degrees(), y.to_degrees(), z.to_degrees())
}


/// Euler angles in degrees back to a unit quaternion — the inverse of
/// [`quat_to_euler_deg`], up to the ambiguity described there.
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

    /// Every painted text and where it starts, `Shape::Vec` recursed into.
    fn texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<(String, egui::Pos2)> {
        fn walk(shape: &egui::Shape, out: &mut Vec<(String, egui::Pos2)>) {
            match shape {
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                egui::Shape::Text(t) => out.push((t.galley.text().to_string(), t.visual_bounding_rect().center())),
                _ => {}
            }
        }
        let mut out = Vec::new();
        shapes.iter().for_each(|c| walk(&c.shape, &mut out));
        out
    }

    /// A group moved from the inspector's Position X takes the rigid body in it along — a body's
    /// `Transform` is its world pose, so the hierarchy alone does not move it (docs/ENGINE.md §3
    /// item 7) — and one undo puts both back where they were before the drag's first frame.
    #[test]
    fn a_group_moved_in_the_inspector_carries_its_body() {
        use gizmo_core::hierarchy::HierarchyExt;
        let mut world = World::new();
        let group = world.spawn_bundle((Transform::new(Vec3::ZERO),));
        let body = world.spawn_bundle((Transform::new(Vec3::new(0.0, 2.0, 0.0)), gizmo_physics_rigid::components::RigidBody::new(1.0, true)));
        world.add_child(group, body);
        let mut state = EditorState::default();
        state.select_exclusive(group);
        let ctx = egui::Context::default();
        let frame = |events: Vec<egui::Event>, state: &mut EditorState| {
            let output = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| draw_transform_section(ui, &world, group, state));
            let found = texts(&output.shapes);
            output.drop_without_applying_deltas();
            found
        };
        // The first number after the "Pozisyon:" row's "X" is Position X.
        let shown = frame(vec![], &mut state);
        let x_label = shown.iter().position(|(t, _)| t == "X").expect("the X label");
        let at = shown[x_label + 1].1;
        let pointer = |pos: egui::Pos2, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(vec![egui::Event::PointerMoved(at)], &mut state);
        frame(vec![pointer(at, true)], &mut state);
        for dx in [5.0, 10.0, 15.0, 20.0] {
            frame(vec![egui::Event::PointerMoved(at + egui::vec2(dx, 0.0))], &mut state);
        }
        frame(vec![pointer(at + egui::vec2(20.0, 0.0), false)], &mut state);
        frame(vec![], &mut state);

        let read = |world: &World, e| *world.get::<Transform>(e).unwrap();
        let (g, b) = (read(&world, group), read(&world, body));
        assert!(g.position.x > 0.1, "the drag did not move the group: {}", g.position);
        assert!(b.position.abs_diff_eq(Vec3::new(g.position.x, 2.0, 0.0), 1e-4), "the body stayed behind: {} (group at {})", b.position, g.position);
        assert!(state.history.can_undo(), "the drag is not in the history");
        state.history.undo(&mut world);
        assert_eq!(read(&world, group).position, Vec3::ZERO, "the undo missed the drag's first frame");
        assert_eq!(read(&world, body).position, Vec3::new(0.0, 2.0, 0.0), "the undo left the body where it was carried");
    }

    /// Checks whether two quaternions represent the SAME rotation by rotating the three basis
    /// vectors and comparing the results. That naturally sidesteps both `q` and `-q` being the
    /// same rotation (the double cover) and Euler decomposition producing different but
    /// equivalent triples.
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

    /// The degree → radian conversion must be right: 90° about the X axis must give the same
    /// rotation as glam's `from_rotation_x(PI/2)`.
    #[test]
    fn degrees_are_converted_to_radians() {
        let q = euler_deg_to_quat(90.0, 0.0, 0.0);
        assert_same_rotation(q, Quat::from_rotation_x(std::f32::consts::FRAC_PI_2), 1e-5);

        let qy = euler_deg_to_quat(0.0, 45.0, 0.0);
        assert_same_rotation(qy, Quat::from_rotation_y(45.0_f32.to_radians()), 1e-5);
    }

    /// Single-axis rotations must read back cleanly as Euler angles (no gimbal lock).
    #[test]
    fn single_axis_round_trips_to_expected_degrees() {
        let (x, y, z) = quat_to_euler_deg(Quat::from_rotation_y(45.0_f32.to_radians()));
        assert!(x.abs() < 1e-3, "x={}", x);
        assert!((y - 45.0).abs() < 1e-3, "y={}", y);
        assert!(z.abs() < 1e-3, "z={}", z);
    }

    /// The real invariant: quat → euler → quat must give the SAME rotation as the input.
    /// Swept over representative angle triples outside gimbal lock (|pitch|≈90°).
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

    /// `euler_deg_to_quat` must always produce a unit (normalised) quaternion.
    #[test]
    fn produced_quaternion_is_unit_length() {
        for (rx, ry, rz) in [(0.0, 0.0, 0.0), (90.0, 45.0, 30.0), (-123.0, 47.0, 200.0)] {
            let q = euler_deg_to_quat(rx, ry, rz);
            assert!((q.length() - 1.0).abs() < 1e-5, "|q|={}", q.length());
        }
    }

    /// A full 360° turn must give the same rotation as none at all (periodicity).
    #[test]
    fn full_turn_is_equivalent_to_no_rotation() {
        let q_full = euler_deg_to_quat(360.0, 0.0, 0.0);
        assert_same_rotation(q_full, Quat::IDENTITY, 1e-4);
    }
}


