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

            // ============================================================
            //  🥊 FIGHTING GAME HUD OVERLAY
            // ============================================================
            if state.fight_hud.active {
                draw_fight_hud(ui, rect, state);
            }

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

/// Dövüş oyunu HUD overlay'ı çizer:
/// - P1 Health Bar (Sol, Mavi)
/// - P2 Health Bar (Sağ, Kırmızı)
/// - Round göstergesi (Ortada)
/// - Timer (Ortada)
fn draw_fight_hud(ui: &mut egui::Ui, rect: egui::Rect, state: &EditorState) {
    let painter = ui.painter();
    let hud = &state.fight_hud;

    let bar_height = 20.0_f32;
    let bar_margin = 16.0_f32;
    let bar_y = rect.min.y + bar_margin;
    let center_gap = 80.0_f32;

    let total_width = rect.width() - bar_margin * 2.0 - center_gap;
    let half_width = total_width * 0.5;

    // ===== P1 HEALTH BAR (Sol) =====
    let p1_bar_left = rect.min.x + bar_margin;
    let p1_ratio = (hud.p1_health / hud.p1_max_health).clamp(0.0, 1.0);

    painter.rect_filled(
        egui::Rect::from_min_max(
            egui::pos2(p1_bar_left, bar_y),
            egui::pos2(p1_bar_left + half_width, bar_y + bar_height),
        ),
        3.0,
        egui::Color32::from_rgb(30, 30, 30),
    );

    let p1_health_color = if p1_ratio > 0.3 {
        egui::Color32::from_rgb(40, 140, 255)
    } else {
        egui::Color32::from_rgb(255, 60, 60)
    };
    painter.rect_filled(
        egui::Rect::from_min_max(
            egui::pos2(p1_bar_left, bar_y),
            egui::pos2(p1_bar_left + half_width * p1_ratio, bar_y + bar_height),
        ),
        3.0,
        p1_health_color,
    );

    painter.text(
        egui::pos2(p1_bar_left + 4.0, bar_y + bar_height + 4.0),
        egui::Align2::LEFT_TOP,
        &hud.p1_name,
        egui::FontId::proportional(13.0),
        egui::Color32::from_rgb(100, 180, 255),
    );

    // ===== P2 HEALTH BAR (Sağ) =====
    let p2_bar_right = rect.max.x - bar_margin;
    let p2_bar_left = p2_bar_right - half_width;
    let p2_ratio = (hud.p2_health / hud.p2_max_health).clamp(0.0, 1.0);

    painter.rect_filled(
        egui::Rect::from_min_max(
            egui::pos2(p2_bar_left, bar_y),
            egui::pos2(p2_bar_right, bar_y + bar_height),
        ),
        3.0,
        egui::Color32::from_rgb(30, 30, 30),
    );

    let p2_health_color = if p2_ratio > 0.3 {
        egui::Color32::from_rgb(255, 60, 60)
    } else {
        egui::Color32::from_rgb(255, 200, 40)
    };
    let p2_fill_left = p2_bar_right - half_width * p2_ratio;
    painter.rect_filled(
        egui::Rect::from_min_max(
            egui::pos2(p2_fill_left, bar_y),
            egui::pos2(p2_bar_right, bar_y + bar_height),
        ),
        3.0,
        p2_health_color,
    );

    painter.text(
        egui::pos2(p2_bar_right - 4.0, bar_y + bar_height + 4.0),
        egui::Align2::RIGHT_TOP,
        &hud.p2_name,
        egui::FontId::proportional(13.0),
        egui::Color32::from_rgb(255, 120, 120),
    );

    // ===== ROUND + TIMER =====
    let center_x = rect.center().x;

    painter.text(
        egui::pos2(center_x, bar_y + bar_height * 0.5),
        egui::Align2::CENTER_CENTER,
        &format!("R{}", hud.current_round),
        egui::FontId::proportional(18.0),
        egui::Color32::from_rgb(255, 220, 80),
    );

    let timer_secs = hud.timer_seconds as u32;
    painter.text(
        egui::pos2(center_x, bar_y + bar_height + 6.0),
        egui::Align2::CENTER_TOP,
        &format!("{}", timer_secs),
        egui::FontId::proportional(24.0),
        egui::Color32::WHITE,
    );

    // ===== K.O. OVERLAY =====
    if hud.p1_health <= 0.0 || hud.p2_health <= 0.0 {
        let winner = if hud.p1_health > 0.0 { &hud.p1_name } else { &hud.p2_name };
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "K.O.",
            egui::FontId::proportional(72.0),
            egui::Color32::from_rgb(255, 50, 50),
        );
        painter.text(
            egui::pos2(rect.center().x, rect.center().y + 50.0),
            egui::Align2::CENTER_CENTER,
            &format!("{} WINS!", winner),
            egui::FontId::proportional(28.0),
            egui::Color32::from_rgb(255, 220, 80),
        );
    }
}
