use crate::EditorState;

/// Draws the game view tab: what the game camera renders. A fight's HUD is the game's own
/// (`gizmo::systems::fight_hud`), in the picture.
pub fn ui_game_view(ui: &mut egui::Ui, state: &mut EditorState) {
    state.game_view_visible = true;
    state.game_view_pointer = None;
    let is_playing = state.is_playing();
    let is_paused = state.mode == crate::editor_state::EditorMode::Paused;

    // The texture is shown whenever there IS one, not only while playing.
    //
    // This used to be `if is_playing || is_paused`, and that one condition threw away a whole
    // feature. The studio renders the game camera into its own target on every frame it is NOT
    // playing — a separate scene pass, editor chrome excluded, built precisely so this panel can
    // show a live preview while you edit (the render's own comments record the work of stopping
    // the two panels drawing the same picture). The panel then refused to display it outside play
    // mode and drew a placeholder instead, so that pass rendered into a texture nobody ever saw.
    //
    // The two conditions were exact opposites: rendered only when not playing, displayed only
    // when playing.
    if state.game_texture_id.is_some() {
        if is_playing {
            ui.input_mut(|i| i.events.clear());
        }
        let rect = ui.available_rect_before_wrap();
        state.game_view_rect = Some(rect);
        state.game_view_size = Some(ui.available_size());
        // The pointer in the panel's own frame: the game's UI is laid out to this panel (its
        // render target is sized in points), so this is the pointer it is hit with.
        state.game_view_pointer = ui
            .input(|i| i.pointer.hover_pos())
            .filter(|p| rect.contains(*p))
            .map(|p| p - rect.min);
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
            } else if !is_playing {
                // A line, not a takeover: the preview is the point of the panel, and the one
                // thing a reader needs told is that it is a preview and how to start the game.
                ui.painter().text(
                    egui::pos2(rect.center().x, rect.max.y - 10.0),
                    egui::Align2::CENTER_BOTTOM,
                    "oyun kamerası — önizleme · ▶ Başlat ile çalıştır",
                    egui::FontId::proportional(12.0),
                    egui::Color32::from_white_alpha(90),
                );
            }
        }
    } else {
        // No texture at all — no game camera in the scene, or the first frame before the render
        // target exists. This is the only case with nothing to show, so it is the only case that
        // gets the placeholder and the shortcut list.
        ui.vertical_centered(|ui| {
            ui.add_space(30.0);
            ui.label(
                egui::RichText::new("▶ Oyunu Başlat")
                    .size(26.0)
                    .color(egui::Color32::from_white_alpha(60)),
            );

            ui.label(
                egui::RichText::new(
                    "Sahnede bir oyun kamerası yok, ya da ilk kare henüz çizilmedi.\nToolbar'daki ▶ Başlat butonuna basarak simülasyonu çalıştırın.",
                )
                .size(14.0)
                .color(egui::Color32::from_white_alpha(40)),
            );
        });

        ui.separator();

        ui.label(egui::RichText::new("📋 Editör Kısayolları").strong());

        // The camera keys are listed WITH the button that arms them. They used to be listed bare,
        // which was both wrong (they now need the right button held) and the reason the collision
        // with the tool shortcuts below went unnoticed for so long: read as written, W meant two
        // different things on the same screen.
        let shortcuts = [
            ("Sağ Tık (basılı) + W A S D", "Kamerayı hareket ettir"),
            ("Sağ Tık (basılı) + Q / E", "Kamerayı aşağı / yukarı taşı"),
            ("Sağ Tık + Sürükle", "Kamerayı döndür"),
            ("Orta Tık + Sürükle", "Kamerayı kaydır (pan)"),
            ("Scroll", "Yakınlaştır / uzaklaştır"),
            ("F", "Seçili objeye odaklan"),
            ("Delete", "Seçili objeyi sil"),
            ("Ctrl + Z / Y", "Geri al / İleri al"),
            ("Ctrl + D", "Seçili objeyi çoğalt"),
            ("Ctrl + Sürükle (Gizmo)", "Snap ile hareket"),
            ("Q / W / E / R", "Araç: Seç / Taşı / Döndür / Ölçek"),
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

