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
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            ui.centered_and_justified(|ui| {
                ui.label(
                    egui::RichText::new("Gizmo Scene View")
                        .color(egui::Color32::from_white_alpha(50)),
                );
            });
        });
    }

    draw_viewport_overlays(ui, world, state, rect);

    // --- GIZMO FARE (MOUSE) ETKİLEŞİMLERİ ---
    let (hover_pos, interact_pos, _latest_pos, any_released, alt_pressed, scroll_y, _primary_down, press_origin) =
        ui.input(|i| {
            (
                i.pointer.hover_pos(),
                i.pointer.interact_pos(),
                i.pointer.latest_pos(),
                i.pointer.any_released(),
                i.modifiers.alt,
                i.smooth_scroll_delta.y,
                i.pointer.press_origin(),
                i.pointer.press_origin(), // Sadece tuple uyumluluğu için
            )
        });

    if response.contains_pointer() || response.dragged() {
        if let Some(pos) = interact_pos {
            // Fare sahne içinde veya sürükleniyor ise NDC (-1.0 ile 1.0) hesapla
            let nx = ((pos.x - rect.left()) / rect.width()) * 2.0 - 1.0;
            let ny = 1.0 - ((pos.y - rect.top()) / rect.height()) * 2.0;

            state.mouse_ndc = Some(gizmo_math::Vec2::new(nx, ny));
        }


        // Sağ tık kamerayı çevirmek için (Egui ham input'u yuttuğu için burdan geçirmeliyiz)
        if response.dragged_by(egui::PointerButton::Secondary) {
            let delta = response.drag_delta();
            state.camera.look_delta = Some(gizmo_math::Vec2::new(delta.x, delta.y));
        } else {
            state.camera.look_delta = None;
        }

        // Orta tık kamerayı kaydırmak (Pan) için
        if response.dragged_by(egui::PointerButton::Middle) {
            let delta = response.drag_delta();
            state.camera.pan_delta = Some(gizmo_math::Vec2::new(delta.x, delta.y));
        } else {
            state.camera.pan_delta = None;
        }

        // Alt + Sol Tık Orbit için
        if alt_pressed && response.dragged_by(egui::PointerButton::Primary) {
            let delta = response.drag_delta();
            state.camera.orbit_delta = Some(gizmo_math::Vec2::new(delta.x, delta.y));
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

    if let Some(dragged_path) = state.dragged_asset.clone() {
        if any_released {
            let latest_pos = ui.input(|i| i.pointer.latest_pos());
            let in_scene = latest_pos.map(|p| rect.contains(p)).unwrap_or(false);
            
            tracing::info!(">>> DRAG RELEASED! latest_pos: {:?}, rect: {:?}, in_scene: {}", latest_pos, rect, in_scene);
            
            if in_scene {
                let mut spawn_pos = gizmo_math::Vec3::ZERO;
                
                // Fare pozisyonuna göre yer düzlemi (Y=0) ile kesişimi hesapla
                if let (Some(ndc), Some(view_mat), Some(proj_mat)) = (state.mouse_ndc, state.camera.view, state.camera.proj) {
                    let view_proj_inv = (proj_mat * view_mat).inverse();
                    let ray = gizmo_math::Ray::from_ndc(ndc, view_proj_inv);
                    
                    if ray.direction.y.abs() > 1e-4 {
                        let t = -ray.origin.y / ray.direction.y;
                        if t > 0.0 {
                            let raw_pos = ray.at(t);
                            let snap = if state.prefs.snap_enabled { state.prefs.snap_translate } else { 0.0 };
                            if snap > 0.0 {
                                spawn_pos = gizmo_math::Vec3::new(
                                    (raw_pos.x / snap).round() * snap,
                                    0.0,
                                    (raw_pos.z / snap).round() * snap,
                                );
                            } else {
                                spawn_pos = gizmo_math::Vec3::new(raw_pos.x, 0.0, raw_pos.z);
                            }
                        }
                    }
                }

                state.log_info(&format!("Model sahneye bırakıldı: {} ({:.1}, {:.1}, {:.1})", dragged_path, spawn_pos.x, spawn_pos.y, spawn_pos.z));
                state.spawn_asset_request = Some(dragged_path);
                state.spawn_asset_position = Some(spawn_pos);
            }
            state.dragged_asset = None; // Her ihtimale karşı sıfırla
        }
    }
    // --- EGUI-GIZMO Entegrasyonu (Aşama 1) ---
    let mut gizmo_interacted = false;
    
    if let (Some(view_mat), Some(proj_mat)) =
        (state.camera.view, state.camera.proj)
    {
        if !state.selection.entities.is_empty() {
            // SAFETY: editor UI runs single-threaded in the egui draw; no concurrent World access.
            let mut transforms = unsafe { world.borrow_mut_unchecked::<gizmo_physics_core::Transform>() };
            
            let primary_id = state.selection.primary.unwrap_or_else(|| *state.selection.entities.iter().next().unwrap());
            if transforms.get_mut(primary_id.id()).is_some() {
                
                use transform_gizmo_egui::prelude::*;
                use transform_gizmo_egui::math::Transform as GizmoTransform;

                let gizmo_orientation = if state.gizmo_local_space {
                    GizmoOrientation::Local
                } else {
                    GizmoOrientation::Global
                };

                let is_ctrl = ui.input(|i| i.modifiers.ctrl);
                let snap_enabled = snap_active(state.prefs.snap_enabled, is_ctrl);
                let snap_distance = if snap_enabled { state.prefs.snap_translate } else { 0.0 };
                let snap_angle = if snap_enabled { state.prefs.snap_rotate_deg.to_radians() } else { 0.0 };
                let snap_scale = if snap_enabled { state.prefs.snap_scale } else { 0.0 };

                let vm = view_mat.to_cols_array_2d();
                let pm = proj_mat.to_cols_array_2d();
                
                let view_matrix = transform_gizmo_egui::mint::RowMatrix4 {
                    x: transform_gizmo_egui::mint::Vector4 { x: vm[0][0] as f64, y: vm[1][0] as f64, z: vm[2][0] as f64, w: vm[3][0] as f64 },
                    y: transform_gizmo_egui::mint::Vector4 { x: vm[0][1] as f64, y: vm[1][1] as f64, z: vm[2][1] as f64, w: vm[3][1] as f64 },
                    z: transform_gizmo_egui::mint::Vector4 { x: vm[0][2] as f64, y: vm[1][2] as f64, z: vm[2][2] as f64, w: vm[3][2] as f64 },
                    w: transform_gizmo_egui::mint::Vector4 { x: vm[0][3] as f64, y: vm[1][3] as f64, z: vm[2][3] as f64, w: vm[3][3] as f64 },
                };
                let projection_matrix = transform_gizmo_egui::mint::RowMatrix4 {
                    x: transform_gizmo_egui::mint::Vector4 { x: pm[0][0] as f64, y: pm[1][0] as f64, z: pm[2][0] as f64, w: pm[3][0] as f64 },
                    y: transform_gizmo_egui::mint::Vector4 { x: pm[0][1] as f64, y: pm[1][1] as f64, z: pm[2][1] as f64, w: pm[3][1] as f64 },
                    z: transform_gizmo_egui::mint::Vector4 { x: pm[0][2] as f64, y: pm[1][2] as f64, z: pm[2][2] as f64, w: pm[3][2] as f64 },
                    w: transform_gizmo_egui::mint::Vector4 { x: pm[0][3] as f64, y: pm[1][3] as f64, z: pm[2][3] as f64, w: pm[3][3] as f64 },
                };

                let config = GizmoConfig {
                    view_matrix,
                    projection_matrix,
                    viewport: transform_gizmo_egui::math::Rect::from_min_max(
                        transform_gizmo_egui::math::Pos2::new(rect.min.x, rect.min.y),
                        transform_gizmo_egui::math::Pos2::new(rect.max.x, rect.max.y),
                    ),
                    modes: match state.gizmo_mode {
                        crate::editor_state::GizmoMode::Translate => {
                            let mut m = transform_gizmo_egui::EnumSet::empty();
                            m.insert(transform_gizmo_egui::GizmoMode::TranslateX);
                            m.insert(transform_gizmo_egui::GizmoMode::TranslateY);
                            m.insert(transform_gizmo_egui::GizmoMode::TranslateZ);
                            m.insert(transform_gizmo_egui::GizmoMode::TranslateXY);
                            m.insert(transform_gizmo_egui::GizmoMode::TranslateYZ);
                            m.insert(transform_gizmo_egui::GizmoMode::TranslateXZ);
                            m
                        },
                        crate::editor_state::GizmoMode::Rotate => {
                            let mut m = transform_gizmo_egui::EnumSet::empty();
                            m.insert(transform_gizmo_egui::GizmoMode::RotateX);
                            m.insert(transform_gizmo_egui::GizmoMode::RotateY);
                            m.insert(transform_gizmo_egui::GizmoMode::RotateZ);
                            m
                        },
                        crate::editor_state::GizmoMode::Scale => {
                            let mut m = transform_gizmo_egui::EnumSet::empty();
                            m.insert(transform_gizmo_egui::GizmoMode::ScaleX);
                            m.insert(transform_gizmo_egui::GizmoMode::ScaleY);
                            m.insert(transform_gizmo_egui::GizmoMode::ScaleZ);
                            m.insert(transform_gizmo_egui::GizmoMode::ScaleUniform);
                            m
                        },
                        crate::editor_state::GizmoMode::Select => transform_gizmo_egui::GizmoMode::all(),
                    },
                    orientation: gizmo_orientation,
                    // `snapping` is the gate: `transform-gizmo` reads `snap_distance`,
                    // `snap_angle` and `snap_scale` ONLY inside `if config.snapping` (see
                    // subgizmo/{translation,rotation,scale}.rs in the crate). It was never
                    // assigned here, so `..Default::default()` supplied `false` and all three
                    // settings — a preferences panel with three sliders and a Ctrl modifier
                    // already wired below — were computed every frame and thrown away.
                    snapping: snap_enabled,
                    snap_distance,
                    snap_angle,
                    snap_scale,
                    visuals: transform_gizmo_egui::GizmoVisuals {
                        gizmo_size: state.prefs.gizmo_size,
                        ..Default::default()
                    },
                    ..Default::default()
                };
                state.transform_gizmo.update_config(config);

                // Tüm seçili objelerin transformlarını topla
                let mut gizmo_transforms = Vec::new();
                let mut selected_ids = Vec::new();

                for &id in state.selection.entities.iter() {
                    if let Some(t) = transforms.get_mut(id.id()).map(|t| *t) {
                        let translation = transform_gizmo_egui::mint::Vector3 { x: t.position.x as f64, y: t.position.y as f64, z: t.position.z as f64 };
                        let rotation = transform_gizmo_egui::mint::Quaternion { v: transform_gizmo_egui::mint::Vector3 { x: t.rotation.x as f64, y: t.rotation.y as f64, z: t.rotation.z as f64 }, s: t.rotation.w as f64 };
                        let scale = transform_gizmo_egui::mint::Vector3 { x: t.scale.x as f64, y: t.scale.y as f64, z: t.scale.z as f64 };
                        
                        gizmo_transforms.push(GizmoTransform::from_scale_rotation_translation(scale, rotation, translation));
                        selected_ids.push(id.id());
                    }
                }

                use transform_gizmo_egui::GizmoExt;
                if state.gizmo_mode != crate::editor_state::GizmoMode::Select && !gizmo_transforms.is_empty() {
                    let is_primary_down = ui.input(|i| i.pointer.primary_down());
                    
                    if let Some((_result, new_transforms)) = state.transform_gizmo.interact(ui, &gizmo_transforms) {
                        gizmo_interacted = true;
                        
                        // Undo (Geri Al) için harekete başlarken ilk değerleri sakla
                        if state.scene.gizmo_original_transforms.is_empty() {
                            for &id in &selected_ids {
                                if let Some(t) = transforms.get_mut(id).map(|t| *t) {
                                    state.scene.gizmo_original_transforms.insert(gizmo_core::entity::Entity::new(id, 0), t);
                                }
                            }
                        }

                        for (i, new_t) in new_transforms.iter().enumerate() {
                            if let Some(&entity_id) = selected_ids.get(i) {
                                if let Some(mut t) = transforms.get_mut(entity_id) {
                                    let nt: transform_gizmo_egui::mint::Vector3<f64> = new_t.translation;
                                    let nr: transform_gizmo_egui::mint::Quaternion<f64> = new_t.rotation;
                                    let ns: transform_gizmo_egui::mint::Vector3<f64> = new_t.scale;
                                    
                                    t.position = gizmo_math::Vec3::new(nt.x as f32, nt.y as f32, nt.z as f32);
                                    t.rotation = gizmo_math::Quat::from_xyzw(nr.v.x as f32, nr.v.y as f32, nr.v.z as f32, nr.s as f32);
                                    t.scale = gizmo_math::Vec3::new(ns.x as f32, ns.y as f32, ns.z as f32);
                                    t.update_local_matrix();
                                }
                            }
                        }
                    } else if !is_primary_down && !state.scene.gizmo_original_transforms.is_empty() {
                        // Fare bırakıldı (Sürükleme bitti), tüm değişiklikleri History'ye bas
                        let mut changes = Vec::new();
                        for (entity, old_t) in state.scene.gizmo_original_transforms.drain() {
                            if let Some(new_t) = transforms.get_mut(entity.id()).map(|t| *t) {
                                if old_t != new_t {
                                    changes.push((entity, old_t, new_t));
                                }
                            }
                        }
                        if !changes.is_empty() {
                            let count = changes.len();
                            state.history.push(crate::history::EditorAction::TransformsChanged { changes });
                            state.status_message = format!("💾 {} obje değiştirildi (Geri Almak için Ctrl+Z)", count);
                        }
                    }
                }
            }
        }
    }

    // --- RUBBER BAND (KUTU İLE ÇOKLU SEÇİM) ---
    let is_dragging_gizmo = gizmo_interacted || !state.scene.gizmo_original_transforms.is_empty();

    if !is_dragging_gizmo
        && (response.clicked_by(egui::PointerButton::Primary)
            || response.drag_started_by(egui::PointerButton::Primary))
        {
            tracing::info!("SceneView CLICKED/DRAG_STARTED! ndc: {:?}", state.mouse_ndc);
            state.do_raycast = true;
        }
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

    if response.drag_stopped_by(egui::PointerButton::Primary) {
        if let (Some(start), Some(curr)) = (
            state.selection.rubber_band_start,
            state.selection.rubber_band_current,
        ) {
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

    if let (Some(start), Some(curr)) = (
        state.selection.rubber_band_start,
        state.selection.rubber_band_current,
    ) {
        let rect =
            egui::Rect::from_two_pos(egui::pos2(start.x, start.y), egui::pos2(curr.x, curr.y));
        ui.painter().rect(
            rect,
            0,
            egui::Color32::from_white_alpha(30),
            egui::Stroke::new(1.0_f32, egui::Color32::WHITE),
            egui::StrokeKind::Inside,
        );
    }

    // F (Focus) Kamera Odaklanması
    if !ui.ctx().egui_wants_keyboard_input() && ui.input(|i| i.key_pressed(egui::Key::F)) {
        if let Some(entity) = state.selection.primary {
            let transforms = world.borrow::<gizmo_physics_core::Transform>();
            if let Some(t) = transforms.get(entity.id()) {
                state.camera.focus_target = Some(t.position);
            }
        }
    }
}

/// Is snapping on for this drag?
///
/// Ctrl **inverts** the preference rather than forcing snapping on: with snapping off it is the
/// hold-to-snap key, and with snapping on it is hold-to-snap-*off*, which is the behaviour every
/// other editor has and the reason this is an XOR and not an OR. It was written inline as `^`,
/// where a reader cannot tell an intended inversion from a typo'd `||`, and where nothing checked
/// the second row of the table.
#[inline]
pub(crate) fn snap_active(pref_enabled: bool, ctrl_held: bool) -> bool {
    pref_enabled ^ ctrl_held
}



/// The prototype's two viewport overlays: a row of state chips at the top, and a RENDER STATS
/// table at the bottom.
///
/// # Why these are two different things
///
/// The chips say what mode you are in — they are the viewport's half of the tool row, and they
/// answer "if I drag now, what happens?". The stats table says what the frame cost. The old overlay
/// mixed both into one translucent box of emoji lines, which meant the answer to "am I in local or
/// global space?" was sitting underneath the frame rate.
///
/// Every number here is measured. `RenderStats` is published by the render pipeline from the batch
/// list it is about to draw, and the frame time comes from `FrameProfiler`. The prototype also
/// shows VRAM; nothing in this engine measures it, so that row is absent rather than plausible.
fn draw_viewport_overlays(
    ui: &egui::Ui,
    world: &World,
    state: &EditorState,
    rect: egui::Rect,
) {
    use crate::theme::palette::*;

    let painter = ui.painter();
    let pad = 8.0;

    // ── Chips, top-left ──────────────────────────────────────────────────────────────────────
    let camera_label = {
        let cameras = world.borrow::<gizmo_renderer::components::Camera>();
        let fov = world
            .iter_alive_entities()
            .into_iter()
            .find_map(|e| cameras.get(e.id()).map(|c| c.fov))
            .unwrap_or(std::f32::consts::FRAC_PI_3);
        // 35 mm equivalent, which is how the prototype labels it: a 36 mm-wide frame subtends
        // `fov`, so the focal length is half the width over the tangent of the half angle.
        let mm = 18.0 / (fov * 0.5).tan();
        format!("Perspective · {mm:.0}mm")
    };
    let mode_label = format!(
        "{} · {}",
        match state.gizmo_mode {
            crate::editor_state::GizmoMode::Select => "Select",
            crate::editor_state::GizmoMode::Translate => "Move",
            crate::editor_state::GizmoMode::Rotate => "Rotate",
            crate::editor_state::GizmoMode::Scale => "Scale",
        },
        if state.gizmo_local_space { "Local" } else { "Global" }
    );
    let snap_label = format!("Snap {:.2} m", state.prefs.snap_translate);

    let mut x = rect.left() + pad;
    for (text, accented) in [
        (camera_label, false),
        (mode_label, false),
        (snap_label, state.prefs.snap_enabled),
    ] {
        let galley = painter.layout_no_wrap(text, egui::FontId::proportional(11.0), TEXT_BRIGHT);
        let size = egui::vec2(galley.size().x + 16.0, 21.0);
        let chip = egui::Rect::from_min_size(egui::pos2(x, rect.top() + pad), size);
        painter.rect_filled(chip, 0.0, if accented { ACCENT_DEEP } else { CHROME });
        painter.rect_stroke(
            chip,
            0.0,
            egui::Stroke::new(1.0_f32, if accented { ACCENT } else { BORDER }),
            egui::StrokeKind::Inside,
        );
        painter.galley(
            egui::pos2(chip.left() + 8.0, chip.center().y - galley.size().y * 0.5),
            galley,
            TEXT_BRIGHT,
        );
        x += size.x + 6.0;
    }

    // ── RENDER STATS, bottom-left ────────────────────────────────────────────────────────────
    let stats = world
        .get_resource::<gizmo_renderer::components::RenderStats>()
        .map(|s| *s)
        .unwrap_or_default();
    let frame_ms = world
        .get_resource::<gizmo_core::FrameProfiler>()
        .map(|p| p.avg_frame_ms(30))
        .unwrap_or(0.0);
    let entities = world.iter_alive_entities().len();

    let mut rows: Vec<(&str, String)> = vec![
        ("frame", format!("{frame_ms:.2} ms")),
        ("draw calls", stats.draw_calls.to_string()),
        ("tris", format_thousands(stats.triangles)),
        ("instances", stats.instances.to_string()),
        ("entities", entities.to_string()),
    ];
    // The prototype's `vram` row, present only when the backend actually reports allocations.
    // Labelled "gpu mem" rather than "vram": this is what wgpu has sub-allocated for this process,
    // not the card's total usage, and the shorter word would claim the larger thing.
    if let Some(bytes) = stats.gpu_allocated_bytes {
        rows.push(("gpu mem", format_bytes(bytes)));
    }

    const ROW: f32 = 16.0;
    const HEADER: f32 = 20.0;
    let width = 200.0_f32.min(rect.width() - pad * 2.0);
    let height = HEADER + rows.len() as f32 * ROW + 6.0;
    let panel = egui::Rect::from_min_size(
        egui::pos2(rect.left() + pad, rect.bottom() - pad - height),
        egui::vec2(width, height),
    );
    if panel.width() < 80.0 || panel.top() < rect.top() {
        return; // too small a viewport to be worth covering
    }
    painter.rect_filled(panel, 0.0, CHROME);
    painter.rect_stroke(panel, 0.0, egui::Stroke::new(1.0_f32, BORDER), egui::StrokeKind::Inside);
    painter.text(
        egui::pos2(panel.left() + 8.0, panel.top() + 5.0),
        egui::Align2::LEFT_TOP,
        "RENDER STATS",
        egui::FontId::proportional(10.0),
        TEXT_MUTED,
    );
    painter.line_segment(
        [
            egui::pos2(panel.left(), panel.top() + HEADER),
            egui::pos2(panel.right(), panel.top() + HEADER),
        ],
        egui::Stroke::new(1.0_f32, BORDER),
    );
    for (i, (label, value)) in rows.iter().enumerate() {
        let y = panel.top() + HEADER + 3.0 + i as f32 * ROW;
        painter.text(
            egui::pos2(panel.left() + 8.0, y),
            egui::Align2::LEFT_TOP,
            *label,
            egui::FontId::proportional(11.0),
            TEXT_DIM,
        );
        painter.text(
            egui::pos2(panel.right() - 8.0, y),
            egui::Align2::RIGHT_TOP,
            value,
            egui::FontId::proportional(11.0),
            TEXT_BRIGHT,
        );
    }
}


/// Bytes as MB or GB, the way a stats panel reads them.
fn format_bytes(bytes: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    let mb = bytes as f64 / MB;
    if mb >= 1024.0 {
        format!("{:.2} GB", mb / 1024.0)
    } else {
        format!("{mb:.0} MB")
    }
}

/// `30588` → `30,588`, the way the prototype prints its triangle count.
fn format_thousands(n: u32) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod snap_tests {
    use super::snap_active;

    #[test]
    fn ctrl_inverts_the_preference_in_both_directions() {
        assert!(!snap_active(false, false), "off by default");
        assert!(snap_active(false, true), "Ctrl is hold-to-snap when the preference is off");
        assert!(snap_active(true, false), "on by preference");
        assert!(
            !snap_active(true, true),
            "Ctrl must SUSPEND snapping when the preference is on — an OR here would make the \
             key do nothing half the time, and nothing would notice"
        );
    }

    /// The gate the three snap settings sit behind.
    ///
    /// `transform-gizmo` reads `snap_distance`, `snap_angle` and `snap_scale` only inside
    /// `if config.snapping` (subgizmo/{translation,rotation,scale}.rs). That field was never
    /// assigned, so `..Default::default()` supplied `false` and three preference sliders plus a
    /// Ctrl modifier were computed every frame and discarded. Nothing here can drive an egui drag,
    /// so this pins the one thing a test can see: that the field is still assigned.
    #[test]
    fn the_gizmo_config_still_assigns_snapping() {
        let src = include_str!("scene_view.rs");
        let code = src.split("#[cfg(test)]").next().unwrap_or("");
        assert!(
            code.contains("snapping: snap_enabled"),
            "GizmoConfig must assign `snapping`, or the snap settings are silently inert again"
        );
        for field in ["snap_distance,", "snap_angle,", "snap_scale,"] {
            assert!(code.contains(field), "GizmoConfig no longer passes {field}");
        }
        assert!(
            code.contains("gizmo_size: state.prefs.gizmo_size"),
            "the gizmo size preference must reach GizmoVisuals"
        );
    }
}
