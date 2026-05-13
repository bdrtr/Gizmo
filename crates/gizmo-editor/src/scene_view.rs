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

    // ─── VIEWPORT BİLGİ OVERLAY'İ ───
    // FPS, entity sayısı, kamera pozisyonu gibi bilgileri sahne görünümünün
    // sol üst köşesinde yarı-şeffaf arkaplanla gösterir (Blender viewport overlay benzeri)
    {
        let painter = ui.painter();
        let overlay_margin = 8.0;
        let line_height = 14.0;

        // FPS bilgisi
        let profiler_fps = world
            .get_resource::<gizmo_core::FrameProfiler>()
            .map(|p| p.estimated_fps())
            .unwrap_or(0.0);
        let fps_color = if profiler_fps >= 55.0 {
            egui::Color32::from_rgb(80, 200, 120) // Yeşil
        } else if profiler_fps >= 30.0 {
            egui::Color32::from_rgb(240, 180, 50) // Sarı
        } else {
            egui::Color32::from_rgb(220, 60, 60) // Kırmızı
        };

        // Entity sayısı
        let entity_count = world.iter_alive_entities().len();
        let selected_count = state.selection.entities.len();

        // Kamera pozisyonu
        let cam_pos = {
            let transforms = world.borrow::<gizmo_physics::components::Transform>();
            // Editor camera'yı bulmak için en basit yol: birinci entity'yi tara
            // (state.editor_camera zaten SceneView'dan erişilemez, ama World'den çekebiliriz)
            let mut pos = gizmo_math::Vec3::ZERO;
            let cameras = world.borrow::<gizmo_renderer::components::Camera>();
            for e in world.iter_alive_entities() {
                if cameras.get(e.id()).is_some() {
                    if let Some(t) = transforms.get(e.id()) {
                        pos = t.position;
                        break;
                    }
                }
            }
            pos
        };

        let lines = [
            (format!("⚡ {:.0} FPS", profiler_fps), fps_color),
            (
                format!("📦 {} obje", entity_count),
                egui::Color32::from_rgb(180, 180, 200),
            ),
            (
                if selected_count > 0 {
                    format!("✅ {} seçili", selected_count)
                } else {
                    "✅ —".to_string()
                },
                if selected_count > 0 {
                    egui::Color32::from_rgb(237, 113, 28) // Blender turuncu
                } else {
                    egui::Color32::from_rgb(120, 120, 140)
                },
            ),
            (
                format!("📍 {:.1}, {:.1}, {:.1}", cam_pos.x, cam_pos.y, cam_pos.z),
                egui::Color32::from_rgb(140, 140, 160),
            ),
        ];

        // Arka plan kutusu
        let bg_height = lines.len() as f32 * line_height + 12.0;
        let bg_width = 165.0;
        let bg_rect = egui::Rect::from_min_size(
            egui::pos2(rect.left() + overlay_margin, rect.top() + overlay_margin),
            egui::vec2(bg_width, bg_height),
        );
        painter.rect_filled(bg_rect, 4.0, egui::Color32::from_rgba_premultiplied(20, 20, 25, 180));

        // Metin
        for (i, (text, color)) in lines.iter().enumerate() {
            painter.text(
                egui::pos2(
                    rect.left() + overlay_margin + 6.0,
                    rect.top() + overlay_margin + 6.0 + i as f32 * line_height,
                ),
                egui::Align2::LEFT_TOP,
                text,
                egui::FontId::monospace(11.0),
                *color,
            );
        }
    }

    // --- GIZMO FARE (MOUSE) ETKİLEŞİMLERİ ---
    let (hover_pos, interact_pos, _latest_pos, any_released, alt_pressed, scroll_y, _primary_down, press_origin) =
        ui.input(|i| {
            (
                i.pointer.hover_pos(),
                i.pointer.interact_pos(),
                i.pointer.latest_pos(),
                i.pointer.any_released(),
                i.modifiers.alt,
                i.raw_scroll_delta.y,
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

        if response.clicked_by(egui::PointerButton::Primary)
            || response.drag_started_by(egui::PointerButton::Primary)
        {
            state.do_raycast = true;
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
        state.camera.scroll_delta = None;
    }

    if let Some(dragged_path) = state.dragged_asset.clone() {
        if any_released {
            let latest_pos = ui.input(|i| i.pointer.latest_pos());
            let in_scene = latest_pos.map(|p| rect.contains(p)).unwrap_or(false);
            
            println!(">>> DRAG RELEASED! latest_pos: {:?}, rect: {:?}, in_scene: {}", latest_pos, rect, in_scene);
            
            if in_scene {
                state.log_info(&format!("Model sahneye bırakıldı: {}", dragged_path));
                state.spawn_asset_request = Some(dragged_path);
                state.spawn_asset_position = Some(gizmo_math::Vec3::ZERO);
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
            let mut transforms = world.borrow_mut::<gizmo_physics::components::Transform>();
            
            let primary_id = state.selection.primary.unwrap_or_else(|| *state.selection.entities.iter().next().unwrap());
            if let Some(primary_t) = transforms.get_mut(primary_id.id()) {
                
                use transform_gizmo_egui::prelude::*;
                use transform_gizmo_egui::math::Transform as GizmoTransform;

                let gizmo_orientation = if state.gizmo_local_space {
                    GizmoOrientation::Local
                } else {
                    GizmoOrientation::Global
                };

                let snap_distance = if state.prefs.snap_enabled { state.prefs.snap_translate as f32 } else { 0.0 };
                let snap_angle = if state.prefs.snap_enabled { state.prefs.snap_rotate_deg.to_radians() as f32 } else { 0.0 };

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
                    modes: GizmoMode::all(), // Simply allow all modes
                    orientation: gizmo_orientation,
                    snap_distance,
                    snap_angle,
                    ..Default::default()
                };
                state.transform_gizmo.update_config(config);

                // Gizmo transform oluştur
                let tr = primary_t.position;
                let rt = primary_t.rotation;
                let sc = primary_t.scale;
                
                let translation = transform_gizmo_egui::mint::Vector3 { x: tr.x as f64, y: tr.y as f64, z: tr.z as f64 };
                let rotation = transform_gizmo_egui::mint::Quaternion { v: transform_gizmo_egui::mint::Vector3 { x: rt.x as f64, y: rt.y as f64, z: rt.z as f64 }, s: rt.w as f64 };
                let scale = transform_gizmo_egui::mint::Vector3 { x: sc.x as f64, y: sc.y as f64, z: sc.z as f64 };
                
                let gizmo_transform = GizmoTransform::from_scale_rotation_translation(scale, rotation, translation);

                use transform_gizmo_egui::GizmoExt;
                if let Some((_result, new_transforms)) = state.transform_gizmo.interact(ui, &[gizmo_transform]) {
                    gizmo_interacted = true;
                    if let Some(new_t) = new_transforms.first() {
                        let nt: transform_gizmo_egui::mint::Vector3<f64> = new_t.translation.into();
                        let nr: transform_gizmo_egui::mint::Quaternion<f64> = new_t.rotation.into();
                        let ns: transform_gizmo_egui::mint::Vector3<f64> = new_t.scale.into();
                        
                        primary_t.position = gizmo_math::Vec3::new(nt.x as f32, nt.y as f32, nt.z as f32);
                        primary_t.rotation = gizmo_math::Quat::from_xyzw(nr.v.x as f32, nr.v.y as f32, nr.v.z as f32, nr.s as f32);
                        primary_t.scale = gizmo_math::Vec3::new(ns.x as f32, ns.y as f32, ns.z as f32);
                        primary_t.update_local_matrix();
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
            0.0,
            egui::Color32::from_white_alpha(30),
            egui::Stroke::new(1.0, egui::Color32::WHITE),
        );
    }
}
