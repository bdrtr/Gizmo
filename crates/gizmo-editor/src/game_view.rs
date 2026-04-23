use crate::EditorState;

pub fn ui_game_view(ui: &mut egui::Ui, state: &mut EditorState) {
    state.game_view_visible = true;
    let is_playing = state.is_playing();
    let is_paused = state.mode == crate::editor_state::EditorMode::Paused;

    if is_playing || is_paused {
        if is_playing {
            ui.input_mut(|i| i.events.clear());
        }
        let rect = ui.available_rect_before_wrap();
        state.game_view_rect = Some(rect);
        state.game_view_size = Some(ui.available_size());
        if let Some(tex_id) = state.game_texture_id {
            let mut mesh = egui::Mesh::with_texture(tex_id);
            mesh.add_rect_with_uv(
                rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
            ui.painter().add(mesh);

            if is_paused {
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "⏸ DURAKLATILDI",
                    egui::FontId::proportional(40.0),
                    egui::Color32::from_white_alpha(150),
                );
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

            ui.label(
                egui::RichText::new(
                    "Toolbar'daki ▶ Başlat butonuna\nbasarak simülasyonu çalıştırın.",
                )
                .size(14.0)
                .color(egui::Color32::from_white_alpha(40)),
            );
        });

        ui.separator();

        ui.label(egui::RichText::new("📋 Editör Kısayolları").strong());

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

        egui::Grid::new(ui.id().with("shortcut_grid"))
            .num_columns(2)
            .spacing([16.0, 4.0])
            .show(ui, |ui| {
                for (key, desc) in &shortcuts {
                    ui.label(
                        egui::RichText::new(*key)
                            .monospace()
                            .color(egui::Color32::from_rgb(200, 200, 100)),
                    );
                    ui.label(egui::RichText::new(*desc).color(egui::Color32::GRAY));
                    ui.end_row();
                }
            });
    }
}
