use crate::EditorState;

pub fn ui_build_console(ui: &mut egui::Ui, state: &mut EditorState) {
    let is_building = state
        .build
        .is_building
        .load(std::sync::atomic::Ordering::Acquire);

    // Asenkron logları topla (Thread-safe)
    if let Some(rx) = &state.build.logs_rx {
        while let Ok(log) = rx.lock().unwrap().try_recv() {
            let lower_log = log.to_lowercase();
            let color = if lower_log.starts_with("error")
                || lower_log.contains("error[")
                || lower_log.contains("❌")
                || lower_log.contains("hata")
            {
                egui::Color32::RED
            } else if lower_log.contains("başarılı")
                || lower_log.contains("tamamlandı")
                || lower_log.contains("🎉")
            {
                egui::Color32::GREEN
            } else if lower_log.starts_with("warning") || lower_log.contains("⚠") {
                egui::Color32::YELLOW
            } else {
                egui::Color32::LIGHT_GRAY
            };
            state.build.cached_logs.push((log, color));
        }
    }

    let row_height = ui.text_style_height(&egui::TextStyle::Monospace);
    egui::ScrollArea::vertical().max_height(360.0).show_rows(
        ui,
        row_height,
        state.build.cached_logs.len(),
        |ui, row_range| {
            for (log, color) in &state.build.cached_logs[row_range] {
                ui.label(
                    egui::RichText::new(log)
                        .family(egui::FontFamily::Monospace)
                        .color(*color),
                );
            }
            if is_building {
                ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
            }
        },
    );

    ui.separator();
    if !is_building && !state.build.cached_logs.is_empty() {
        if ui
            .button(egui::RichText::new("✖ Konsolu Temizle").strong())
            .clicked()
        {
            state.build.cached_logs.clear();
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
}

pub fn ui_settings_window(ui: &mut egui::Ui, state: &mut EditorState) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        // --- Kamera ---
        egui::CollapsingHeader::new("🎥 Kamera")
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Hareket Hızı:");
                    if ui
                        .add(
                            egui::Slider::new(&mut state.prefs.camera_speed, 1.0..=100.0)
                                .suffix(" m/s"),
                        )
                        .drag_stopped()
                    {
                        state.prefs.mark_dirty();
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
                    if ui
                        .add(
                            egui::DragValue::new(&mut state.prefs.snap_translate)
                                .speed(0.05)
                                .range(0.01..=10.0)
                                .suffix(" m"),
                        )
                        .drag_stopped()
                    {
                        state.prefs.mark_dirty();
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Döndürme:");
                    if ui
                        .add(
                            egui::DragValue::new(&mut state.prefs.snap_rotate_deg)
                                .speed(1.0)
                                .range(1.0..=90.0)
                                .suffix("°"),
                        )
                        .drag_stopped()
                    {
                        state.prefs.mark_dirty();
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Ölçekleme:");
                    if ui
                        .add(
                            egui::DragValue::new(&mut state.prefs.snap_scale)
                                .speed(0.01)
                                .range(0.01..=5.0)
                                .suffix(" x"),
                        )
                        .drag_stopped()
                    {
                        state.prefs.mark_dirty();
                    }
                });
            });

        ui.separator();

        // --- Grid ---
        egui::CollapsingHeader::new("📐 Grid & Görünüm")
            .default_open(true)
            .show(ui, |ui| {
                if ui
                    .checkbox(&mut state.prefs.show_grid, "Grid Çizgilerini Göster")
                    .changed()
                {
                    state.prefs.mark_dirty();
                }
                ui.horizontal(|ui| {
                    ui.label("Gizmo Boyutu:");
                    if ui
                        .add(egui::Slider::new(&mut state.prefs.gizmo_size, 20.0..=200.0))
                        .drag_stopped()
                    {
                        state.prefs.mark_dirty();
                    }
                });
            });

        ui.separator();

        // --- Rendering ---
        egui::CollapsingHeader::new("🎨 Rendering & Post-Processing")
            .default_open(true)
            .show(ui, |ui| {
                ui.checkbox(&mut state.fxaa_enabled, "FXAA Anti-Aliasing");
                ui.label(
                    egui::RichText::new("Kenar yumuşatma — düşük GPU maliyeti ile pürüzsüz kenarlar.")
                        .weak()
                        .small(),
                );
                
                ui.separator();
                ui.heading("Bloom (Parlama)");
                ui.add(egui::Slider::new(&mut state.bloom_intensity, 0.0..=3.0).text("Şiddet (Intensity)"));
                ui.add(egui::Slider::new(&mut state.bloom_threshold, 0.0..=2.0).text("Eşik (Threshold)"));
                
                ui.separator();
                ui.heading("Tonemapping (Renk Filtreleri)");
                ui.add(egui::Slider::new(&mut state.exposure, 0.1..=5.0).text("Pozlama (Exposure)"));
                ui.add(egui::Slider::new(&mut state.vignette, 0.0..=1.0).text("Vignette (Kenar Karartması)"));
                ui.add(egui::Slider::new(&mut state.chromatic_aberration, 0.0..=0.05).text("Chromatic Aberration"));
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
                ui.hyperlink_to(
                    "GitHub - Gizmo Engine",
                    "https://github.com/Gizmo-engine/Gizmo",
                );
            });
    });
}

pub fn ui_script_editor(ui: &mut egui::Ui, state: &mut EditorState) {
    let has_script = state.script.active_path.is_some();
    let current_path = state.script.active_path.clone();

    ui.horizontal(|ui| {
        if let Some(path) = current_path {
            ui.label(format!("Düzenleniyor: {}", path));
            if ui.button("💾 Kaydet").clicked() {
                if let Err(e) =
                    std::fs::write(&path, state.script.active_content.as_deref().unwrap_or(""))
                {
                    state.log_error(&format!("Script kaydedilemedi: {}", e));
                } else {
                    state.log_info(&format!("Script kaydedildi: {}", path));
                    state.script.is_dirty = false;
                }
            }
        } else {
            ui.label("Yeni Script");
        }

        if state.script.pending_clear_confirm {
            let confirm_bg = egui::Color32::from_rgb(200, 50, 50);
            if ui
                .add(
                    egui::Button::new(
                        egui::RichText::new("⚠️ Onayla: Sil").color(egui::Color32::WHITE),
                    )
                    .fill(confirm_bg),
                )
                .clicked()
            {
                state.script.active_content = None;
                state.script.active_path = None;
                state.script.is_dirty = false;
                state.script.pending_clear_confirm = false;
            }
        } else {
            let clear_text = if state.script.is_dirty {
                "❌ İptal/Sil"
            } else {
                "❌ Temizle"
            };
            let clear_bg = if state.script.is_dirty {
                egui::Color32::from_rgb(150, 50, 50)
            } else {
                ui.visuals().widgets.inactive.bg_fill
            };
            if ui
                .add(
                    egui::Button::new(egui::RichText::new(clear_text).color(egui::Color32::WHITE))
                        .fill(clear_bg),
                )
                .clicked()
            {
                if state.script.is_dirty {
                    state.script.pending_clear_confirm = true;
                } else {
                    state.script.active_content = None;
                    state.script.active_path = None;
                    state.script.is_dirty = false;
                }
            }
        }
    });

    ui.separator();

    // Code Editor Alanı
    if has_script {
        // İçerik henüz string değilse boş olarak init edelim
        let content = state.script.active_content.get_or_insert_with(String::new);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let response = ui.add_sized(
                    ui.available_size(),
                    // TODO: Syntax Highlight engine needed (egui_extras / custom)
                    egui::TextEdit::multiline(content)
                        .font(egui::TextStyle::Monospace) // Monospace font
                        .code_editor()
                        .lock_focus(true) // Tab tuşunun focus u kaçırmasını engelle
                        .desired_width(f32::INFINITY),
                );
                if response.changed() {
                    state.script.is_dirty = true;
                }
            });
    } else {
        ui.centered_and_justified(|ui| {
            ui.label("Görüntülemek veya düzenlemek için bir script seçin.");
        });
    }
}
