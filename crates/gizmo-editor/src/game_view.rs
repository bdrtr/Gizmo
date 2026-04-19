use crate::EditorState;

pub fn ui_game_view(ui: &mut egui::Ui, state: &mut EditorState) {
    state.game_view_visible = true;
    let is_playing = state.is_playing();
    let is_paused = state.mode == crate::editor_state::EditorMode::Paused;

    if is_playing || is_paused {
        let rect = ui.available_rect_before_wrap();
        if let Some(tex_id) = state.scene_texture_id {
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
                egui::RichText::new(
                    "Toolbar'daki ▶ Başlat butonuna\nbasarak simülasyonu çalıştırın.",
                )
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
