pub mod hud;
pub mod menus;
pub mod theme;

use crate::state::GameState;
use gizmo::egui;
use gizmo::prelude::*;

pub fn render_ui(ctx: &egui::Context, state: &mut GameState, world: &World) {
    // 1. Temayı her frame (basitçe) uygula
    theme::setup_modern_theme(ctx);

    // HUD ve Menüler
    let mode = world
        .get_resource::<crate::state::AppMode>()
        .map(|m| *m)
        .unwrap_or(crate::state::AppMode::InGame);

    if mode == crate::state::AppMode::InGame {
        hud::render_game_hud(ctx, world, state);
    } else if mode == crate::state::AppMode::MainMenu {
        menus::render_main_menu(ctx, world);
    } else if mode == crate::state::AppMode::PauseMenu {
        menus::render_pause_menu(ctx, world);
    }

    if state.show_devtools {
        render_devtools(ctx, state, world);
    }
}

fn render_devtools(ctx: &egui::Context, state: &mut GameState, world: &World) {
    // --- PROFILER OVERLAY ---
    egui::Window::new("Profiler")
        .fixed_pos([240.0, 50.0])
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .frame(egui::Frame::window(&ctx.style()).fill(egui::Color32::from_black_alpha(200)))
        .show(ctx, |ui| {
            ui.label(
                egui::RichText::new("📊 BİLGİ EKRANI")
                    .color(egui::Color32::WHITE)
                    .strong(),
            );
            ui.separator();

            let fps = state.current_fps;
            let ms = if fps > 0.0 { 1000.0 / fps } else { 0.0 };

            let fps_color = if fps >= 55.0 {
                egui::Color32::GREEN
            } else if fps >= 30.0 {
                egui::Color32::YELLOW
            } else {
                egui::Color32::RED
            };

            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("FPS:").color(egui::Color32::LIGHT_GRAY));
                ui.label(
                    egui::RichText::new(format!("{:.1}", fps))
                        .color(fps_color)
                        .strong(),
                );
                ui.label(egui::RichText::new(format!("({:.2} ms)", ms)).color(egui::Color32::GRAY));
            });

            let entity_count = world.entity_count();
            let draw_calls = world
                .query_ref::<gizmo::renderer::components::MeshRenderer>()
                .map(|q| q.s1.dense.len())
                .unwrap_or(0);

            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Varlık (Entity):").color(egui::Color32::LIGHT_GRAY));
                ui.label(
                    egui::RichText::new(format!("{}", entity_count))
                        .color(egui::Color32::WHITE)
                        .strong(),
                );
            });

            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("Çizim Çağrısı (Draw Calls):")
                        .color(egui::Color32::LIGHT_GRAY),
                );
                ui.label(
                    egui::RichText::new(format!("{}", draw_calls + 1))
                        .color(egui::Color32::WHITE)
                        .strong(),
                );
            });

            ui.separator();
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Fizik Hızı (FPS):").color(egui::Color32::LIGHT_GRAY));
                ui.add(egui::Slider::new(
                    &mut state.target_physics_fps,
                    5.0..=240.0,
                ));
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
                state.free_cam = true;
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
                if let Some(names) = world.borrow::<crate::EntityName>() {
                    for entity in world.iter_alive_entities() {
                        if let Some(n) = names.get(entity.id()) {
                            if n.0.contains("Mermi -") {
                                targets.push(entity.id());
                            }
                        }
                    }
                }
                if let Some(mut trans) = world.borrow_mut::<gizmo::physics::components::Transform>() {
                    for &id in &targets {
                        if let Some(t) = trans.get_mut(id) {
                            t.position.x = -15.0;
                            t.position.y = 3.0;
                        }
                    }
                }
                if let Some(mut vels) = world.borrow_mut::<gizmo::physics::components::Velocity>() {
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
