//! Toolbar — Üst toolbar paneli (Save/Load/Play/Pause/Gizmo mode)

use egui;
use crate::editor_state::{EditorState, GizmoMode, EditorMode};

/// Toolbar panelini çizer
pub fn draw_toolbar(ctx: &egui::Context, state: &mut EditorState) {
    egui::TopBottomPanel::top("toolbar_panel")
        .exact_height(36.0)
        .show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                
                // === DOSYA İŞLEMLERİ ===
                if ui.button("💾 Kaydet").clicked() {
                    state.status_message = format!("Sahne kaydediliyor → {}", state.scene_path);
                }
                
                if ui.button("📂 Yükle").clicked() {
                    state.status_message = format!("Sahne yükleniyor ← {}", state.scene_path);
                }
                
                ui.separator();
                
                // === PLAY/PAUSE/STOP ===
                let play_text = match state.mode {
                    EditorMode::Edit => "▶ Başlat",
                    EditorMode::Play => "⏹ Durdur",
                    EditorMode::Paused => "▶ Devam",
                };
                let play_color = match state.mode {
                    EditorMode::Edit => egui::Color32::from_rgb(80, 200, 80),
                    EditorMode::Play => egui::Color32::from_rgb(200, 80, 80),
                    EditorMode::Paused => egui::Color32::from_rgb(200, 200, 80),
                };
                
                if ui.button(egui::RichText::new(play_text).color(play_color)).clicked() {
                    state.toggle_play();
                }
                
                if state.mode == EditorMode::Play
                    && ui.button("⏸ Duraklat").clicked() {
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
                
                // === PANEL TOGGLE ===
                ui.checkbox(&mut state.show_hierarchy, "Hiyerarşi");
                ui.checkbox(&mut state.show_inspector, "Inspector");
                ui.checkbox(&mut state.show_asset_browser, "Assets");
                
                // === SAHNE YOLU ===
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(&state.status_message).weak().small());
                });
            });
        });
}
