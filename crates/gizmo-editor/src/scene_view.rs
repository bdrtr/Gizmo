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
    let (alt_pressed, scroll_y, hover_pos, any_released, primary_down, press_origin) = ui.input(|i| (
        i.modifiers.alt,
        i.raw_scroll_delta.y,
        i.pointer.hover_pos(),
        i.pointer.any_released(),
        i.pointer.primary_down(),
        i.pointer.press_origin(),
    ));

    if response.contains_pointer() || response.dragged() {
        if let Some(pos) = hover_pos {
            // Fare sahne içinde veya sürükleniyor ise NDC (-1.0 ile 1.0) hesapla
            let nx = ((pos.x - rect.left()) / rect.width()) * 2.0 - 1.0;
            let ny = 1.0 - ((pos.y - rect.top()) / rect.height()) * 2.0;

            state.mouse_ndc = Some(gizmo_math::Vec2::new(nx, ny));
        }

        if response.clicked_by(egui::PointerButton::Primary)
            || response.drag_started_by(egui::PointerButton::Primary)
        {
            state.do_raycast = true;
        }

        // Sağ tık kamerayı çevirmek için (Egui ham input'u yuttuğu için burdan geçirmeliyiz)
        if response.dragged_by(egui::PointerButton::Secondary) {
            let delta = response.drag_delta();
            state.camera.look_delta =
                Some(gizmo_math::Vec2::new(delta.x, delta.y));
        } else {
            state.camera.look_delta = None;
        }

        // Orta tık kamerayı kaydırmak (Pan) için
        if response.dragged_by(egui::PointerButton::Middle) {
            let delta = response.drag_delta();
            state.camera.pan_delta =
                Some(gizmo_math::Vec2::new(delta.x, delta.y));
        } else {
            state.camera.pan_delta = None;
        }

        // Alt + Sol Tık Orbit için
        if alt_pressed && response.dragged_by(egui::PointerButton::Primary) {
            let delta = response.drag_delta();
            state.camera.orbit_delta =
                Some(gizmo_math::Vec2::new(delta.x, delta.y));
        } else {
            state.camera.orbit_delta = None;
        }

        // Scroll Zoom için
        if scroll_y.abs() > 0.0 {
            state.camera.scroll_delta = Some(scroll_y);
        } else {
            state.camera.scroll_delta = None;
        }
    } else {
        state.mouse_ndc = None;
        state.camera.look_delta = None;
        state.camera.pan_delta = None;
        state.camera.orbit_delta = None;
        state.camera.scroll_delta = None;
    }

    // Dışarıdan veya UI'dan sürüklenen objeyi Scene View'a bırakma yakakalayıcısı
    if let Some(dragged_path) = state.dragged_asset.clone() {
        if response.hovered() && any_released {
            state.spawn_asset_request = Some(dragged_path);

            // Farenin bırakıldığı yerin NDC koordinatı ile objeyi spawner'a gönderelim (sıfır değil)
            // Asset Drop Raycasting (Aşama 2):
            if let Some(_ndc) = state.mouse_ndc {
                todo!("NDC koordinatı kamera raycast'i kullanılarak world position'a dönüştürülmeli");
            } else {
                state.spawn_asset_position = Some(gizmo_math::Vec3::ZERO);
            }

            state.dragged_asset = None;
        }
    }

    // --- EGUI-GIZMO Entegrasyonu (Aşama 1) ---
    let mut gizmo_interacted = false;
    if let (Some(view_mat), Some(proj_mat)) =
        (state.camera.view, state.camera.proj)
    {
        if !state.selection.entities.is_empty() {
            let transforms_storage = world.borrow_mut::<gizmo_physics::components::Transform>();
            match transforms_storage {
                Err(e) => {
                    eprintln!("ECS borrow hatası: {:?}", e);
                }
                Ok(mut transforms) => {
                    let primary_id = state.selection.primary.unwrap_or_else(|| *state.selection.entities.iter().next().unwrap());
                    let mut primary_model_mat = gizmo_math::Mat4::IDENTITY;
                    if let Some(primary_t) = transforms.get(primary_id.id()) {
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
                    if state.scene.gizmo_original_transforms.is_empty() {
                        // Tüm seçili objelerin orijinal durumlarını kaydet
                        for &id in state.selection.entities.iter() {
                            if let Some(tx) = transforms.get(id.id()) {
                                state.scene.gizmo_original_transforms.insert(id, *tx);
                            }
                        }
                    }

                    if let Some(orig_pivot) =
                        state.scene.gizmo_original_transforms.get(&primary_id)
                    {
                        let new_mat = gizmo_math::Mat4::from_cols_array_2d(
                            &result.transform().into(),
                        );
                        let delta_mat = new_mat * orig_pivot.model_matrix().inverse();

                        for &id in state.selection.entities.iter() {
                            if let Some(orig_t) =
                                state.scene.gizmo_original_transforms.get(&id)
                            {
                                if let Some(t) = transforms.get_mut(id.id()) {
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
                } else if !state.scene.gizmo_original_transforms.is_empty() {
                    // Gizmo bırakıldıysa veya sürükleme iptal edildiyse
                    if !primary_down {
                        // Sürükleme bittiğinde değişimi History'e aktar
                    let mut changes = Vec::new();
                    for &id in state.selection.entities.iter() {
                        if let Some(old_t) = state.scene.gizmo_original_transforms.get(&id) {
                            if let Some(t) = transforms.get(id.id()) {
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
                    state.scene.gizmo_original_transforms.clear();
                    }
                }
                }
            }
        }
    }

    // --- RUBBER BAND (KUTU İLE ÇOKLU SEÇİM) ---
    let is_dragging_gizmo = gizmo_interacted || !state.scene.gizmo_original_transforms.is_empty();
    if !is_dragging_gizmo && response.dragged_by(egui::PointerButton::Primary) {
        if state.selection.rubber_band_start.is_none() {
            if let Some(pos) = press_origin {
                state.selection.rubber_band_start = Some(gizmo_math::Vec2::new(pos.x, pos.y));
            }
        }
        if let Some(pos) = hover_pos {
            state.selection.rubber_band_current = Some(gizmo_math::Vec2::new(pos.x, pos.y));
        }
    }

    if response.drag_released_by(egui::PointerButton::Primary) {
        if let (Some(start), Some(curr)) = (state.selection.rubber_band_start, state.selection.rubber_band_current) {
            let diff_x = (start.x - curr.x).abs();
            let diff_y = (start.y - curr.y).abs();
            if diff_x > 5.0 || diff_y > 5.0 {
                // Kutuyu onaylamak için event isteği bırak (studio_input'ta işlenecek)
                state.selection.rubber_band_request = Some((start, curr));
            }
        }
        state.selection.rubber_band_start = None;
        state.selection.rubber_band_current = None;
    }

    if let (Some(start), Some(curr)) = (state.selection.rubber_band_start, state.selection.rubber_band_current) {
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
