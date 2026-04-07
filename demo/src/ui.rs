use gizmo::prelude::*;
use gizmo::egui;
use crate::state::{GameState, RaceStatus};

pub fn render_ui(ctx: &egui::Context, state: &mut GameState, world: &World) {
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
                state.spawn_domino_requests.set(1);
                state.free_cam = true; // Domino effect için otomatik serbest kameraya geçsin
            }
            if ui.button("▶️ Domino Başlat").clicked() {
                state.release_domino_requests.set(1);
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
                                    state.asset_spawn_requests.borrow_mut().push(path.to_string_lossy().to_string());
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
            let mut pp = state.post_process_settings.borrow_mut();
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
