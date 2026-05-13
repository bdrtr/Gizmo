use crate::editor_state::{ConsoleMode, EditorState};
use gizmo_core::logger::{self, LogLevel};

pub fn ui_console(ui: &mut egui::Ui, state: &mut EditorState) {
    // Konsol sekmeleri (Engine vs Build)
    ui.horizontal(|ui| {
        ui.selectable_value(&mut state.console.mode, ConsoleMode::EngineLogs, "📝 Motor Logları");
        ui.selectable_value(&mut state.console.mode, ConsoleMode::BuildOutput, "🔨 Derleme Çıktısı");
    });
    ui.separator();

    match state.console.mode {
        ConsoleMode::EngineLogs => draw_engine_logs(ui, state),
        ConsoleMode::BuildOutput => crate::windows::ui_build_console(ui, state),
    }
}

fn draw_engine_logs(ui: &mut egui::Ui, state: &mut EditorState) {
    let current_version = logger::get_log_version();
    let mut filter_changed = false;

    // Top Bar UI
    ui.horizontal(|ui| {
        ui.label(format!(
            "{} Log",
            state.console.count_info + state.console.count_warn + state.console.count_error
        ));

        ui.separator();

        if ui.button("🗑 Temizle").clicked() {
            logger::clear_logs();
        }

        ui.separator();

        let prev_info = state.console.show_info;
        ui.toggle_value(&mut state.console.show_info, format!("ℹ Info ({})", state.console.count_info));
        if prev_info != state.console.show_info { filter_changed = true; }

        let prev_warn = state.console.show_warn;
        ui.toggle_value(&mut state.console.show_warn, format!("⚠️ Warn ({})", state.console.count_warn));
        if prev_warn != state.console.show_warn { filter_changed = true; }

        let prev_error = state.console.show_error;
        ui.toggle_value(&mut state.console.show_error, format!("🔴 Error ({})", state.console.count_error));
        if prev_error != state.console.show_error { filter_changed = true; }

        ui.separator();

        ui.label("🔍");
        let response = ui.text_edit_singleline(&mut state.console.filter_text);
        if response.changed() { filter_changed = true; }
    });

    ui.separator();

    if current_version != state.console.last_version || filter_changed {
        logger::get_logs(|logs| {
            let mut info_cnt = 0;
            let mut warn_cnt = 0;
            let mut err_cnt = 0;

            let filter_lower = state.console.filter_text.to_lowercase();
            state.console.cached_logs.clear();

            for log in logs {
                match log.level {
                    LogLevel::Info => info_cnt += 1,
                    LogLevel::Warning => warn_cnt += 1,
                    LogLevel::Error => err_cnt += 1,
                }

                // Filtering pass
                if !state.console.show_info && log.level == LogLevel::Info { continue; }
                if !state.console.show_warn && log.level == LogLevel::Warning { continue; }
                if !state.console.show_error && log.level == LogLevel::Error { continue; }
                if !filter_lower.is_empty() && !log.message.to_lowercase().contains(&filter_lower) { continue; }

                state.console.cached_logs.push(log.clone());
            }

            state.console.count_info = info_cnt;
            state.console.count_warn = warn_cnt;
            state.console.count_error = err_cnt;
        });
        state.console.last_version = current_version;
    }

    let row_height = 22.0;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .show_rows(
            ui,
            row_height,
            state.console.cached_logs.len(),
            |ui, row_range| {
                for i in row_range {
                    let log = &state.console.cached_logs[i];
                    let (text_color, bg_color, icon) = match log.level {
                        LogLevel::Info => (egui::Color32::WHITE, egui::Color32::TRANSPARENT, "ℹ"),
                        LogLevel::Warning => (egui::Color32::from_rgb(255, 200, 0), egui::Color32::from_rgba_unmultiplied(255, 200, 0, 15), "⚠️"),
                        LogLevel::Error => (egui::Color32::RED, egui::Color32::from_rgba_unmultiplied(255, 0, 0, 20), "🔴"),
                    };

                    let text = format!("[{}] {} {}", log.timestamp, icon, log.message);
                    
                    let frame = egui::Frame::none()
                        .fill(bg_color)
                        .inner_margin(egui::Margin::symmetric(4.0, 2.0));
                        
                    let response = frame.show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(egui::RichText::new(&text).color(text_color).family(egui::FontFamily::Monospace))
                    }).response;

                    let interact_response = ui.interact(response.rect, response.id.with("interact"), egui::Sense::click());
                    if interact_response.clicked() {
                        ui.output_mut(|o| o.copied_text = text.clone());
                    }
                    interact_response.on_hover_text("Tıkla: Panoya kopyala");
                }
            },
        );
}
