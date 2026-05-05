//! Toolbar — Üst toolbar paneli (Save/Load/Play/Pause/Gizmo mode)

use crate::editor_state::{BuildTarget, EditorMode, EditorState, GizmoMode};
use egui;

/// Toolbar panelini çizer
pub fn draw_toolbar(ctx: &egui::Context, state: &mut EditorState) {
    egui::TopBottomPanel::top("toolbar_panel")
        .exact_height(36.0)
        .show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;

                // === DOSYA İŞLEMLERİ ===
                if ui
                    .button("🪄 Yeni/Temizle")
                    .on_hover_text("Sahneyi sıfırla")
                    .clicked()
                {
                    state.scene.clear_request = true;
                }

                ui.label("Sahne:");
                ui.add(egui::TextEdit::singleline(&mut state.scene_path).desired_width(120.0));

                let is_dialog_open = state.pending_dialog_rx.is_some();

                if ui
                    .add_enabled(!is_dialog_open, egui::Button::new("💾 Kaydet"))
                    .clicked()
                {
                    let (tx, rx) = std::sync::mpsc::channel();
                    state.pending_dialog_rx = Some(std::sync::Mutex::new(rx));
                    let scene_path = state.scene_path.clone();
                    std::thread::spawn(move || {
                        let mut initial_dir = std::path::PathBuf::from(".");
                        if let Some(parent) = std::path::Path::new(&scene_path).parent() {
                            if parent.exists() && parent.is_dir() {
                                initial_dir = parent.to_path_buf();
                            }
                        }
                        let res = rfd::FileDialog::new()
                            .add_filter("Gizmo Scene", &["scene"])
                            .set_directory(&initial_dir)
                            .save_file();
                        let _ = tx.send((
                            true,
                            res.map(|p| {
                                let s = p.to_string_lossy().to_string();
                                if s.starts_with(r"\\?\") {
                                    s[4..].to_string()
                                } else {
                                    s
                                }
                            }),
                        ));
                    });
                }

                if ui
                    .add_enabled(!is_dialog_open, egui::Button::new("📂 Yükle"))
                    .clicked()
                {
                    let (tx, rx) = std::sync::mpsc::channel();
                    state.pending_dialog_rx = Some(std::sync::Mutex::new(rx));
                    let scene_path = state.scene_path.clone();
                    std::thread::spawn(move || {
                        let mut initial_dir = std::path::PathBuf::from(".");
                        if let Some(parent) = std::path::Path::new(&scene_path).parent() {
                            if parent.exists() && parent.is_dir() {
                                initial_dir = parent.to_path_buf();
                            }
                        }
                        let res = rfd::FileDialog::new()
                            .add_filter("Gizmo Scene", &["scene"])
                            .set_directory(&initial_dir)
                            .pick_file();
                        let _ = tx.send((
                            false,
                            res.map(|p| {
                                let s = p.to_string_lossy().to_string();
                                if s.starts_with(r"\\?\") {
                                    s[4..].to_string()
                                } else {
                                    s
                                }
                            }),
                        ));
                    });
                }

                ui.separator();

                if state.mode == EditorMode::Edit {
                    if ui
                        .button(
                            egui::RichText::new("▶ Başlat")
                                .color(egui::Color32::from_rgb(80, 200, 80)),
                        )
                        .clicked()
                    {
                        state.toggle_play();
                    }
                } else {
                    let pause_text = if state.mode == EditorMode::Play {
                        "⏸ Duraklat"
                    } else {
                        "▶ Devam"
                    };
                    if ui
                        .button(
                            egui::RichText::new(pause_text)
                                .color(egui::Color32::from_rgb(200, 200, 80)),
                        )
                        .clicked()
                    {
                        state.toggle_pause();
                    }

                    if ui
                        .button(
                            egui::RichText::new("⏹ Durdur")
                                .color(egui::Color32::from_rgb(200, 80, 80)),
                        )
                        .clicked()
                    {
                        state.toggle_play();
                    }
                }

                ui.separator();

                // === GIZMO MODE ===
                ui.label("Araç:");

                let t_selected = state.gizmo_mode == GizmoMode::Translate;
                if ui.selectable_label(t_selected, "🔀 Taşı").clicked() {
                    state.gizmo_mode = GizmoMode::Translate;
                }
                let r_selected = state.gizmo_mode == GizmoMode::Rotate;
                if ui.selectable_label(r_selected, "🔄 Döndür").clicked() {
                    state.gizmo_mode = GizmoMode::Rotate;
                }
                let s_selected = state.gizmo_mode == GizmoMode::Scale;
                if ui.selectable_label(s_selected, "📏 Ölçekle").clicked() {
                    state.gizmo_mode = GizmoMode::Scale;
                }

                ui.separator();

                // === GIZMO UZAYI (LOCAL/GLOBAL) ===
                let space_text = if state.gizmo_local_space {
                    "📦 Local"
                } else {
                    "🌐 Global"
                };
                if ui.button(space_text).clicked() {
                    state.gizmo_local_space = !state.gizmo_local_space;
                }

                let snap_color = if state.prefs.snap_enabled {
                    egui::Color32::from_rgb(100, 255, 100) // Aktif (yeşil)
                } else {
                    ui.visuals().text_color() // Pasif
                };

                if ui
                    .button(egui::RichText::new("🧲 Snap").color(snap_color))
                    .on_hover_text("Grid yapılarına (hareket/döndürme) yapışma")
                    .clicked()
                {
                    state.prefs.snap_enabled = !state.prefs.snap_enabled;
                    state.prefs.mark_dirty();
                }

                ui.separator();

                // === PENCERELER ===
                ui.menu_button("🪟 Pencereler", |ui| {
                    if ui.button("💾 Düzeni Kaydet").clicked() {
                        state.save_layout();
                        ui.close_menu();
                    }
                    if ui.button("♻ Varsayılan Düzene Dön").clicked() {
                        state.reset_layout();
                        ui.close_menu();
                    }
                });

                ui.separator();
                
                // === AI TOOLS ===
                if ui.button(egui::RichText::new("🤖 NavMesh Kur").color(egui::Color32::from_rgb(100, 200, 255)))
                   .on_hover_text("Fiziksel dünyadaki statik objelere göre Yapay Zeka navigasyon ızgarasını (NavMesh) yeniden oluşturur.")
                   .clicked() {
                    state.scene.rebuild_navmesh_request = true;
                }

                ui.separator();

                // === AYARLAR ===
                let settings_color = if state.is_tab_open(&crate::editor_state::EditorTab::Settings)
                {
                    egui::Color32::from_rgb(100, 200, 255)
                } else {
                    egui::Color32::GRAY
                };
                if ui
                    .button(egui::RichText::new("⚙️ Ayarlar").color(settings_color))
                    .clicked()
                {
                    state.open_tab(crate::editor_state::EditorTab::Settings);
                }

                ui.separator();

                // === BUILD SİSTEMİ ===
                if state
                    .build
                    .is_building
                    .load(std::sync::atomic::Ordering::Acquire)
                {
                    ui.add(egui::Spinner::new());
                    if let Some(st) = state.build.start_time {
                        let elapsed = st.elapsed().as_secs();
                        ui.label(
                            egui::RichText::new(format!("Derleniyor... ({}s)", elapsed))
                                .color(egui::Color32::YELLOW),
                        );
                    } else {
                        ui.label(egui::RichText::new("Derleniyor...").color(egui::Color32::YELLOW));
                    }
                } else {
                    // -- İşletim Sistemi Seçimi --
                    let target_label = match state.build.target {
                        BuildTarget::Native => "💻 Native (Mevcut OS)",
                        BuildTarget::Linux => "🐧 Linux",
                        BuildTarget::Windows => "🪟 Windows",
                        BuildTarget::MacOs => "🍎 macOS",
                    };
                    egui::ComboBox::from_id_source(egui::Id::new("build_target_combo"))
                        .selected_text(target_label)
                        .width(105.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut state.build.target,
                                BuildTarget::Native,
                                "💻 Native (Mevcut OS)",
                            );
                            ui.selectable_value(
                                &mut state.build.target,
                                BuildTarget::Linux,
                                "🐧 Linux (ELF)",
                            );
                            ui.selectable_value(
                                &mut state.build.target,
                                BuildTarget::Windows,
                                "🪟 Windows (.exe)",
                            );
                            ui.selectable_value(
                                &mut state.build.target,
                                BuildTarget::MacOs,
                                "🍎 macOS",
                            );
                        });

                    if ui
                        .button(
                            egui::RichText::new("🚀 Build Et")
                                .strong()
                                .color(egui::Color32::from_rgb(100, 255, 100)),
                        )
                        .clicked()
                    {
                        state.build.request = true;
                        state.build.start_time = Some(std::time::Instant::now());
                        state.open_tab(crate::editor_state::EditorTab::BuildConsole);
                    }
                }
            });
        });
}
