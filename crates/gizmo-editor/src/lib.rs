//! Gizmo Editor — Egui tabanlı sahne editörü
//!
//! ## Paneller
//! - **Toolbar** — Üst çubuk: Save/Load, Play/Pause, Gizmo modu
//! - **Hierarchy** — Sol panel: Entity ağacı
//! - **Inspector** — Sağ panel: Component düzenleyici
//! - **Asset Browser** — Alt panel: Dosya gezgini

pub mod gui;
pub mod editor_state;
pub mod hierarchy;
pub mod inspector;
pub mod toolbar;
pub mod asset_browser;
pub mod history;

pub use gui::EditorContext;
pub use editor_state::{EditorState, GizmoMode, EditorMode, DragAxis, BuildTarget};

use gizmo_core::World;

use egui_dock::{DockArea, TabViewer};

pub struct EditorTabViewer<'a> {
    pub world: &'a World,
    pub state: &'a mut EditorState,
}

impl<'a> TabViewer for EditorTabViewer<'a> {
    type Tab = String;

    fn title(&mut self, tab: &mut Self::Tab) -> egui::WidgetText {
        tab.as_str().into()
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Self::Tab) {
        match tab.as_str() {
            "Hierarchy" => {
                hierarchy::ui_hierarchy(ui, self.world, self.state);
            }
            "Inspector" => {
                inspector::ui_inspector(ui, self.world, self.state);
            }
            "Asset Browser" => {
                asset_browser::ui_asset_browser(ui, self.state);
            }
            "Scene View" => {
                self.state.scene_view_visible = true;
                
                let response = ui.allocate_response(ui.available_size(), egui::Sense::click_and_drag());
                let rect = response.rect;

                self.state.scene_view_rect = Some(rect);

                if let Some(texture_id) = self.state.scene_texture_id {
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
                            ui.label(egui::RichText::new("Gizmo Scene View").color(egui::Color32::from_white_alpha(50)));
                        });
                    });
                }

                // --- GIZMO FARE (MOUSE) ETKİLEŞİMLERİ ---
                if let Some(hover_pos) = ui.input(|i| i.pointer.hover_pos()) {
                    if response.contains_pointer() || response.dragged() {
                        // Fare sahne içinde veya sürükleniyor ise NDC (-1.0 ile 1.0) hesapla
                        let nx = ((hover_pos.x - rect.left()) / rect.width()) * 2.0 - 1.0;
                        let ny = 1.0 - ((hover_pos.y - rect.top()) / rect.height()) * 2.0;

                        self.state.mouse_ndc = Some(gizmo_math::Vec2::new(nx, ny));

                        if response.clicked_by(egui::PointerButton::Primary) || response.drag_started_by(egui::PointerButton::Primary) {
                            self.state.do_raycast = true;
                        }
                        
                        // Sağ tık kamerayı çevirmek için (Egui ham input'u yuttuğu için burdan geçirmeliyiz)
                        if response.dragged_by(egui::PointerButton::Secondary) {
                            let delta = response.drag_delta();
                            self.state.camera_look_delta = Some(gizmo_math::Vec2::new(delta.x, delta.y));
                        } else {
                            self.state.camera_look_delta = None;
                        }
                        
                        // Orta tık kamerayı kaydırmak (Pan) için
                        if response.dragged_by(egui::PointerButton::Middle) {
                            let delta = response.drag_delta();
                            self.state.camera_pan_delta = Some(gizmo_math::Vec2::new(delta.x, delta.y));
                        } else {
                            self.state.camera_pan_delta = None;
                        }

                        // Alt + Sol Tık Orbit için
                        let alt_pressed = ui.input(|i| i.modifiers.alt);
                        if alt_pressed && response.dragged_by(egui::PointerButton::Primary) {
                            let delta = response.drag_delta();
                            self.state.camera_orbit_delta = Some(gizmo_math::Vec2::new(delta.x, delta.y));
                        } else {
                            self.state.camera_orbit_delta = None;
                        }
                        
                        // Scroll Zoom için
                        let scroll_y = ui.input(|i| i.raw_scroll_delta.y); // raw_scroll kullanırsak daha yumuşak gelir
                        if scroll_y.abs() > 0.0 {
                            self.state.camera_scroll_delta = Some(scroll_y);
                        } else {
                            self.state.camera_scroll_delta = None;
                        }
                        
                    } else {
                        self.state.mouse_ndc = None;
                        self.state.camera_look_delta = None;
                        self.state.camera_pan_delta = None;
                        self.state.camera_orbit_delta = None;
                        self.state.camera_scroll_delta = None;
                    }
                }

                // Fareyi bırakırsa sürükleme (axis_drag) iptal olur
                if ui.input(|i| i.pointer.any_released()) && self.state.dragging_axis.is_some() {
                    self.state.dragging_axis = None;
                }

                // Dışarıdan veya UI'dan sürüklenen objeyi Scene View'a bırakma yakakalayıcısı
                if let Some(dragged_path) = ui.memory(|m| m.data.get_temp::<String>(egui::Id::new("dragged_asset_path"))) {
                    if response.hovered() && ui.input(|i| i.pointer.any_released()) {
                        self.state.spawn_asset_request = Some(dragged_path);
                        
                        // Farenin bırakıldığı yerin NDC koordinatı ile objeyi spawner'a gönderelim (sıfır değil)
                        // Asset Drop Raycasting (Aşama 2): 
                        if let Some(ndc) = self.state.mouse_ndc {
                            // Biz şimdilik NDC'yi direkt pozisyon olarak veriyoruz (bunu main.rs raycast'e dönüştürecek)
                            self.state.spawn_asset_position = Some(gizmo_math::Vec3::new(ndc.x, ndc.y, 1.0)); 
                        } else {
                            self.state.spawn_asset_position = Some(gizmo_math::Vec3::ZERO); 
                        }
                        
                        ui.memory_mut(|m| m.data.remove::<String>(egui::Id::new("dragged_asset_path")));
                    }
                }

                // --- EGUI-GIZMO Entegrasyonu (Aşama 1) ---
                if let (Some(view_mat), Some(proj_mat)) = (self.state.camera_view, self.state.camera_proj) {
                    if !self.state.selected_entities.is_empty() {
                        if let Some(mut transforms) = self.world.borrow_mut::<gizmo_physics::components::Transform>() {
                            let primary_id = *self.state.selected_entities.iter().next().unwrap();
                            let mut primary_model_mat = gizmo_math::Mat4::IDENTITY;
                            if let Some(primary_t) = transforms.get(primary_id) {
                                primary_model_mat = primary_t.model_matrix();
                            }
                                
                            let gizmo_mode = match self.state.gizmo_mode {
                                crate::editor_state::GizmoMode::Translate => egui_gizmo::GizmoMode::Translate,
                                crate::editor_state::GizmoMode::Rotate => egui_gizmo::GizmoMode::Rotate,
                                crate::editor_state::GizmoMode::Scale => egui_gizmo::GizmoMode::Scale,
                            };

                            let gizmo_orientation = if self.state.gizmo_local_space {
                                egui_gizmo::GizmoOrientation::Local
                            } else {
                                egui_gizmo::GizmoOrientation::Global
                            };

                            let snap_enabled = ui.input(|i| i.modifiers.command); // Ctrl (Windows/Linux) veya Cmd (Mac)
                            let snap_distance = 0.5;
                            let snap_angle = 15.0_f32.to_radians();

                            let gizmo = egui_gizmo::Gizmo::new("scene_gizmo")
                                .view_matrix(view_mat.to_cols_array_2d().into())
                                .projection_matrix(proj_mat.to_cols_array_2d().into())
                                .model_matrix(primary_model_mat.to_cols_array_2d().into())
                                .mode(gizmo_mode)
                                .orientation(gizmo_orientation)
                                .snapping(snap_enabled)
                                .snap_distance(snap_distance)
                                .snap_angle(snap_angle);

                            if let Some(result) = gizmo.interact(ui) {
                                if self.state.gizmo_original_transforms.is_empty() {
                                    // Tüm seçili objelerin orijinal durumlarını kaydet
                                    for &id in self.state.selected_entities.iter() {
                                        if let Some(tx) = transforms.get(id) {
                                            self.state.gizmo_original_transforms.insert(id, *tx);
                                        }
                                    }
                                }

                                if let Some(orig_pivot) = self.state.gizmo_original_transforms.get(&primary_id) {
                                    let new_mat = gizmo_math::Mat4::from_cols_array_2d(&result.transform().into());
                                    let delta_mat = new_mat * orig_pivot.model_matrix().inverse();
                                    
                                    for &id in self.state.selected_entities.iter() {
                                        if let Some(orig_t) = self.state.gizmo_original_transforms.get(&id) {
                                            if let Some(t) = transforms.get_mut(id) {
                                                let final_mat = delta_mat * orig_t.model_matrix();
                                                let (scale, rot, pos) = final_mat.to_scale_rotation_translation();
                                                t.position = pos;
                                                t.rotation = rot;
                                                t.scale = scale;
                                                t.update_local_matrix();
                                            }
                                        }
                                    }
                                }
                            } else if !self.state.gizmo_original_transforms.is_empty() {
                                // Sürükleme bittiğinde değişimi History'e aktar
                                let mut changes = Vec::new();
                                for &id in self.state.selected_entities.iter() {
                                    if let Some(old_t) = self.state.gizmo_original_transforms.get(&id) {
                                        if let Some(t) = transforms.get(id) {
                                            if old_t.position != t.position || old_t.rotation != t.rotation || old_t.scale != t.scale {
                                                changes.push((id, *old_t, *t));
                                            }
                                        }
                                    }
                                }
                                
                                if !changes.is_empty() {
                                    self.state.history.push(crate::history::EditorAction::TransformsChanged {
                                        changes,
                                    });
                                }
                                self.state.gizmo_original_transforms.clear();
                            }
                        }
                    }
                }
            }
            "Game View" => {
                self.state.game_view_visible = true;
                let is_playing = self.state.is_playing();
                let is_paused = self.state.mode == crate::editor_state::EditorMode::Paused;

                if is_playing || is_paused {
                    let rect = ui.available_rect_before_wrap();
                    if let Some(tex_id) = self.state.scene_texture_id {
                        let mut mesh = egui::Mesh::with_texture(tex_id);
                        mesh.add_rect_with_uv(
                            rect,
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            egui::Color32::WHITE,
                        );
                        ui.painter().add(mesh);
                        
                        if is_paused {
                            ui.allocate_ui_at_rect(rect, |ui| {
                                ui.centered_and_justified(|ui| {
                                    ui.label(
                                        egui::RichText::new("⏸ DURAKLATILDI")
                                            .size(40.0)
                                            .color(egui::Color32::from_white_alpha(150))
                                            .strong(),
                                    );
                                });
                            });
                        }
                    }
                } else {
                    ui.vertical_centered(|ui| {
                        ui.add_space(30.0);
                        ui.label(
                            egui::RichText::new("▶ Oyunu Başlat")
                                .size(26.0)
                                .color(egui::Color32::from_white_alpha(60)),
                        );
                        ui.add_space(8.0);
                        ui.label(
                            egui::RichText::new("Toolbar'daki ▶ Başlat butonuna\nbasarak simülasyonu çalıştırın.")
                                .size(14.0)
                                .color(egui::Color32::from_white_alpha(40)),
                        );
                    });

                    ui.add_space(20.0);
                    ui.separator();
                    ui.add_space(10.0);

                    ui.label(egui::RichText::new("📋 Editör Kısayolları").strong());
                    ui.add_space(6.0);

                    let shortcuts = [
                        ("W / A / S / D", "Kamerayı hareket ettir"),
                        ("Sağ Tık + Sürükle", "Kamerayı döndür"),
                        ("Orta Tık + Sürükle", "Kamerayı kaydır (pan)"),
                        ("Scroll", "Yakınlaştır / uzaklaştır"),
                        ("F", "Seçili objeye odaklan"),
                        ("Delete", "Seçili objeyi sil"),
                        ("Ctrl + Z / Y", "Geri al / İleri al"),
                        ("Ctrl + D", "Seçili objeyi çoğalt"),
                        ("Ctrl + Sürükle (Gizmo)", "Snap ile hareket"),
                    ];

                    egui::Grid::new("shortcut_grid")
                        .num_columns(2)
                        .spacing([16.0, 4.0])
                        .show(ui, |ui| {
                            for (key, desc) in &shortcuts {
                                ui.label(egui::RichText::new(*key).monospace().color(egui::Color32::from_rgb(200, 200, 100)));
                                ui.label(egui::RichText::new(*desc).color(egui::Color32::GRAY));
                                ui.end_row();
                            }
                        });
                }
            }

            "Console" => {
                ui.heading("Geliştirici Konsolu");
                ui.separator();
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        if let Ok(logs) = gizmo_core::logger::GLOBAL_LOGS.lock() {
                            for log in logs.iter() {
                                let color = match log.level {
                                    gizmo_core::logger::LogLevel::Info => egui::Color32::WHITE,
                                    gizmo_core::logger::LogLevel::Warning => egui::Color32::from_rgb(255, 200, 0),
                                    gizmo_core::logger::LogLevel::Error => egui::Color32::RED,
                                };
                                ui.label(egui::RichText::new(&log.message).color(color));
                            }
                        }
                    });
            }
            _ => {
                ui.label(format!("Bilinmeyen Tab: {}", tab));
            }
        }
    }
}

/// Tüm editör panellerini tek çağrıyla çizer
pub fn draw_editor(ctx: &egui::Context, world: &World, state: &mut EditorState) {
    // 1. Toolbar (en üstte kalmaya devam etmeli, dock'un dışında)
    if state.show_toolbar {
        toolbar::draw_toolbar(ctx, state);
    }

    // Editör çiziminden hemen önce kamera çizim durumlarını resetleyelim
    state.scene_view_visible = false;
    state.game_view_visible = false;

    // 2. Docking Alanı (Geri kalan tüm alanı kaplar)
    let mut viewer = EditorTabViewer {
        world,
        state: unsafe { &mut *(state as *mut _) },
    };

    DockArea::new(&mut state.dock_state)
        .style(egui_dock::Style::from_egui(ctx.style().as_ref()))
        .show(ctx, &mut viewer);

    // 3. Build Konsolu (Yüzücü Pencere)
    let is_building = state.is_building.load(std::sync::atomic::Ordering::SeqCst);
    let mut logs_lock = state.build_logs.lock().unwrap();
    if is_building || !logs_lock.is_empty() {
        egui::Window::new("🚀 Build (Derleme) Konsolu")
            .default_size([520.0, 420.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(360.0)
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for log in logs_lock.iter() {
                            let color = if log.contains("HATA") || log.contains("error:") || log.contains("❌") {
                                egui::Color32::RED
                            } else if log.contains("Başarılı") || log.contains("TAMAMLANDI") || log.contains("🎉") {
                                egui::Color32::GREEN
                            } else if log.contains("⚠") {
                                egui::Color32::YELLOW
                            } else {
                                egui::Color32::LIGHT_GRAY
                            };
                            ui.label(egui::RichText::new(log).family(egui::FontFamily::Monospace).color(color));
                        }
                    });

                ui.separator();
                if !is_building && !logs_lock.is_empty() {
                    if ui.button(egui::RichText::new("✖ Kapat").strong()).clicked() {
                        logs_lock.clear();
                    }
                } else if is_building {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new());
                        ui.label(egui::RichText::new("Derleniyor, lütfen bekleyin...").strong().color(egui::Color32::YELLOW));
                    });
                }
            });
    }
    drop(logs_lock);

    // 4. Ayarlar Penceresi (Yüzücü)
    if state.settings_open {
        let mut open = state.settings_open;
        egui::Window::new("⚙️ Editör Ayarları")
            .open(&mut open)
            .default_size([320.0, 420.0])
            .resizable(true)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    // --- Kamera ---
                    egui::CollapsingHeader::new("🎥 Kamera")
                        .default_open(true)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label("Hareket Hızı:");
                                ui.add(egui::Slider::new(&mut state.camera_speed, 1.0..=100.0).suffix(" m/s"));
                            });
                        });

                    ui.separator();

                    // --- Snap ---
                    egui::CollapsingHeader::new("🔧 Snap (Izgara Kilitleme)")
                        .default_open(true)
                        .show(ui, |ui| {
                            ui.label(egui::RichText::new("Ctrl basılıyken aktif olur.").weak().small());
                            ui.horizontal(|ui| {
                                ui.label("Taşıma:");
                                ui.add(egui::DragValue::new(&mut state.snap_translate).speed(0.05).clamp_range(0.01..=10.0).suffix(" m"));
                            });
                            ui.horizontal(|ui| {
                                ui.label("Döndürme:");
                                ui.add(egui::DragValue::new(&mut state.snap_rotate_deg).speed(1.0).clamp_range(1.0..=90.0).suffix("°"));
                            });
                            ui.horizontal(|ui| {
                                ui.label("Ölçekleme:");
                                ui.add(egui::DragValue::new(&mut state.snap_scale).speed(0.01).clamp_range(0.01..=5.0));
                            });
                        });

                    ui.separator();

                    // --- Grid ---
                    egui::CollapsingHeader::new("📐 Grid & Görünüm")
                        .default_open(true)
                        .show(ui, |ui| {
                            ui.checkbox(&mut state.show_grid, "Grid Çizgilerini Göster");
                            ui.checkbox(&mut state.gizmo_local_space, "Varsayılan Local Space");
                        });

                    ui.separator();

                    // --- Düzeni Yönet ---
                    egui::CollapsingHeader::new("🪟 Panel Düzeni")
                        .default_open(false)
                        .show(ui, |ui| {
                            if ui.button("💾 Mevcut Düzeni Kaydet").clicked() {
                                state.save_layout();
                            }
                            if ui.button("♻ Varsayılan Düzene Dön").clicked() {
                                state.reset_layout();
                            }
                        });

                    ui.separator();

                    // --- Hakkında ---
                    egui::CollapsingHeader::new("ℹ️ Hakkında")
                        .default_open(false)
                        .show(ui, |ui| {
                            ui.label("Gizmo Engine — Editör");
                            ui.label(egui::RichText::new("Rust + WGPU + Egui").weak().small());
                            ui.hyperlink_to("GitHub", "https://github.com");
                        });
                });
            });
        state.settings_open = open;
    }
}

