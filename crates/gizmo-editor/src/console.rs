use gizmo_core::logger::{LogLevel, GLOBAL_LOGS};

pub fn ui_console(ui: &mut egui::Ui) {
    ui.heading("Geliştirici Konsolu");
    ui.separator();
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .show(ui, |ui| {
            if let Ok(logs) = GLOBAL_LOGS.lock() {
                for log in logs.iter() {
                    let color = match log.level {
                        LogLevel::Info => egui::Color32::WHITE,
                        LogLevel::Warning => egui::Color32::from_rgb(255, 200, 0),
                        LogLevel::Error => egui::Color32::RED,
                    };
                    ui.label(egui::RichText::new(&log.message).color(color));
                }
            }
        });
}
