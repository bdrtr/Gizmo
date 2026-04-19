use crate::EditorState;
use gizmo_core::World;

pub fn ui_scene_view(ui: &mut egui::Ui, world: &World, state: &mut EditorState) {
    state.scene_view_visible = true;

    let response = ui.allocate_response(ui.available_size(), egui::Sense::click_and_drag());
    let rect = response.rect;

    state.scene_view_rect = Some(rect);

    if let Some(texture_id) = state.scene_texture_id {
        let mut mesh = egui::Mesh::with_texture(texture_id);
        mesh.add_rect_with_uv(
            rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
        ui.painter().add(mesh);
    } else {
        ui.allocate_ui_at_rect(rect, |ui| {
            ui.centered_and_justified(|ui| {
                ui.label(
                    egui::RichText::new("Gizmo Scene View")
                        .color(egui::Color32::from_white_alpha(50)),
                );
            });
        });
    }

    // --- GIZMO FARE (MOUSE) ETKİLEŞİMLERİ ---
    if let Some(hover_pos) = ui.input(|i| i.pointer.hover_pos()) {
        if response.contains_pointer() || response.dragged() {
            // Fare sahne içinde veya sürükleniyor ise NDC (-1.0 ile 1.0) hesapla
            let nx = ((hover_pos.x - rect.left()) / rect.width()) * 2.0 - 1.0;
            let ny = 1.0 - ((hover_pos.y - rect.top()) / rect.height()) * 2.0;

            state.mouse_ndc = Some(gizmo_math::Vec2::new(nx, ny));

            if response.clicked_by(egui::PointerButton::Primary)
                || response.drag_started_by(egui::PointerButton::Primary)
            {
                state.do_raycast = true;
            }

            // Sağ tık kamerayı çevirmek için (Egui ham input'u yuttuğu için burdan geçirmeliyiz)
            if response.dragged_by(egui::PointerButton::Secondary) {
                let delta = response.drag_delta();
                state.camera_look_delta =
                    Some(gizmo_math::Vec2::new(delta.x, delta.y));
            } else {
                state.camera_look_delta = None;
            }

            // Orta tık kamerayı kaydırmak (Pan) için
            if response.dragged_by(egui::PointerButton::Middle) {
                let delta = response.drag_delta();
                state.camera_pan_delta =
                    Some(gizmo_math::Vec2::new(delta.x, delta.y));
            } else {
                state.camera_pan_delta = None;
            }

            // Alt + Sol Tık Orbit için
            let alt_pressed = ui.input(|i| i.modifiers.alt);
            if alt_pressed && response.dragged_by(egui::PointerButton::Primary) {
                let delta = response.drag_delta();
                state.camera_orbit_delta =
                    Some(gizmo_math::Vec2::new(delta.x, delta.y));
            } else {
                state.camera_orbit_delta = None;
            }

            // Scroll Zoom için
            let scroll_y = ui.input(|i| i.raw_scroll_delta.y); // raw_scroll kullanırsak daha yumuşak gelir
            if scroll_y.abs() > 0.0 {
                state.camera_scroll_delta = Some(scroll_y);
            } else {
                state.camera_scroll_delta = None;
            }
        } else {
            state.mouse_ndc = None;
            state.camera_look_delta = None;
            state.camera_pan_delta = None;
            state.camera_orbit_delta = None;
            state.camera_scroll_delta = None;
        }
    }

    // Dışarıdan veya UI'dan sürüklenen objeyi Scene View'a bırakma yakakalayıcısı
    if let Some(dragged_path) = ui.memory(|m| {
        m.data
            .get_temp::<String>(egui::Id::new("dragged_asset_path"))
    }) {
        if response.hovered() && ui.input(|i| i.pointer.any_released()) {
            state.spawn_asset_request = Some(dragged_path);

            // Farenin bırakıldığı yerin NDC koordinatı ile objeyi spawner'a gönderelim (sıfır değil)
            // Asset Drop Raycasting (Aşama 2):
            if let Some(ndc) = state.mouse_ndc {
                // Biz şimdilik NDC'yi direkt pozisyon olarak veriyoruz (bunu main.rs raycast'e dönüştürecek)
                state.spawn_asset_position =
                    Some(gizmo_math::Vec3::new(ndc.x, ndc.y, 1.0));
            } else {
                state.spawn_asset_position = Some(gizmo_math::Vec3::ZERO);
            }

            ui.memory_mut(|m| {
                m.data.remove::<String>(egui::Id::new("dragged_asset_path"))
            });
        }
    }

    // --- EGUI-GIZMO Entegrasyonu (Aşama 1) ---
    let mut gizmo_interacted = false;
    if let (Some(view_mat), Some(proj_mat)) =
        (state.camera_view, state.camera_proj)
    {
        if !state.selected_entities.is_empty() {
            if let Some(mut transforms) = world
                .borrow_mut::<gizmo_physics::components::Transform>()
            {
                let primary_id = *state.selected_entities.iter().next().unwrap();
                let mut primary_model_mat = gizmo_math::Mat4::IDENTITY;
                if let Some(primary_t) = transforms.get(primary_id) {
                    primary_model_mat = primary_t.model_matrix();
                }

                let gizmo_mode = match state.gizmo_mode {
                    crate::editor_state::GizmoMode::Translate => {
                        egui_gizmo::GizmoMode::Translate
                    }
                    crate::editor_state::GizmoMode::Rotate => {
                        egui_gizmo::GizmoMode::Rotate
                    }
                    crate::editor_state::GizmoMode::Scale => {
                        egui_gizmo::GizmoMode::Scale
                    }
                };

                let gizmo_orientation = if state.gizmo_local_space {
                    egui_gizmo::GizmoOrientation::Local
                } else {
                    egui_gizmo::GizmoOrientation::Global
                };

                let snap_enabled = state.prefs.snap_enabled || ui.input(|i| i.modifiers.command);
                let snap_distance = state.prefs.snap_translate;
                let snap_angle = state.prefs.snap_rotate_deg.to_radians();

                let gizmo = egui_gizmo::Gizmo::new("scene_gizmo")
                    .view_matrix(view_mat.to_cols_array_2d().into())
                    .projection_matrix(proj_mat.to_cols_array_2d().into())
                    .model_matrix(primary_model_mat.to_cols_array_2d().into())
                    .mode(gizmo_mode)
                    .orientation(gizmo_orientation)
                    .snapping(snap_enabled)
                    .snap_distance(snap_distance)
                    .snap_angle(snap_angle)
                    .visuals(egui_gizmo::GizmoVisuals {
                        gizmo_size: state.prefs.gizmo_size,
                        ..Default::default()
                    });

                if let Some(result) = gizmo.interact(ui) {
                    gizmo_interacted = true;
                    if state.gizmo_original_transforms.is_empty() {
                        // Tüm seçili objelerin orijinal durumlarını kaydet
                        for &id in state.selected_entities.iter() {
                            if let Some(tx) = transforms.get(id) {
                                state.gizmo_original_transforms.insert(id, *tx);
                            }
                        }
                    }

                    if let Some(orig_pivot) =
                        state.gizmo_original_transforms.get(&primary_id)
                    {
                        let new_mat = gizmo_math::Mat4::from_cols_array_2d(
                            &result.transform().into(),
                        );
                        let delta_mat = new_mat * orig_pivot.model_matrix().inverse();

                        for &id in state.selected_entities.iter() {
                            if let Some(orig_t) =
                                state.gizmo_original_transforms.get(&id)
                            {
                                if let Some(t) = transforms.get_mut(id) {
                                    let final_mat = delta_mat * orig_t.model_matrix();
                                    let (scale, rot, pos) =
                                        final_mat.to_scale_rotation_translation();
                                    t.position = pos;
                                    t.rotation = rot;
                                    t.scale = scale;
                                    t.update_local_matrix();
                                }
                            }
                        }
                    }
                } else if !state.gizmo_original_transforms.is_empty() {
                    // Sürükleme bittiğinde değişimi History'e aktar
                    let mut changes = Vec::new();
                    for &id in state.selected_entities.iter() {
                        if let Some(old_t) =
                            state.gizmo_original_transforms.get(&id)
                        {
                            if let Some(t) = transforms.get(id) {
                                if old_t.position != t.position
                                    || old_t.rotation != t.rotation
                                    || old_t.scale != t.scale
                                {
                                    changes.push((id, *old_t, *t));
                                }
                            }
                        }
                    }

                    if !changes.is_empty() {
                        state.history.push(
                            crate::history::EditorAction::TransformsChanged { changes },
                        );
                    }
                    state.gizmo_original_transforms.clear();
                }
            }
        }
    }

    // --- RUBBER BAND (KUTU İLE ÇOKLU SEÇİM) ---
    if !gizmo_interacted && response.dragged_by(egui::PointerButton::Primary) {
        if state.rubber_band_start.is_none() {
            if let Some(pos) = ui.input(|i| i.pointer.press_origin()) {
                state.rubber_band_start = Some(gizmo_math::Vec2::new(pos.x, pos.y));
            }
        }
        if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
            state.rubber_band_current = Some(gizmo_math::Vec2::new(pos.x, pos.y));
        }
    }

    if response.drag_released_by(egui::PointerButton::Primary) {
        if let (Some(start), Some(curr)) = (state.rubber_band_start, state.rubber_band_current) {
            let diff_x = (start.x - curr.x).abs();
            let diff_y = (start.y - curr.y).abs();
            if diff_x > 5.0 || diff_y > 5.0 {
                // Kutuyu onaylamak için event isteği bırak (studio_input'ta işlenecek)
                state.rubber_band_request = Some((start, curr));
            }
        }
        state.rubber_band_start = None;
        state.rubber_band_current = None;
    }

    if let (Some(start), Some(curr)) = (state.rubber_band_start, state.rubber_band_current) {
        let rect = egui::Rect::from_two_pos(
            egui::pos2(start.x, start.y),
            egui::pos2(curr.x, curr.y),
        );
        ui.painter().rect(
            rect,
            0.0,
            egui::Color32::from_white_alpha(30),
            egui::Stroke::new(1.0, egui::Color32::WHITE),
        );
    }
}
