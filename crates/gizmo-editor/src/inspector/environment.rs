
use crate::editor_state::EditorState;
use egui;

/// One labelled slider, sized to the panel it is in.
///
/// Every row here used to be `ui.horizontal(|ui| { ui.label(..); ui.add(Slider::new(..).text(..)) })`.
/// A horizontal layout neither wraps nor shrinks, so the row's width was the sum of a long Turkish
/// label, the theme's fixed 96 px slider, a value box and a second suffix label — a flat 422.7 px
/// that did not move when the panel got narrower. The Inspector is 25% of the window
/// (`create_default_dock_state`), i.e. 400 px at the default 1600 px studio window, so the panel
/// clipped its own contents out of the box, and worse: the over-wide rows widened the enclosing
/// `ScrollArea`, so the closing hint wrapped past the panel edge and lost the end of both lines.
///
/// The label goes above the slider instead of beside it — a vertical layout wraps, and the slider
/// is then given the whole remaining width. The suffix text is gone with it: it was a second name
/// for a control that already had one ("Bloom Yoğunluğu:" … "Glow").
fn slider_row(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
) {
    ui.label(label);
    // The value box and the frame's own padding come out of the panel; the rest is the track.
    // Floored so a panel dragged very narrow degrades into a small slider rather than a negative
    // width, which egui would round up to something wider than the panel — the defect again.
    const VALUE_BOX: f32 = 68.0;
    const MIN_TRACK: f32 = 40.0;
    ui.spacing_mut().slider_width = (ui.available_width() - VALUE_BOX).max(MIN_TRACK);
    ui.add(egui::Slider::new(value, range));
    ui.add_space(6.0);
}

pub fn draw_environment_settings(ui: &mut egui::Ui, state: &mut EditorState) {
    ui.heading("🌍 World & Environment Settings");
    ui.label(
        egui::RichText::new(
            "Sahnedeki genel aydınlatma ve post-processing (kamera efektleri) ayarlarını buradan yapabilirsiniz.",
        )
        .weak()
        .small(),
    );
    ui.separator();

    egui::ScrollArea::vertical().show(ui, |ui| {
        egui::CollapsingHeader::new(crate::theme::section_title("Post-Processing / Bloom"))
            .default_open(true)
            .show(ui, |ui| {
                slider_row(ui, "Bloom Yoğunluğu", &mut state.post_process.bloom_intensity, 0.0..=5.0);
                slider_row(ui, "Bloom Eşiği (Threshold)", &mut state.post_process.bloom_threshold, 0.0..=2.0);
                slider_row(ui, "Film Greni (Grain)", &mut state.post_process.film_grain, 0.0..=1.0);
            });

        egui::CollapsingHeader::new(crate::theme::section_title("Camera Lens Effects"))
            .default_open(true)
            .show(ui, |ui| {
                slider_row(ui, "Kamera Pozlaması (Exposure)", &mut state.post_process.exposure, 0.1..=5.0);
                slider_row(ui, "Köşe Karartması (Vignette)", &mut state.post_process.vignette, 0.0..=1.0);
                slider_row(ui, "Kromatik Sapma (Aberration)", &mut state.post_process.chromatic_aberration, 0.0..=0.05);
            });

        egui::CollapsingHeader::new(crate::theme::section_title("Depth of Field (Odak)"))
            .default_open(true)
            .show(ui, |ui| {
                slider_row(ui, "Odak Uzaklığı (metre)", &mut state.post_process.dof_focus_dist, 0.1..=100.0);
                slider_row(ui, "Odak Aralığı (Net Alan)", &mut state.post_process.dof_focus_range, 0.1..=50.0);
                slider_row(ui, "Arka Plan Bulanıklığı", &mut state.post_process.dof_blur_size, 0.0..=10.0);
            });

        ui.add_space(20.0);
        ui.label(
            egui::RichText::new(
                "💡 İpucu: Güneşin (Directional Light) yönünü ve rengini ayarlamak için Hierarchy panelinden 'Directional Light' objesini seçin.",
            )
            .color(egui::Color32::from_rgb(180, 180, 180)),
        );
    });
}
