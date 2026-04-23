use crate::editor_state::EditorState;
use gizmo_core::logger::{self, LogLevel};

pub fn ui_console(ui: &mut egui::Ui, state: &mut EditorState) {
    let current_version = logger::get_log_version();
    let mut filter_changed = false;

    // Top Bar UI
    ui.horizontal(|ui| {
        ui.heading("Geliştirici Konsolu");
        ui.label(format!(
            "({} Toplam)",
            state.console.count_info + state.console.count_warn + state.console.count_error
        ));

        ui.separator();

        if ui.button("🗑 Temizle").clicked() {
            logger::clear_logs();
        }

        ui.separator();

        let prev_info = state.console.show_info;
        ui.checkbox(
            &mut state.console.show_info,
            format!("ℹ Info ({})", state.console.count_info),
        );
        if prev_info != state.console.show_info {
            filter_changed = true;
        }

        let prev_warn = state.console.show_warn;
        ui.checkbox(
            &mut state.console.show_warn,
            format!("⚠️ Warn ({})", state.console.count_warn),
        );
        if prev_warn != state.console.show_warn {
            filter_changed = true;
        }

        let prev_error = state.console.show_error;
        ui.checkbox(
            &mut state.console.show_error,
            format!("🔴 Error ({})", state.console.count_error),
        );
        if prev_error != state.console.show_error {
            filter_changed = true;
        }

        ui.separator();

        ui.label("🔍");
        let response = ui.text_edit_singleline(&mut state.console.filter_text);
        if response.changed() {
            filter_changed = true;
        }
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
                if !state.console.show_info && log.level == LogLevel::Info {
                    continue;
                }
                if !state.console.show_warn && log.level == LogLevel::Warning {
                    continue;
                }
                if !state.console.show_error && log.level == LogLevel::Error {
                    continue;
                }

                if !filter_lower.is_empty() && !log.message.to_lowercase().contains(&filter_lower) {
                    continue;
                }

                state.console.cached_logs.push(log.clone());
            }

            state.console.count_info = info_cnt;
            state.console.count_warn = warn_cnt;
            state.console.count_error = err_cnt;
        });
        state.console.last_version = current_version;
    }

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .show_rows(
            ui,
            18.0,
            state.console.cached_logs.len(),
            |ui, row_range| {
                for i in row_range {
                    let log = &state.console.cached_logs[i];
                    let (color, icon) = match log.level {
                        LogLevel::Info => (egui::Color32::WHITE, "ℹ"),
                        LogLevel::Warning => (egui::Color32::from_rgb(255, 200, 0), "⚠️"),
                        LogLevel::Error => (egui::Color32::RED, "🔴"),
                    };

                    let text = format!("[{}] {} {}", log.timestamp, icon, log.message);

                    let label = egui::Label::new(egui::RichText::new(&text).color(color))
                        .wrap()
                        .sense(egui::Sense::click());
                    let response = ui.add(label);

                    if response.clicked() {
                        ui.output_mut(|o| o.copied_text = text.clone());
                    }
                    response.on_hover_text("Tıkla: Panoya kopyala");
                }
            },
        );
}
