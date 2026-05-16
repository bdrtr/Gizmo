
use crate::editor_state::EditorState;
use egui;


pub fn draw_environment_settings(ui: &mut egui::Ui, state: &mut EditorState) {
    ui.heading("🌍 World & Environment Settings");
    ui.label(egui::RichText::new("Sahnedeki genel aydınlatma ve post-processing (kamera efektleri) ayarlarını buradan yapabilirsiniz.").weak().small());
    ui.separator();

    egui::ScrollArea::vertical().show(ui, |ui| {
        egui::CollapsingHeader::new("✨ Post-Processing / Bloom")
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Bloom Yoğunluğu:");
                    ui.add(egui::Slider::new(&mut state.post_process.bloom_intensity, 0.0..=5.0).text("Glow"));
                });
                ui.horizontal(|ui| {
                    ui.label("Bloom Eşiği (Threshold):");
                    ui.add(egui::Slider::new(&mut state.post_process.bloom_threshold, 0.0..=2.0).text("Eşik"));
                });
                ui.horizontal(|ui| {
                    ui.label("Film Greni (Grain):");
                    ui.add(egui::Slider::new(&mut state.post_process.film_grain, 0.0..=1.0).text("Kumlanma"));
                });
            });

        egui::CollapsingHeader::new("📷 Camera Lens Effects")
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Kamera Pozlaması (Exposure):");
                    ui.add(egui::Slider::new(&mut state.post_process.exposure, 0.1..=5.0).text("EV"));
                });
                ui.horizontal(|ui| {
                    ui.label("Köşe Karartması (Vignette):");
                    ui.add(egui::Slider::new(&mut state.post_process.vignette, 0.0..=1.0).text("Vignette"));
                });
                ui.horizontal(|ui| {
                    ui.label("Kromatik Sapma (Aberration):");
                    ui.add(egui::Slider::new(&mut state.post_process.chromatic_aberration, 0.0..=0.05).text("Aberration"));
                });
            });
            
        egui::CollapsingHeader::new("🔍 Depth of Field (Odak)")
            .default_open(false)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Odak Uzaklığı:");
                    ui.add(egui::Slider::new(&mut state.post_process.dof_focus_dist, 0.1..=100.0).text("Metre"));
                });
                ui.horizontal(|ui| {
                    ui.label("Odak Aralığı (Net Alan):");
                    ui.add(egui::Slider::new(&mut state.post_process.dof_focus_range, 0.1..=50.0).text("Menzil"));
                });
                ui.horizontal(|ui| {
                    ui.label("Arka Plan Bulanıklığı:");
                    ui.add(egui::Slider::new(&mut state.post_process.dof_blur_size, 0.0..=10.0).text("Blur"));
                });
            });
            
        ui.add_space(20.0);
        ui.label(egui::RichText::new("💡 İpucu: Güneşin (Directional Light) yönünü ve rengini ayarlamak için Hierarchy panelinden 'Directional Light' objesini seçin.").color(egui::Color32::from_rgb(180, 180, 180)));
    });
}


