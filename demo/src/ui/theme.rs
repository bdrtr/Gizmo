use gizmo::egui;

pub fn setup_modern_theme(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    
    // Unity / Unreal tarzı renk paleti
    style.visuals.window_fill = egui::Color32::from_rgb(32, 32, 34);
    style.visuals.panel_fill = egui::Color32::from_rgb(42, 42, 44);
    style.visuals.faint_bg_color = egui::Color32::from_rgb(25, 25, 27);
    style.visuals.extreme_bg_color = egui::Color32::from_rgb(18, 18, 20);

    style.visuals.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(42, 42, 44);
    style.visuals.widgets.inactive.bg_fill = egui::Color32::from_rgb(55, 55, 58);
    style.visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(67, 67, 70);
    style.visuals.widgets.active.bg_fill = egui::Color32::from_rgb(80, 120, 200); // Vurgu Rengi: Açık Mavi

    style.visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(60, 60, 62));
    style.visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(70, 70, 72));
    style.visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(100, 100, 102));

    style.visuals.widgets.noninteractive.fg_stroke.color = egui::Color32::from_rgb(180, 180, 180);
    style.visuals.widgets.inactive.fg_stroke.color = egui::Color32::from_rgb(210, 210, 210);
    style.visuals.widgets.hovered.fg_stroke.color = egui::Color32::WHITE;
    style.visuals.widgets.active.fg_stroke.color = egui::Color32::WHITE;

    style.visuals.selection.bg_fill = egui::Color32::from_rgb(60, 100, 200);
    style.visuals.selection.stroke.color = egui::Color32::WHITE;

    // Keskin, profesyonel kenarlar (Rounding=4.0)
    style.visuals.window_rounding = egui::Rounding::same(4.0);
    style.visuals.widgets.noninteractive.rounding = egui::Rounding::same(3.0);
    style.visuals.widgets.inactive.rounding = egui::Rounding::same(3.0);
    style.visuals.widgets.hovered.rounding = egui::Rounding::same(3.0);
    style.visuals.widgets.active.rounding = egui::Rounding::same(3.0);

    // Boşluklar
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.window_margin = egui::style::Margin::same(12.0);

    ctx.set_style(style);
}
