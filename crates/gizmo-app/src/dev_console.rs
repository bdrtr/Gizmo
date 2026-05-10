use gizmo_core::cvar::{CVarRegistry, DevConsoleState};
use gizmo_core::world::World;

pub fn ui_dev_console(world: &mut World, ctx: &egui::Context, input: &gizmo_core::input::Input) {
    // Tilde tuşuna (Esc'nin altındaki tuş) basılınca konsolu aç/kapat.
    // Winit'te bu Backquote olarak geçiyor.
    if input.is_key_just_pressed(winit::keyboard::KeyCode::Backquote as u32) {
        let has_state = world.get_resource::<DevConsoleState>().is_some();
        if has_state {
            let mut state = world.get_resource_mut::<DevConsoleState>().unwrap();
            state.is_open = !state.is_open;
        } else {
            let mut state = DevConsoleState::default();
            state.is_open = true;
            world.insert_resource(state);
        }
    }

    let mut is_open = false;
    if let Some(state) = world.get_resource::<DevConsoleState>() {
        is_open = state.is_open;
    }

    if !is_open {
        return;
    }

    // Konsol penceresi ekranın üstünde, yarı saydam, animasyonlu inecek şekilde tasarlanır
    let window = egui::Window::new("Gizmo Developer Console")
        .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 0.0))
        .default_width(ctx.screen_rect().width())
        .fixed_pos(egui::pos2(0.0, 0.0))
        .collapsible(false)
        .title_bar(false)
        .resizable(false)
        .frame(egui::Frame::window(&ctx.style()).fill(egui::Color32::from_black_alpha(200)));

    window.show(ctx, |ui| {
        ui.add_space(5.0);

        // Logların görünümü
        egui::ScrollArea::vertical()
            .max_height(ctx.screen_rect().height() * 0.4) // Ekranın %40'ı kadar kaplar
            .stick_to_bottom(true)
            .show(ui, |ui| {
                if let Some(state) = world.get_resource::<DevConsoleState>() {
                    for line in &state.output_log {
                        if line.contains("Hata") || line.contains("Bilinmeyen") {
                            ui.colored_label(egui::Color32::RED, line);
                        } else if line.contains(" = ") {
                            ui.colored_label(egui::Color32::LIGHT_GREEN, line);
                        } else {
                            ui.colored_label(egui::Color32::WHITE, line);
                        }
                    }
                }

                // Keep some space at the bottom to avoid text cutoff
                ui.add_space(5.0);
            });

        ui.separator();

        let mut execute_cmd = None;

        if let Some(mut state) = world.get_resource_mut::<DevConsoleState>() {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(">")
                        .color(egui::Color32::WHITE)
                        .strong(),
                );

                let response = ui.add(
                    egui::TextEdit::singleline(&mut state.input_buffer)
                        .desired_width(ui.available_width())
                        .font(egui::TextStyle::Monospace)
                        .text_color(egui::Color32::WHITE)
                        .lock_focus(true),
                );

                // Focus the text edit automatically when opened
                response.request_focus();

                // `TextEdit::singleline` Egui'nin içindeki Enter eventini yutabilir.
                // Bu yüzden motorun kendi `input` modülünü kullanarak Enter'a basılıp basılmadığını kontrol ediyoruz.
                let enter_pressed =
                    input.is_key_just_pressed(winit::keyboard::KeyCode::Enter as u32);

                if enter_pressed && !state.input_buffer.trim().is_empty() {
                    execute_cmd = Some(state.input_buffer.clone());
                    state.input_buffer.clear();
                }
            });
        }

        // Execute command and apply to CVarRegistry
        if let Some(cmd) = execute_cmd {
            let result = if let Some(mut registry) = world.get_resource_mut::<CVarRegistry>() {
                registry.execute(&cmd)
            } else {
                "CVarRegistry bulunamadi. Motor henüz hazir değil.".to_string()
            };

            if let Some(mut state) = world.get_resource_mut::<DevConsoleState>() {
                state.output_log.push(format!("> {}", cmd));
                if result == "CLEAR_SCREEN_REQUEST" {
                    state.output_log.clear();
                } else if !result.is_empty() {
                    for line in result.lines() {
                        state.output_log.push(line.to_string());
                    }
                }
            }
        }
    });
}
