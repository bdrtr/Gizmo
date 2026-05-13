use eframe::egui;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([400.0, 400.0]),
        ..Default::default()
    };
    eframe::run_simple_native("Test", options, |ctx, _frame| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let bg_response = ui.interact(
                ui.available_rect_before_wrap(),
                ui.id().with("bg"),
                egui::Sense::click_and_drag(),
            );
            if bg_response.clicked() {
                println!("BG CLICKED");
            }

            egui::ScrollArea::vertical().show(ui, |ui| {
                if ui.selectable_label(false, "Test Label").clicked() {
                    println!("LABEL CLICKED");
                }
            });
        });
    })
}
