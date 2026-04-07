use gizmo::prelude::*;
use gizmo::egui;
use crate::state::{GameState, RaceStatus};

pub fn render_ui(ctx: &egui::Context, state: &mut GameState, world: &World) {
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
                .fill(egui::Color32::from_rgba_premultiplied(10, 10, 20, 230))
                .rounding(egui::Rounding::same(12.0)))
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
    if state.race_status != RaceStatus::Idle || !state.checkpoints.is_empty() {
        let cp_done = state.checkpoints.iter().filter(|c| c.activated).count();
        let cp_total = state.checkpoints.len();

        egui::Window::new("##race_hud")
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-20.0, 20.0))
            .title_bar(false)
            .resizable(false)
            .collapsible(false)
            .frame(egui::Frame::window(&ctx.style())
                .fill(egui::Color32::from_rgba_premultiplied(0, 0, 0, 180))
                .rounding(egui::Rounding::same(10.0)))
            .show(ctx, |ui| {
                let status_txt = match state.race_status {
                    RaceStatus::Idle     => "⏸ Hazır",
                    RaceStatus::Running  => "🏁 Yarış!",
                    RaceStatus::Finished => "🏆 Bitti!",
                };
                ui.label(egui::RichText::new(status_txt).color(egui::Color32::WHITE).strong().size(18.0));
                ui.separator();
                let mins = (state.race_timer / 60.0) as u32;
                let secs = state.race_timer % 60.0;
                ui.label(egui::RichText::new(format!("⏱ {:02}:{:05.2}", mins, secs))
                    .color(egui::Color32::from_rgb(100, 255, 150)).size(22.0).strong());
                if cp_total > 0 {
                    ui.separator();
                    ui.label(egui::RichText::new(format!("📍 {}/{}", cp_done, cp_total))
                        .color(egui::Color32::LIGHT_BLUE).size(14.0));
                    ui.add(egui::ProgressBar::new(cp_done as f32 / cp_total as f32)
                        .desired_width(130.0)
                        .fill(egui::Color32::from_rgb(50, 200, 100)));
                }
            });
    }

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
                    ui.label(egui::RichText::new("NoroNest Pre-Alpha").size(24.0).color(egui::Color32::from_rgb(255, 200, 80)));
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
