use gizmo::prelude::*;
use gizmo::egui;

pub fn render_main_menu(ctx: &egui::Context, world: &World) {
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

pub fn render_pause_menu(ctx: &egui::Context, world: &World) {
    egui::Area::new("pause_menu_bg")
        .anchor(egui::Align2::LEFT_TOP, egui::vec2(0.0, 0.0))
        .order(egui::Order::Background)
        .interactable(true) 
        .show(ctx, |ui| {
            let screen_rect = ctx.screen_rect();
            // Oyunu hafif karartarak pause hissi ver
            ui.painter().rect_filled(screen_rect, 0.0, egui::Color32::from_rgba_premultiplied(0, 0, 0, 150));
        });

    egui::Area::new("pause_menu_content")
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new("DURAKLATILDI").size(52.0).strong().color(egui::Color32::WHITE));
                ui.add_space(40.0);

                let btn_size = egui::vec2(250.0, 50.0);
                
                if ui.add_sized(btn_size, egui::Button::new(egui::RichText::new("Devam Et (Resume)").size(22.0))).clicked() {
                    if let Some(mut m) = world.get_resource_mut::<crate::state::AppMode>() {
                        *m = crate::state::AppMode::InGame;
                    }
                }
                ui.add_space(20.0);
                
                if ui.add_sized(btn_size, egui::Button::new(egui::RichText::new("Ana Menü (Main Menu)").size(22.0))).clicked() {
                    if let Some(mut m) = world.get_resource_mut::<crate::state::AppMode>() {
                        *m = crate::state::AppMode::MainMenu;
                    }
                }
                ui.add_space(20.0);

                if ui.add_sized(btn_size, egui::Button::new(egui::RichText::new("Çıkış").size(22.0))).clicked() {
                    std::process::exit(0);
                }
            });
        });
}
