use gizmo::prelude::*;
use gizmo::egui;
use crate::state::GameState;

pub fn render_game_hud(ctx: &egui::Context, world: &World, state: &GameState) {
    // --- DİYALOG KUTUSU (Lua'dan tetiklenir) ---
    if let Some(ref dlg) = state.active_dialogue {
        egui::Window::new("##dialogue")
            .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -60.0))
            .title_bar(false)
            .resizable(false)
            .collapsible(false)
            .min_width(500.0)
            .frame(egui::Frame::window(&ctx.style())
                .fill(egui::Color32::from_rgba_premultiplied(25, 25, 27, 240))
                .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(60, 60, 62)))
                .rounding(egui::Rounding::same(6.0)))
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.label(egui::RichText::new(&dlg.speaker)
                        .color(egui::Color32::from_rgb(255, 200, 80))
                        .strong().size(14.0));
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new(&dlg.text)
                        .color(egui::Color32::WHITE).size(16.0));
                    if dlg.timer > 0.0 {
                        ui.add_space(6.0);
                        ui.add(egui::ProgressBar::new(
                            (dlg.timer / 3.0_f32.max(dlg.timer)).clamp(0.0, 1.0)
                        ).desired_width(200.0).fill(egui::Color32::from_rgb(255, 200, 80)));
                    }
                });
            });
    }

    // --- YARIŞ HUD ---
    if let Some(ref race) = state.ps1_race {
        let draw_shadow_text = |ui: &mut egui::Ui, text: String, pos: egui::Pos2, size: f32, color: egui::Color32| {
            let font = egui::FontId::proportional(size);
            ui.painter().text(pos + egui::vec2(2.0, 3.0), egui::Align2::CENTER_CENTER, &text, font.clone(), egui::Color32::from_black_alpha(150));
            ui.painter().text(pos, egui::Align2::CENTER_CENTER, &text, font, color);
        };

        if race.phase == crate::race::RacePhase::Countdown {
            let txt = crate::race::countdown_text(race);
            if !txt.is_empty() {
                egui::Area::new("countdown_hud")
                    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, -80.0))
                    .order(egui::Order::Foreground)
                    .interactable(false)
                    .show(ctx, |ui| {
                        let time_fract = race.countdown_timer.fract();
                        let pulse = 1.0 + (time_fract * std::f32::consts::PI).sin() * 0.2;
                        let size = 120.0 * (if txt == "GO!" { 1.5 } else { pulse });
                        let color = if txt == "GO!" { egui::Color32::from_rgb(50, 255, 100) } else { egui::Color32::from_rgb(255, 200, 50) };
                        let pos = ui.max_rect().center();
                        draw_shadow_text(ui, txt.to_string(), pos, size, color);
                    });
            }
        }

        if race.phase == crate::race::RacePhase::Racing {
            // Hız Göstergesi
            egui::Area::new("race_speedometer")
                .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-40.0, -40.0))
                .order(egui::Order::Foreground)
                .interactable(false)
                .show(ctx, |ui| {
                    let desired_size = egui::vec2(200.0, 80.0);
                    let (rect, _) = ui.allocate_exact_size(desired_size, egui::Sense::hover());
                    ui.painter().rect_filled(rect, 16.0, egui::Color32::from_rgba_premultiplied(10, 15, 25, 200));
                    ui.painter().rect_stroke(rect, 16.0, egui::Stroke::new(2.0, egui::Color32::from_white_alpha(30)));
                    
                    let speed = crate::race::get_speed_kmh(world, race.player_entity);
                    ui.painter().text(
                        rect.right_center() + egui::vec2(-20.0, 15.0),
                        egui::Align2::RIGHT_BOTTOM,
                        "KM/H",
                        egui::FontId::proportional(20.0),
                        egui::Color32::from_gray(150),
                    );

                    let speed_color = if speed > 150.0 { egui::Color32::from_rgb(255, 80, 50) } else { egui::Color32::from_rgb(200, 240, 255) }; 
                    ui.painter().text(
                        rect.right_center() + egui::vec2(-20.0, -10.0),
                        egui::Align2::RIGHT_BOTTOM,
                        format!("{:.0}", speed),
                        egui::FontId::proportional(64.0),
                        speed_color,
                    );
                });

            // Sıralama ve Tur
            egui::Area::new("race_status_top")
                .anchor(egui::Align2::LEFT_TOP, egui::vec2(40.0, 40.0))
                .order(egui::Order::Foreground)
                .interactable(false)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        let (pos_rect, _) = ui.allocate_exact_size(egui::vec2(160.0, 70.0), egui::Sense::hover());
                        ui.painter().rect_filled(pos_rect, 12.0, egui::Color32::from_rgba_premultiplied(10, 15, 25, 220));
                        ui.painter().rect_stroke(pos_rect, 12.0, egui::Stroke::new(2.0, egui::Color32::from_white_alpha(30)));
                        
                        ui.painter().text(
                            pos_rect.left_center() + egui::vec2(15.0, 0.0),
                            egui::Align2::LEFT_CENTER,
                            "POS",
                            egui::FontId::proportional(22.0),
                            egui::Color32::from_rgb(255, 200, 80),
                        );

                        let pos = crate::race::get_player_position(race, world);
                        let total_cars = 1 + race.ai_entities.len();
                        let postfix = match pos { 1 => "st", 2 => "nd", 3 => "rd", _ => "th" };

                        ui.painter().text(
                            pos_rect.right_center() + egui::vec2(-15.0, -8.0),
                            egui::Align2::RIGHT_CENTER,
                            format!("{}", pos),
                            egui::FontId::proportional(42.0),
                            egui::Color32::WHITE,
                        );
                        ui.painter().text(
                            pos_rect.right_center() + egui::vec2(-15.0, 18.0),
                            egui::Align2::RIGHT_CENTER,
                            format!("{}/{}", postfix, total_cars),
                            egui::FontId::proportional(16.0),
                            egui::Color32::GRAY,
                        );

                        ui.add_space(20.0);

                        let (lap_rect, _) = ui.allocate_exact_size(egui::vec2(160.0, 70.0), egui::Sense::hover());
                        ui.painter().rect_filled(lap_rect, 12.0, egui::Color32::from_rgba_premultiplied(10, 15, 25, 220));
                        ui.painter().rect_stroke(lap_rect, 12.0, egui::Stroke::new(2.0, egui::Color32::from_white_alpha(30)));
                        
                        ui.painter().text(
                            lap_rect.left_center() + egui::vec2(15.0, 0.0),
                            egui::Align2::LEFT_CENTER,
                            "LAP",
                            egui::FontId::proportional(22.0),
                            egui::Color32::LIGHT_BLUE,
                        );

                        ui.painter().text(
                            lap_rect.right_center() + egui::vec2(-15.0, 0.0),
                            egui::Align2::RIGHT_CENTER,
                            format!("{}/{}", race.player_laps.min(race.total_laps) + 1, race.total_laps),
                            egui::FontId::proportional(36.0),
                            egui::Color32::WHITE,
                        );
                    });
                });

            // Yarış Süresi
            egui::Area::new("race_timer_top")
                .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 30.0))
                .order(egui::Order::Foreground)
                .interactable(false)
                .show(ctx, |ui| {
                    let mins = (race.race_timer / 60.0) as u32;
                    let secs = race.race_timer % 60.0;
                    let text = format!("{:02}:{:05.2}", mins, secs);

                    let (rect, _) = ui.allocate_exact_size(egui::vec2(200.0, 50.0), egui::Sense::hover());
                    ui.painter().rect_filled(rect, 8.0, egui::Color32::from_rgba_premultiplied(0, 0, 0, 150));
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        text,
                        egui::FontId::proportional(32.0),
                        egui::Color32::from_rgb(100, 255, 150),
                    );
                });
        }

        // Bitiş Ekranı
        if race.phase == crate::race::RacePhase::Finished {
            let screen_rect = ctx.screen_rect();
            egui::Area::new("finish_bg")
                .anchor(egui::Align2::LEFT_TOP, egui::vec2(0.0, 0.0))
                .order(egui::Order::Foreground)
                .interactable(true)
                .show(ctx, |ui| {
                    ui.painter().rect_filled(screen_rect, 0.0, egui::Color32::from_black_alpha(200));
                });

            egui::Area::new("finish_panel")
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    let panel_size = egui::vec2(500.0, 400.0);
                    let (rect, _) = ui.allocate_exact_size(panel_size, egui::Sense::hover());
                    
                    ui.painter().rect_filled(rect, 16.0, egui::Color32::from_rgba_premultiplied(20, 25, 40, 240));
                    ui.painter().rect_stroke(rect, 16.0, egui::Stroke::new(2.0, egui::Color32::from_rgb(100, 150, 255)));

                    ui.painter().text(
                        rect.center_top() + egui::vec2(0.0, 40.0),
                        egui::Align2::CENTER_CENTER,
                        "RACE FINISHED",
                        egui::FontId::proportional(48.0),
                        egui::Color32::from_rgb(255, 200, 80),
                    );
                    
                    ui.painter().line_segment(
                        [rect.left_top() + egui::vec2(50.0, 80.0), rect.right_top() + egui::vec2(-50.0, 80.0)], 
                        egui::Stroke::new(2.0, egui::Color32::from_white_alpha(50))
                    );

                    let mut start_y = 130.0;
                    for (i, &(id, time)) in race.finish_order.iter().enumerate() {
                        let is_player = id == race.player_entity;
                        let name = if is_player { "PLAYER (YOU)" } else { "AI RIVAL" };
                        let color = if is_player { egui::Color32::from_rgb(50, 255, 120) } else { egui::Color32::from_gray(200) };
                        let mins = (time / 60.0) as u32;
                        let secs = time % 60.0;
                        
                        let rank_txt = format!("{}  -  {}", i + 1, name);
                        let time_txt = format!("{:02}:{:05.2}", mins, secs);

                        ui.painter().text(
                            rect.left_top() + egui::vec2(60.0, start_y),
                            egui::Align2::LEFT_CENTER,
                            rank_txt,
                            egui::FontId::proportional(24.0),
                            color,
                        );

                        ui.painter().text(
                            rect.right_top() + egui::vec2(-60.0, start_y),
                            egui::Align2::RIGHT_CENTER,
                            time_txt,
                            egui::FontId::proportional(24.0),
                            color,
                        );

                        start_y += 50.0;
                    }
                    
                    ui.painter().text(
                        rect.center_bottom() + egui::vec2(0.0, -30.0),
                        egui::Align2::CENTER_CENTER,
                        "Press 'R' to Restart or 'ESC' to Menu",
                        egui::FontId::proportional(16.0),
                        egui::Color32::from_gray(120),
                    );
                });
        }
    }

    // --- GENEL OYUN HUD (Ammo / Crosshair vb) ---
    if let Some(stats) = world.get_resource::<crate::state::PlayerStats>() {
        let health_pct = (stats.health / stats.max_health).clamp(0.0, 1.0);
        
        // Health Bar 
        egui::Area::new("health_hud")
            .anchor(egui::Align2::LEFT_BOTTOM, egui::vec2(30.0, -30.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let desired_size = egui::vec2(250.0, 24.0);
                let (rect, _resp) = ui.allocate_exact_size(desired_size, egui::Sense::hover());
                
                ui.painter().rect_filled(rect, 12.0, egui::Color32::from_rgba_premultiplied(20, 20, 20, 150));
                
                let mut fill_rect = rect;
                fill_rect.max.x = rect.min.x + (rect.width() * health_pct);
                let color = if health_pct > 0.5 { egui::Color32::from_rgb(40, 200, 80) } 
                            else if health_pct > 0.2 { egui::Color32::from_rgb(220, 150, 20) } 
                            else { egui::Color32::from_rgb(220, 40, 40) };
                
                ui.painter().rect_filled(fill_rect, 12.0, color);
                
                let text = format!("+ {:.0}", stats.health);
                ui.painter().text(
                    rect.left_center() + egui::vec2(15.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    text,
                    egui::FontId::proportional(16.0),
                    egui::Color32::WHITE,
                );
            });

        // Ammo Counter 
        egui::Area::new("ammo_hud")
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-40.0, -30.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!("{}", stats.ammo))
                            .size(48.0)
                            .strong()
                            .color(egui::Color32::WHITE)
                    );
                    ui.label(
                        egui::RichText::new(format!("/{}", stats.max_ammo))
                            .size(24.0)
                            .color(egui::Color32::GRAY)
                    );
                });
            });
    }

    // --- GIZMO CITY DASH HUD ---
    if state.game_max_score > 0 {
        egui::Area::new("coin_hud")
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 40.0))
            .order(egui::Order::Foreground)
            .interactable(false)
            .show(ctx, |ui| {
                let text = format!("COINS: {} / {}", state.game_score, state.game_max_score);
                let color = if state.game_score == state.game_max_score {
                    egui::Color32::from_rgb(50, 255, 100) // Yeşil
                } else {
                    egui::Color32::from_rgb(255, 215, 0) // Altın Sarısı
                };
                
                ui.painter().text(
                    ui.max_rect().center() + egui::vec2(3.0, 3.0),
                    egui::Align2::CENTER_CENTER,
                    &text,
                    egui::FontId::proportional(48.0),
                    egui::Color32::from_black_alpha(200),
                );
                
                ui.painter().text(
                    ui.max_rect().center(),
                    egui::Align2::CENTER_CENTER,
                    &text,
                    egui::FontId::proportional(48.0),
                    color,
                );
            });

        if state.game_score == state.game_max_score {
            egui::Area::new("victory_hud")
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .order(egui::Order::Foreground)
                .interactable(false)
                .show(ctx, |ui| {
                    ui.painter().text(
                        ui.max_rect().center() + egui::vec2(5.0, 5.0),
                        egui::Align2::CENTER_CENTER,
                        "VICTORY!",
                        egui::FontId::proportional(120.0),
                        egui::Color32::from_black_alpha(200),
                    );
                    
                    ui.painter().text(
                        ui.max_rect().center(),
                        egui::Align2::CENTER_CENTER,
                        "VICTORY!",
                        egui::FontId::proportional(120.0),
                        egui::Color32::from_rgb(100, 200, 255), // Buz Mavisi
                    );
                });
        }
    }

    // Crosshair
    egui::Area::new("crosshair")
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .order(egui::Order::Foreground)
        .interactable(false)
        .show(ctx, |ui| {
            let rect = ui.allocate_space(egui::vec2(20.0, 20.0)).1;
            let c = rect.center();
            let stroke = egui::Stroke::new(2.0, egui::Color32::from_white_alpha(150));
            ui.painter().line_segment([c - egui::vec2(10.0, 0.0), c - egui::vec2(4.0, 0.0)], stroke);
            ui.painter().line_segment([c + egui::vec2(4.0, 0.0), c + egui::vec2(10.0, 0.0)], stroke);
            ui.painter().line_segment([c - egui::vec2(0.0, 10.0), c - egui::vec2(0.0, 4.0)], stroke);
            ui.painter().line_segment([c + egui::vec2(0.0, 4.0), c + egui::vec2(0.0, 10.0)], stroke);
        });
}
