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
    let current_version = logger::log_version();
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

        // The prototype's chips: label plus count, the active one an accent fill. `toggle_value`
        // with an emoji prefix was the old form — and the ℹ/⚠️/🔴 glyphs render as boxes in egui's
        // bundled font, which is why they are gone rather than restyled.
        if crate::theme::toggle(
            ui,
            &mut state.console.show_info,
            &format!("Info {}", state.console.count_info),
        ) {
            filter_changed = true;
        }

        if crate::theme::toggle(
            ui,
            &mut state.console.show_warn,
            &format!("Warn {}", state.console.count_warn),
        ) {
            filter_changed = true;
        }

        if crate::theme::toggle(
            ui,
            &mut state.console.show_error,
            &format!("Error {}", state.console.count_error),
        ) {
            filter_changed = true;
        }

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
                        // From the palette, not invented here — and warnings are amber, matching
                        // both their own `⚠` icon and the `⚠` labels elsewhere in the editor.
                        LogLevel::Info => (crate::theme::palette::TEXT, egui::Color32::TRANSPARENT, "ℹ"),
                        LogLevel::Warning => (
                            crate::theme::palette::WARNING,
                            crate::theme::palette::WARNING_WASH,
                            "⚠",
                        ),
                        LogLevel::Error => (
                            crate::theme::palette::DANGER,
                            crate::theme::palette::DANGER_WASH,
                            "🔴",
                        ),
                    };

                    let text = format!("[{}] {} {}", log.timestamp, icon, log.message);
                    
                    let frame = egui::Frame::new()
                        .fill(bg_color)
                        .inner_margin(egui::Margin::symmetric(4, 2));
                        
                    let response = frame.show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(egui::RichText::new(&text).color(text_color).family(egui::FontFamily::Monospace))
                    }).response;

                    let interact_response = ui.interact(response.rect, response.id.with("interact"), egui::Sense::click());
                    if interact_response.clicked() {
                        ui.output_mut(|o| o.commands.push(egui::OutputCommand::CopyText(text.clone())));
                    }
                    interact_response.on_hover_text("Tıkla: Panoya kopyala");
                }
            },
        );
}

#[cfg(test)]
mod console_colour_tests {
    use crate::theme::palette;

    /// A warning must not be drawn in the colour that means "fine".
    ///
    /// The console drew warnings green — text and row tint — while labelling them `⚠️`, and while
    /// the rest of the editor was already using yellow for the same idea (`asset_browser`'s
    /// "⚠ Asset dizini bulunamadı", the fight HUD's amber). A console of warnings scanned as a
    /// console of successes, and today's work routes more warnings into it than ever.
    #[test]
    fn a_warning_is_not_painted_green() {
        let (r, g, b, _) = palette::WARNING.to_tuple();
        assert!(
            r >= g,
            "the warning colour is greener than it is red ({r}, {g}, {b}) — that is the colour of \
             success, not of a warning"
        );
        assert!(
            r > b && g > b,
            "the warning colour must sit in the amber/yellow family, not blue-ward: ({r}, {g}, {b})"
        );
    }

    /// The three levels have to be distinguishable at a glance, which is the entire job of the
    /// colour: `Info` is unremarkable text, `Warning` is amber, `Error` is red.
    #[test]
    fn the_three_levels_are_told_apart_by_colour() {
        let info = palette::TEXT;
        let warn = palette::WARNING;
        let error = palette::DANGER;
        assert_ne!(info, warn);
        assert_ne!(warn, error);
        assert_ne!(info, error);

        let (wr, wg, _, _) = warn.to_tuple();
        let (er, eg, _, _) = error.to_tuple();
        assert!(
            er as i32 - eg as i32 > wr as i32 - wg as i32,
            "the error colour is not redder than the warning colour, so the two rows read the same"
        );
    }

    /// And the panel actually paints with those colours.
    ///
    /// The constants above only matter if the console reads them; it used to hold its own inline
    /// `from_rgb(100, 255, 100)`. This drives a real console frame with one warning and one error
    /// in the log and reads the colours back out of the shapes egui emitted.
    #[test]
    fn the_console_paints_warnings_and_errors_in_those_colours() {
        use gizmo_core::logger::{self, LogLevel};

        let marker = format!("renk-testi-{}", std::process::id());
        logger::log_message(LogLevel::Warning, format!("{marker}-uyari"), "test.rs", 1);
        logger::log_message(LogLevel::Error, format!("{marker}-hata"), "test.rs", 2);

        let mut state = crate::EditorState::default();
        let ctx = egui::Context::default();
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let mut panel = ui.new_child(egui::UiBuilder::new().max_rect(
                egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(880.0, 1200.0)),
            ));
            super::ui_console(&mut panel, &mut state);
        });

        // (text, colour) for everything the frame drew.
        fn collect(shape: &egui::Shape, out: &mut Vec<(String, egui::Color32)>) {
            match shape {
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, out)),
                egui::Shape::Text(t) => {
                    let colour = t.override_text_color.unwrap_or_else(|| {
                        t.galley
                            .job
                            .sections
                            .first()
                            .map(|s| s.format.color)
                            .unwrap_or(egui::Color32::PLACEHOLDER)
                    });
                    out.push((t.galley.text().to_owned(), colour));
                }
                _ => {}
            }
        }
        let mut painted = Vec::new();
        output.shapes.iter().for_each(|s| collect(&s.shape, &mut painted));
        output.drop_without_applying_deltas();

        let colour_of = |needle: &str| {
            painted
                .iter()
                .find(|(text, _)| text.contains(needle))
                .map(|(_, c)| *c)
        };

        assert_eq!(
            colour_of(&format!("{marker}-uyari")),
            Some(palette::WARNING),
            "the console did not paint the warning row in the palette's warning colour"
        );
        assert_eq!(
            colour_of(&format!("{marker}-hata")),
            Some(palette::DANGER),
            "the console did not paint the error row in the palette's error colour"
        );
    }

    /// Errors must not wear the accent, which in this editor means "selected".
    #[test]
    fn an_error_does_not_look_like_a_selection() {
        assert_ne!(
            palette::DANGER,
            palette::ACCENT,
            "errors painted in the selection colour make every error look like the row you clicked"
        );
    }
}
