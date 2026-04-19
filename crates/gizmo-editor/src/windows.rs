use crate::EditorState;

pub fn ui_build_console(ctx: &egui::Context, state: &mut EditorState) {
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
                            let color = if log.contains("HATA")
                                || log.contains("error:")
                                || log.contains("❌")
                            {
                                egui::Color32::RED
                            } else if log.contains("Başarılı")
                                || log.contains("TAMAMLANDI")
                                || log.contains("🎉")
                            {
                                egui::Color32::GREEN
                            } else if log.contains("⚠") {
                                egui::Color32::YELLOW
                            } else {
                                egui::Color32::LIGHT_GRAY
                            };
                            ui.label(
                                egui::RichText::new(log)
                                    .family(egui::FontFamily::Monospace)
                                    .color(color),
                            );
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
                        ui.label(
                            egui::RichText::new("Derleniyor, lütfen bekleyin...")
                                .strong()
                                .color(egui::Color32::YELLOW),
                        );
                    });
                }
            });
    }
}

pub fn ui_settings_window(ctx: &egui::Context, state: &mut EditorState) {
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
                                if ui.add(
                                    egui::Slider::new(&mut state.prefs.camera_speed, 1.0..=100.0)
                                        .suffix(" m/s"),
                                ).changed() {
                                    state.prefs.save();
                                }
                            });
                        });

                    ui.separator();

                    // --- Snap ---
                    egui::CollapsingHeader::new("🔧 Snap (Izgara Kilitleme)")
                        .default_open(true)
                        .show(ui, |ui| {
                            ui.label(
                                egui::RichText::new("Ctrl basılıyken aktif olur.")
                                    .weak()
                                    .small(),
                            );
                            ui.horizontal(|ui| {
                                ui.label("Taşıma:");
                                if ui.add(egui::DragValue::new(&mut state.prefs.snap_translate).speed(0.05).clamp_range(0.01..=10.0).suffix(" m")).changed() {
                                    state.prefs.save();
                                }
                            });
                            ui.horizontal(|ui| {
                                ui.label("Döndürme:");
                                if ui.add(egui::DragValue::new(&mut state.prefs.snap_rotate_deg).speed(1.0).clamp_range(1.0..=90.0).suffix("°")).changed() {
                                    state.prefs.save();
                                }
                            });
                            ui.horizontal(|ui| {
                                ui.label("Ölçekleme:");
                                if ui.add(egui::DragValue::new(&mut state.prefs.snap_scale).speed(0.01).clamp_range(0.01..=5.0)).changed() {
                                    state.prefs.save();
                                }
                            });
                        });

                    ui.separator();

                    // --- Grid ---
                    egui::CollapsingHeader::new("📐 Grid & Görünüm")
                        .default_open(true)
                        .show(ui, |ui| {
                            if ui.checkbox(&mut state.prefs.show_grid, "Grid Çizgilerini Göster").changed() {
                                state.prefs.save();
                            }
                            ui.horizontal(|ui| {
                                ui.label("Gizmo Boyutu:");
                                if ui.add(egui::Slider::new(&mut state.prefs.gizmo_size, 20.0..=200.0)).changed() {
                                    state.prefs.save();
                                }
                            });
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

pub fn ui_script_editor(ctx: &egui::Context, state: &mut EditorState) {
    if state.script_editor_open {
        let mut open = state.script_editor_open;
        
        let path = state.active_script_path.clone();
        
        egui::Window::new(format!("📝 Script Editor - {}", path))
            .open(&mut open)
            .default_size([600.0, 500.0])
            .resizable(true)
            .show(ctx, |ui| {
                // Toolbar
                ui.horizontal(|ui| {
                    if ui.button("💾 Kaydet").clicked()
                        && !path.is_empty() {
                            if let Err(e) = std::fs::write(&path, &state.active_script_content) {
                                state.log_error(&format!("Script kaydedilemedi: {}", e));
                            } else {
                                state.log_info(&format!("Script kaydedildi: {}", path));
                            }
                        }
                    if ui.button("❌ Kapat").clicked() {
                        state.script_editor_open = false;
                    }
                });

                ui.separator();

                // Code Editor Alanı
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.add_sized(
                            ui.available_size(),
                            egui::TextEdit::multiline(&mut state.active_script_content)
                                .font(egui::TextStyle::Monospace) // Monospace font
                                .code_editor()
                                .lock_focus(true)
                                .desired_width(f32::INFINITY),
                        );
                    });
            });
            
        state.script_editor_open = open;
    }
}
