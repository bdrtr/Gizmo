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
    let axis_gizmo_hot = draw_axis_gizmo(
        ui,
        state,
        rect,
        response.clicked_by(egui::PointerButton::Primary),
    );

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
        && !axis_gizmo_hot
        && (response.clicked_by(egui::PointerButton::Primary)
            || response.drag_started_by(egui::PointerButton::Primary))
        {
            tracing::info!("SceneView CLICKED/DRAG_STARTED! ndc: {:?}", state.mouse_ndc);
            state.do_raycast = true;
        }
    if !is_dragging_gizmo && !axis_gizmo_hot && response.dragged_by(egui::PointerButton::Primary) {
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


/// Arm length of the axis gizmo, centre to handle centre.
const AXIS_ARM: f32 = 26.0;
/// Radius of an axis handle.
const AXIS_HANDLE: f32 = 7.0;
/// Half-width of the whole widget: the arm, its handle, and a little air.
const AXIS_HALF: f32 = AXIS_ARM + AXIS_HANDLE + 4.0;

/// One drawable end of a world axis, already projected into the widget.
struct AxisHandle {
    /// Where the camera should end up looking if this handle is clicked.
    look_dir: gizmo_math::Vec3,
    centre: egui::Pos2,
    /// View-space z of the axis direction. The view matrix is right-handed and the camera looks
    /// down its own -Z, so a **larger** z means the axis points more towards the viewer.
    depth: f32,
    color: egui::Color32,
    label: &'static str,
    positive: bool,
}

/// The six axis ends, projected through `view` and sorted **back to front** — so drawing them in
/// order gives the right occlusion, and walking the slice backwards gives the right hit test.
///
/// Split out from the drawing because every decision in it is a sign that fails silently: a
/// forgotten y-flip puts +Y at the bottom, a flipped depth hides the near handle behind the far
/// one, and negating the wrong vector sends the camera to the far side of the scene.
fn axis_handles(view: gizmo_math::Mat4, centre: egui::Pos2) -> Vec<AxisHandle> {
    let visuals = transform_gizmo_egui::GizmoVisuals::default();
    let mut handles = Vec::with_capacity(6);
    for (axis, label, color) in [
        (gizmo_math::Vec3::X, "X", visuals.x_color),
        (gizmo_math::Vec3::Y, "Y", visuals.y_color),
        (gizmo_math::Vec3::Z, "Z", visuals.z_color),
    ] {
        for sign in [1.0_f32, -1.0] {
            let world = axis * sign;
            let v = view.transform_vector3(world);
            handles.push(AxisHandle {
                // Clicking the near end of an axis means standing on that side, so the camera
                // looks back along it.
                look_dir: -world,
                // View-space +Y is up and screen +y is down, hence the flip on y only.
                centre: centre + egui::vec2(v.x * AXIS_ARM, -v.y * AXIS_ARM),
                depth: v.z,
                color,
                label,
                positive: sign > 0.0,
            });
        }
    }
    handles.sort_by(|a, b| a.depth.total_cmp(&b.depth));
    handles
}

/// The prototype's viewport axis gizmo, top-right corner.
///
/// # Why it is drawn from the view matrix
///
/// The three world axes are pushed through `state.camera.view` — the same matrix the renderer drew
/// the frame with — rather than rebuilt from yaw/pitch. So it cannot disagree with what is on
/// screen: if the corner says +Z points left, +Z points left. Nothing here needs to know the
/// camera's Euler convention, which is exactly the thing that gets a sign wrong.
///
/// # Why these three colours
///
/// They are read off `GizmoVisuals::default()` — the live colours of the transform handles in the
/// middle of the viewport. Hardcoding a red/green/blue triple here would have made the editor claim
/// X is red in one corner and pink in the middle of the same frame.
///
/// # Interaction
///
/// A click on a handle asks for a *direction*, not a camera placement: the camera keeps its
/// distance and its pivot, and only turns. The near end (`+X`) is the side you end up standing on,
/// so clicking it looks along `-X`, the way every editor's view cube behaves.
///
/// Returns `true` while the pointer belongs to this widget, so the caller can keep the same click
/// from also falling through to selection or a rubber band.
fn draw_axis_gizmo(
    ui: &egui::Ui,
    state: &mut EditorState,
    rect: egui::Rect,
    primary_clicked: bool,
) -> bool {
    use crate::theme::palette::*;

    let Some(view) = state.camera.view else {
        return false; // no camera measured yet — draw nothing rather than a plausible cube
    };
    if rect.width() < AXIS_HALF * 4.0 || rect.height() < AXIS_HALF * 4.0 {
        return false; // too small a viewport to spend a corner on
    }

    let centre = egui::pos2(
        rect.right() - 8.0 - AXIS_HALF,
        rect.top() + 8.0 + AXIS_HALF,
    );
    let handles = axis_handles(view, centre);

    let (hover_pos, press_origin) =
        ui.input(|i| (i.pointer.hover_pos(), i.pointer.press_origin()));

    // Front-most handle wins the hover: the draw order is back-to-front, so the hit test walks it
    // backwards. Without this, an axis hidden behind another would still take the click.
    let hovered = hover_pos.and_then(|p| {
        handles
            .iter()
            .rposition(|h| h.centre.distance(p) <= AXIS_HANDLE + 2.0)
    });

    let painter = ui.painter();
    painter.circle_filled(centre, AXIS_HALF, CHROME.gamma_multiply(0.55));
    painter.circle_stroke(centre, AXIS_HALF, egui::Stroke::new(1.0_f32, BORDER));

    for (i, h) in handles.iter().enumerate() {
        let hot = hovered == Some(i);
        if h.positive {
            // Only the positive ends get a stem. Six spokes would be a wheel, not an axis triad.
            painter.line_segment(
                [centre, h.centre],
                egui::Stroke::new(2.0_f32, h.color.gamma_multiply(if hot { 1.0 } else { 0.8 })),
            );
            painter.circle_filled(h.centre, AXIS_HANDLE, h.color);
            painter.text(
                h.centre,
                egui::Align2::CENTER_CENTER,
                h.label,
                egui::FontId::proportional(9.0),
                VOID,
            );
        } else {
            // Hollow, the way every view cube marks the far end: same hue, no fill, and the letter
            // only while hovered — otherwise six letters compete in a 74 px circle.
            painter.circle_filled(h.centre, AXIS_HANDLE, VOID.gamma_multiply(0.75));
            painter.circle_stroke(
                h.centre,
                AXIS_HANDLE,
                egui::Stroke::new(1.5_f32, h.color.gamma_multiply(if hot { 1.0 } else { 0.7 })),
            );
            if hot {
                painter.text(
                    h.centre,
                    egui::Align2::CENTER_CENTER,
                    h.label,
                    egui::FontId::proportional(9.0),
                    h.color,
                );
            }
        }
        if hot {
            painter.circle_stroke(
                h.centre,
                AXIS_HANDLE + 2.5,
                egui::Stroke::new(1.0_f32, TEXT_BRIGHT),
            );
        }
    }

    if let Some(i) = hovered {
        if primary_clicked {
            state.camera.view_request = Some(handles[i].look_dir);
        }
    }

    // The widget owns its whole disc for the duration of a press, not just the frames where a
    // handle is under the cursor: a press that starts here and drags off must not leave a rubber
    // band behind it.
    hovered.is_some()
        || press_origin.is_some_and(|p| p.distance(centre) <= AXIS_HALF)
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
mod axis_gizmo_tests {
    use super::{axis_handles, AXIS_ARM};

    /// A camera at the origin looking down world -Z with +Y up — the plainest view there is, and
    /// the one where every projected axis has an answer you can name without doing any arithmetic.
    fn plain_view() -> gizmo_math::Mat4 {
        gizmo_math::Mat4::look_at_rh(
            gizmo_math::Vec3::ZERO,
            gizmo_math::Vec3::NEG_Z,
            gizmo_math::Vec3::Y,
        )
    }

    fn handle(view: gizmo_math::Mat4, label: &str, positive: bool) -> super::AxisHandle {
        let centre = egui::pos2(100.0, 100.0);
        axis_handles(view, centre)
            .into_iter()
            .find(|h| h.label == label && h.positive == positive)
            .expect("all six ends are always produced")
    }

    /// Screen y grows downward and view-space y grows upward. Getting this wrong draws a gizmo
    /// that is upside down but otherwise entirely plausible.
    #[test]
    fn up_is_up_and_right_is_right() {
        let v = plain_view();
        let centre = egui::pos2(100.0, 100.0);

        let up = handle(v, "Y", true);
        assert!(
            up.centre.y < centre.y - AXIS_ARM * 0.9,
            "+Y must be drawn ABOVE the centre, got {:?}",
            up.centre
        );
        let right = handle(v, "X", true);
        assert!(
            right.centre.x > centre.x + AXIS_ARM * 0.9,
            "+X must be drawn to the RIGHT for a camera looking down -Z, got {:?}",
            right.centre
        );
    }

    /// The view matrix is right-handed and the camera looks down its own -Z, so an axis pointing
    /// at the viewer has a POSITIVE view-space z. Flip this and the far handles paint over the
    /// near ones, and the hit test hands clicks to the axis you cannot see.
    #[test]
    fn the_axis_pointing_at_the_viewer_is_the_front_most() {
        let v = plain_view();
        let towards_viewer = handle(v, "Z", true); // world +Z, i.e. behind this camera
        let away = handle(v, "Z", false);
        assert!(
            towards_viewer.depth > away.depth,
            "+Z points at a camera that looks down -Z: {} should exceed {}",
            towards_viewer.depth,
            away.depth
        );

        let sorted = axis_handles(v, egui::pos2(100.0, 100.0));
        assert_eq!(
            sorted.last().map(|h| (h.label, h.positive)),
            Some(("Z", true)),
            "back-to-front order must end on the handle nearest the viewer"
        );
    }

    /// Clicking the near end of an axis puts the camera on that side, which means looking back
    /// along it. Dropping the negation aims the camera away from the scene — at nothing.
    #[test]
    fn a_handle_looks_back_along_its_own_axis() {
        let v = plain_view();
        for (label, positive, expected) in [
            ("X", true, gizmo_math::Vec3::NEG_X),
            ("X", false, gizmo_math::Vec3::X),
            ("Y", true, gizmo_math::Vec3::NEG_Y),
            ("Z", true, gizmo_math::Vec3::NEG_Z),
        ] {
            let h = handle(v, label, positive);
            assert!(
                (h.look_dir - expected).length() < 1e-6,
                "the {label} handle (positive={positive}) must look along {expected}, got {}",
                h.look_dir
            );
        }
    }

    /// Where the camera *is* must not move the widget. The axes are directions, so only the view
    /// matrix's rotation may touch them; if the eye position leaked in — a `transform_point3` where
    /// a `transform_vector3` belongs — the whole triad slides off into the corner of the screen.
    ///
    /// Measured as: the handle's offset and its depth are two legs of a unit vector. On screen the
    /// offset alone is shorter than the arm (an axis pointing away from you is drawn short, which
    /// is the foreshortening that makes the widget readable), so the arm circle is a bound, not an
    /// equality — the length that stays fixed is the 3D one.
    #[test]
    fn the_camera_position_does_not_move_the_widget() {
        // Two cameras with the same orientation, parked a long way apart.
        let near = gizmo_math::Mat4::look_at_rh(
            gizmo_math::Vec3::new(0.0, 0.0, 5.0),
            gizmo_math::Vec3::ZERO,
            gizmo_math::Vec3::Y,
        );
        let far = gizmo_math::Mat4::look_at_rh(
            gizmo_math::Vec3::new(0.0, 0.0, 900.0),
            gizmo_math::Vec3::new(0.0, 0.0, 895.0),
            gizmo_math::Vec3::Y,
        );
        let centre = egui::pos2(100.0, 100.0);
        let a = axis_handles(near, centre);
        let b = axis_handles(far, centre);
        for (ha, hb) in a.iter().zip(b.iter()) {
            assert!(
                ha.centre.distance(hb.centre) < 1e-3,
                "{}{} moved with the camera: {:?} vs {:?}",
                if ha.positive { "+" } else { "-" },
                ha.label,
                ha.centre,
                hb.centre
            );
            let offset = ha.centre - centre;
            let len3 = ((offset.x / AXIS_ARM).powi(2)
                + (offset.y / AXIS_ARM).powi(2)
                + ha.depth.powi(2))
            .sqrt();
            assert!(
                (len3 - 1.0).abs() < 1e-4,
                "a projected unit axis must stay unit length, got {len3}"
            );
            assert!(
                offset.length() <= AXIS_ARM + 1e-4,
                "no handle may be drawn outside the arm circle"
            );
        }
    }

    /// The three colours must be the ones the transform handles in the middle of the viewport are
    /// drawn with. A local red/green/blue triple would have the editor calling X two colours in
    /// one frame.
    #[test]
    fn the_colours_come_from_the_transform_gizmo() {
        let visuals = transform_gizmo_egui::GizmoVisuals::default();
        let v = plain_view();
        assert_eq!(handle(v, "X", true).color, visuals.x_color);
        assert_eq!(handle(v, "Y", true).color, visuals.y_color);
        assert_eq!(handle(v, "Z", true).color, visuals.z_color);
        // ...and both ends of an axis share it.
        assert_eq!(handle(v, "X", false).color, visuals.x_color);
    }
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
