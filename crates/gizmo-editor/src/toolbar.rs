//! Toolbar — Üst toolbar paneli (Save/Load/Play/Pause/Gizmo mode)

use egui;
use crate::editor_state::{EditorState, GizmoMode, EditorMode, BuildTarget};

/// Toolbar panelini çizer
pub fn draw_toolbar(ctx: &egui::Context, state: &mut EditorState) {
    egui::TopBottomPanel::top("toolbar_panel")
        .exact_height(36.0)
        .show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;

                // === DOSYA İŞLEMLERİ ===
                ui.label("Sahne:");
                ui.add(egui::TextEdit::singleline(&mut state.scene_path).desired_width(120.0));

                if ui.button("💾 Kaydet").clicked() {
                    state.status_message = format!("Sahne kaydediliyor → {}", state.scene_path);
                    state.scene_save_request = Some(state.scene_path.clone());
                }

                if ui.button("📂 Yükle").clicked() {
                    state.status_message = format!("Sahne yükleniyor ← {}", state.scene_path);
                    state.scene_load_request = Some(state.scene_path.clone());
                }

                ui.separator();

                // === PLAY/PAUSE/STOP ===
                let play_text = match state.mode {
                    EditorMode::Edit   => "▶ Başlat",
                    EditorMode::Play   => "⏹ Durdur",
                    EditorMode::Paused => "▶ Devam",
                };
                let play_color = match state.mode {
                    EditorMode::Edit   => egui::Color32::from_rgb(80, 200, 80),
                    EditorMode::Play   => egui::Color32::from_rgb(200, 80, 80),
                    EditorMode::Paused => egui::Color32::from_rgb(200, 200, 80),
                };

                if ui.button(egui::RichText::new(play_text).color(play_color)).clicked() {
                    state.toggle_play();
                }

                if state.mode == EditorMode::Play
                    && ui.button("⏸ Duraklat").clicked()
                {
                    state.toggle_pause();
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
                let space_text = if state.gizmo_local_space { "📦 Local" } else { "🌐 Global" };
                if ui.button(space_text).clicked() {
                    state.gizmo_local_space = !state.gizmo_local_space;
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

                // === AYARLAR ===
                let settings_color = if state.settings_open {
                    egui::Color32::from_rgb(100, 200, 255)
                } else {
                    egui::Color32::GRAY
                };
                if ui.button(egui::RichText::new("⚙️ Ayarlar").color(settings_color)).clicked() {
                    state.settings_open = !state.settings_open;
                }

                ui.separator();

                // === BUILD SİSTEMİ ===
                if state.is_building.load(std::sync::atomic::Ordering::SeqCst) {
                    ui.add(egui::Spinner::new());
                    ui.label(egui::RichText::new("Derleniyor...").color(egui::Color32::YELLOW));
                } else {
                    // -- İşletim Sistemi Seçimi --
                    let target_label = match state.build_target {
                        BuildTarget::Native  => "💻 Native",
                        BuildTarget::Linux   => "🐧 Linux",
                        BuildTarget::Windows => "🪟 Windows",
                        BuildTarget::MacOs   => "🍎 macOS",
                    };
                    egui::ComboBox::from_id_source("build_target_combo")
                        .selected_text(target_label)
                        .width(105.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut state.build_target, BuildTarget::Native,  "💻 Native (Mevcut OS)");
                            ui.selectable_value(&mut state.build_target, BuildTarget::Linux,   "🐧 Linux (ELF)");
                            ui.selectable_value(&mut state.build_target, BuildTarget::Windows, "🪟 Windows (.exe)");
                            ui.selectable_value(&mut state.build_target, BuildTarget::MacOs,   "🍎 macOS");
                        });

                    if ui.button(
                        egui::RichText::new("🚀 Build Et")
                            .strong()
                            .color(egui::Color32::from_rgb(100, 255, 100)),
                    ).clicked() {
                        state.build_request = true;
                    }
                }

                // === DURUM MESAJI (sağ taraf) ===
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(&state.status_message).weak().small());
                });
            });
        });
}
