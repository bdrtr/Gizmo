use gizmo::prelude::*;
use gizmo::egui;
use crate::state::GameState;

pub fn setup_modern_theme(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    
    // Unity / Unreal tarzı renk paleti
    style.visuals.window_fill = egui::Color32::from_rgb(32, 32, 34);
    style.visuals.panel_fill = egui::Color32::from_rgb(42, 42, 44);
    style.visuals.faint_bg_color = egui::Color32::from_rgb(25, 25, 27);
    style.visuals.extreme_bg_color = egui::Color32::from_rgb(18, 18, 20);

    style.visuals.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(42, 42, 44);
    style.visuals.widgets.inactive.bg_fill = egui::Color32::from_rgb(55, 55, 58);
    style.visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(67, 67, 70);
    style.visuals.widgets.active.bg_fill = egui::Color32::from_rgb(80, 120, 200); // Vurgu Rengi: Açık Mavi

    style.visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(60, 60, 62));
    style.visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(70, 70, 72));
    style.visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(100, 100, 102));

    style.visuals.widgets.noninteractive.fg_stroke.color = egui::Color32::from_rgb(180, 180, 180);
    style.visuals.widgets.inactive.fg_stroke.color = egui::Color32::from_rgb(210, 210, 210);
    style.visuals.widgets.hovered.fg_stroke.color = egui::Color32::WHITE;
    style.visuals.widgets.active.fg_stroke.color = egui::Color32::WHITE;

    style.visuals.selection.bg_fill = egui::Color32::from_rgb(60, 100, 200);
    style.visuals.selection.stroke.color = egui::Color32::WHITE;

    // Keskin, profesyonel kenarlar (Rounding=4.0)
    style.visuals.window_rounding = egui::Rounding::same(4.0);
    style.visuals.widgets.noninteractive.rounding = egui::Rounding::same(3.0);
    style.visuals.widgets.inactive.rounding = egui::Rounding::same(3.0);
    style.visuals.widgets.hovered.rounding = egui::Rounding::same(3.0);
    style.visuals.widgets.active.rounding = egui::Rounding::same(3.0);

    // Boşluklar
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.window_margin = egui::style::Margin::same(12.0);

    ctx.set_style(style);
}

pub fn render_ui(ctx: &egui::Context, state: &mut GameState, world: &World) {
    // 1. Temayı her frame (basitçe) uygula
    setup_modern_theme(ctx);

    if let Some(mut editor_state) = world.get_resource_mut::<gizmo::editor::EditorState>() {
        gizmo::editor::draw_editor(ctx, world, &mut editor_state);
    }
    
    // YENİ HUD SİSTEMİ ÇAĞRISI
    render_game_ui(ctx, world);

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
        // Yardımcı fonksiyon: Gölgede metin çizmek için
        let draw_shadow_text = |ui: &mut egui::Ui, text: String, pos: egui::Pos2, size: f32, color: egui::Color32| {
            let font = egui::FontId::proportional(size);
            // Gölge
            ui.painter().text(pos + egui::vec2(2.0, 3.0), egui::Align2::CENTER_CENTER, &text, font.clone(), egui::Color32::from_black_alpha(150));
            // Ana Metin
            ui.painter().text(pos, egui::Align2::CENTER_CENTER, &text, font, color);
        };

        let screen_rect = ctx.screen_rect();

        // Geri sayım (Countdown) - Merkezi, Büyük ve Gölgeli
        if race.phase == crate::race::RacePhase::Countdown {
            let txt = crate::race::countdown_text(race);
            if !txt.is_empty() {
                egui::Area::new("countdown_hud")
                    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, -80.0))
                    .order(egui::Order::Foreground)
                    .interactable(false)
                    .show(ctx, |ui| {
                        // Dinamik boyut (zaman küçüldükçe hafif büyüme efekti / yaylanma)
                        let time_fract = race.countdown_timer.fract();
                        let pulse = 1.0 + (time_fract * std::f32::consts::PI).sin() * 0.2;
                        let size = 120.0 * (if txt == "GO!" { 1.5 } else { pulse });
                        
                        let color = if txt == "GO!" { 
                            egui::Color32::from_rgb(50, 255, 100) 
                        } else { 
                            egui::Color32::from_rgb(255, 200, 50) 
                        };

                        let pos = ui.max_rect().center(); // Alanın merkezi
                        draw_shadow_text(ui, txt.to_string(), pos, size, color);
                    });
            }
        }

        if race.phase == crate::race::RacePhase::Racing {
            // --- Hız Göstergesi (Sağ Alt) ---
            egui::Area::new("race_speedometer")
                .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-40.0, -40.0))
                .order(egui::Order::Foreground)
                .interactable(false)
                .show(ctx, |ui| {
                    let desired_size = egui::vec2(200.0, 80.0);
                    let (rect, _) = ui.allocate_exact_size(desired_size, egui::Sense::hover());
                    
                    // Şık yarı saydam arka plan (Cam efekti)
                    ui.painter().rect_filled(rect, 16.0, egui::Color32::from_rgba_premultiplied(10, 15, 25, 200));
                    ui.painter().rect_stroke(rect, 16.0, egui::Stroke::new(2.0, egui::Color32::from_white_alpha(30)));
                    
                    let speed = crate::race::get_speed_kmh(world, race.player_entity);
                    
                    // "KM/H" etiket
                    ui.painter().text(
                        rect.right_center() + egui::vec2(-20.0, 15.0),
                        egui::Align2::RIGHT_BOTTOM,
                        "KM/H",
                        egui::FontId::proportional(20.0),
                        egui::Color32::from_gray(150),
                    );

                    // Hız Rakamları (Dinamik ve büyük)
                    let speed_color = if speed > 150.0 { egui::Color32::from_rgb(255, 80, 50) } // Yüksek hızda kırmızıya dönük
                                      else { egui::Color32::from_rgb(200, 240, 255) }; 
                    ui.painter().text(
                        rect.right_center() + egui::vec2(-20.0, -10.0),
                        egui::Align2::RIGHT_BOTTOM,
                        format!("{:.0}", speed),
                        egui::FontId::proportional(64.0),
                        speed_color,
                    );
                });

            // --- Sıralama ve Tur (Sol Üst / Sağ Üst) ---
            egui::Area::new("race_status_top")
                .anchor(egui::Align2::LEFT_TOP, egui::vec2(40.0, 40.0))
                .order(egui::Order::Foreground)
                .interactable(false)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        // POSITION BOX
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

                        // LAPS BOX
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

            // --- Yarış Süresi (Merkez Üst) ---
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

        // --- Bitiş Ekranı (Finish Screen) ---
        if race.phase == crate::race::RacePhase::Finished {
            // Arkaplanı karart
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

                    // Başlık
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

                    // Sıralama Listesi
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
                    
                    // Alt Kısımda Talimat
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

    if state.show_devtools {
        // --- PROFILER OVERLAY ---
        egui::Window::new("Profiler")
            .fixed_pos([240.0, 50.0])
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .frame(egui::Frame::window(&ctx.style()).fill(egui::Color32::from_black_alpha(200)))
            .show(ctx, |ui| {
                ui.label(egui::RichText::new("📊 BİLGİ EKRANI").color(egui::Color32::WHITE).strong());
                ui.separator();
                
                let fps = state.current_fps;
                let ms = if fps > 0.0 { 1000.0 / fps } else { 0.0 };
                
                // Renklendirme mantığı (FPS)
                let fps_color = if fps >= 55.0 { egui::Color32::GREEN }
                                else if fps >= 30.0 { egui::Color32::YELLOW }
                                else { egui::Color32::RED };

                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("FPS:").color(egui::Color32::LIGHT_GRAY));
                    ui.label(egui::RichText::new(format!("{:.1}", fps)).color(fps_color).strong());
                    ui.label(egui::RichText::new(format!("({:.2} ms)", ms)).color(egui::Color32::GRAY));
                });
                
                let entity_count = world.entity_count();
                let draw_calls = world.query_ref::<gizmo::renderer::components::MeshRenderer>()
                                      .map(|q| q.s1.dense.len())
                                      .unwrap_or(0);
                                      
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Varlık (Entity):").color(egui::Color32::LIGHT_GRAY));
                    ui.label(egui::RichText::new(format!("{}", entity_count)).color(egui::Color32::WHITE).strong());
                });

                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Çizim Çağrısı (Draw Calls):").color(egui::Color32::LIGHT_GRAY));
                    ui.label(egui::RichText::new(format!("{}", draw_calls + 1)).color(egui::Color32::WHITE).strong()); // +1 Shadow Pass
                });
                
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Fizik Hızı (FPS):").color(egui::Color32::LIGHT_GRAY));
                    ui.add(egui::Slider::new(&mut state.target_physics_fps, 5.0..=240.0));
                });
            });

        // --- DEMO DEVTOOLS ---
        egui::Window::new("Demo DevTools (Stres & Görsel test)")
            .default_pos([240.0, 150.0])
            .show(ctx, |ui| {
                ui.heading("Stres Testleri (Fizik)");

                if ui.button("🎯 Domino Etkisi (Sıfırla ve Kur)").clicked() {
                    if let Some(mut events) = world.get_resource_mut::<gizmo::core::event::Events<crate::state::SpawnDominoEvent>>() {
                        events.push(crate::state::SpawnDominoEvent { count: 1 });
                    }
                    state.free_cam = true; // Domino effect için otomatik serbest kameraya geçsin
                }
                if ui.button("▶️ Domino Başlat").clicked() {
                    if let Some(mut events) = world.get_resource_mut::<gizmo::core::event::Events<crate::state::ReleaseDominoEvent>>() {
                        events.push(crate::state::ReleaseDominoEvent { count: 1 });
                    }
                }
                ui.checkbox(&mut state.free_cam, "Serbest Kamera Modu (Sol Mouse ile çevreye bak ve WASD)");
                
                ui.separator();
                ui.heading("Varlıklar (Assets)");
                egui::ScrollArea::vertical().id_source("assets_demo").max_height(100.0).show(ui, |ui| {
                    if let Ok(entries) = std::fs::read_dir("demo/assets") {
                        for entry in entries.flatten() {
                            let path = entry.path();
                            if path.is_file() {
                                let ext = path.extension().unwrap_or_default().to_string_lossy().to_string();
                                if ext == "glb" || ext == "gltf" || ext == "obj" {
                                    let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                                    if ui.button(format!("📦 {}", name)).clicked() {
                                        if let Some(mut events) = world.get_resource_mut::<gizmo::core::event::Events<crate::state::AssetSpawnEvent>>() {
                                            events.push(crate::state::AssetSpawnEvent { path: path.to_string_lossy().to_string() });
                                        }
                                    }
                                }
                            }
                        }
                    } else {
                        ui.label("No assets folder found.");
                    }
                });

                ui.separator();
                ui.heading("Sinematik Efektler (Post-Process)");
                if let Some(mut pp) = world.get_resource_mut::<gizmo::renderer::renderer::PostProcessUniforms>() {
                    ui.horizontal(|ui| {
                        ui.label("Bloom Şiddeti:");
                        ui.add(egui::Slider::new(&mut pp.bloom_intensity, 0.0..=2.0));
                    });
                    ui.horizontal(|ui| {
                        ui.label("Bloom Eşiği:");
                        ui.add(egui::Slider::new(&mut pp.bloom_threshold, 0.1..=5.0));
                    });
                    ui.horizontal(|ui| {
                        ui.label("Pozlama (Exposure):");
                        ui.add(egui::Slider::new(&mut pp.exposure, 0.1..=5.0));
                    });
                    ui.horizontal(|ui| {
                        ui.label("Renk Sapması (Chromatic):");
                        ui.add(egui::Slider::new(&mut pp.chromatic_aberration, 0.0..=5.0));
                    });
                    ui.horizontal(|ui| {
                        ui.label("Köşe Karartması (Vignette):");
                        ui.add(egui::Slider::new(&mut pp.vignette_intensity, 0.0..=2.0));
                    });
                }

                ui.separator();
                if ui.button("🔄 Sahneleri Başa Sar (Testi Tekrarlat)").clicked() {
                    let mut targets = Vec::new();
                    if let Some(names) = world.borrow::<EntityName>() {
                        for entity in world.iter_alive_entities() {
                            if let Some(n) = names.get(entity.id()) {
                                if n.0.contains("Mermi -") {
                                    targets.push(entity.id());
                                }
                            }
                        }
                    }
                    if let Some(mut trans) = world.borrow_mut::<Transform>() {
                        for &id in &targets {
                            if let Some(t) = trans.get_mut(id) {
                                t.position.x = -15.0;
                                t.position.y = 3.0;
                            }
                        }
                    }
                    if let Some(mut vels) = world.borrow_mut::<Velocity>() {
                        for &id in &targets {
                            if let Some(v) = vels.get_mut(id) {
                                v.linear = Vec3::new(50.0, 0.0, 0.0);
                                v.angular = Vec3::ZERO;
                            }
                        }
                    }
                }
            });
    }
}

pub fn render_game_ui(ctx: &egui::Context, world: &World) {
    let mode = world.get_resource::<crate::state::AppMode>().map(|m| *m).unwrap_or(crate::state::AppMode::InGame);
    
    // --- HUD (InGame) ---
    if mode == crate::state::AppMode::InGame {
        if let Some(stats) = world.get_resource::<crate::state::PlayerStats>() {
            let health_pct = (stats.health / stats.max_health).clamp(0.0, 1.0);
            
            // Health Bar (Sol Alt, Yuvarlatılmış Modern Tasarım)
            egui::Area::new("health_hud")
                .anchor(egui::Align2::LEFT_BOTTOM, egui::vec2(30.0, -30.0))
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    let desired_size = egui::vec2(250.0, 24.0);
                    let (rect, _resp) = ui.allocate_exact_size(desired_size, egui::Sense::hover());
                    
                    // Arkaplan
                    ui.painter().rect_filled(rect, 12.0, egui::Color32::from_rgba_premultiplied(20, 20, 20, 150));
                    
                    // Dolum
                    let mut fill_rect = rect;
                    fill_rect.max.x = rect.min.x + (rect.width() * health_pct);
                    let color = if health_pct > 0.5 { egui::Color32::from_rgb(40, 200, 80) } 
                                else if health_pct > 0.2 { egui::Color32::from_rgb(220, 150, 20) } 
                                else { egui::Color32::from_rgb(220, 40, 40) };
                    
                    ui.painter().rect_filled(fill_rect, 12.0, color);
                    
                    // Yazı
                    let text = format!("+ {:.0}", stats.health);
                    ui.painter().text(
                        rect.left_center() + egui::vec2(15.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        text,
                        egui::FontId::proportional(16.0),
                        egui::Color32::WHITE,
                    );
                });

            // Ammo Counter (Sağ Alt)
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

        // Crosshair (Nişangah Merkezi)
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

    // --- MAIN MENU ---
    if mode == crate::state::AppMode::MainMenu {
        // Ekranı karartarak tıklamaları engelle
        egui::Area::new("main_menu_bg")
            .anchor(egui::Align2::LEFT_TOP, egui::vec2(0.0, 0.0))
            .order(egui::Order::Background)
            .interactable(true) 
            .show(ctx, |ui| {
                let screen_rect = ctx.screen_rect();
                ui.painter().rect_filled(screen_rect, 0.0, egui::Color32::from_rgba_premultiplied(10, 10, 12, 230));
            });

        // Ortalı Menü Öğeleri
        egui::Area::new("main_menu_content")
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.label(egui::RichText::new("GIZMO ENGINE").size(72.0).strong().color(egui::Color32::WHITE));
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new("NoroNest Pre-Alpha").size(24.0).color(egui::Color32::from_rgb(100, 150, 255)));
                    ui.add_space(60.0);

                    let btn_size = egui::vec2(250.0, 50.0);
                    
                    if ui.add_sized(btn_size, egui::Button::new(egui::RichText::new("Oyuna Başla").size(22.0))).clicked() {
                        if let Some(mut m) = world.get_resource_mut::<crate::state::AppMode>() {
                            *m = crate::state::AppMode::InGame;
                        }
                    }
                    ui.add_space(20.0);
                    
                    if ui.add_sized(btn_size, egui::Button::new(egui::RichText::new("Ayarlar").size(22.0))).clicked() {
                        // TODO: Settings
                    }
                    ui.add_space(20.0);
                    
                    if ui.add_sized(btn_size, egui::Button::new(egui::RichText::new("Çıkış").size(22.0))).clicked() {
                        std::process::exit(0);
                    }
                });
            });
    }
}
