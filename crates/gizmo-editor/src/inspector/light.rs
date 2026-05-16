
use crate::editor_state::EditorState;
use egui;
use gizmo_core::World;
use gizmo_math::Vec3;
use gizmo_renderer::components::{PointLight, DirectionalLight};


pub fn draw_point_light_section(
    ui: &mut egui::Ui,
    world: &World,
    entity_id: gizmo_core::entity::Entity,
    _state: &mut EditorState,
) {
    let mut lights = world.borrow_mut::<PointLight>();
    {
        if let Some(light) = lights.get_mut(entity_id.id()) {
            egui::CollapsingHeader::new("💡 PointLight")
                .default_open(false)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Renk:");
                        let mut color = [light.color.x, light.color.y, light.color.z];
                        if ui.color_edit_button_rgb(&mut color).changed() {
                            light.color = Vec3::new(color[0], color[1], color[2]);
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Yoğunluk:");
                        ui.add(
                            egui::DragValue::new(&mut light.intensity)
                                .speed(0.1)
                                .range(0.0..=100.0),
                        );
                    });
                });
            ui.separator();
        }
    }
}


pub fn draw_directional_light_section(
    ui: &mut egui::Ui,
    world: &World,
    entity_id: gizmo_core::entity::Entity,
    _state: &mut EditorState,
) {
    let mut lights = world.borrow_mut::<DirectionalLight>();
    if let Some(light) = lights.get_mut(entity_id.id()) {
        egui::CollapsingHeader::new("☀️ Directional Light (Güneş)")
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Güneş Rengi:");
                    let mut color = [light.color.x, light.color.y, light.color.z];
                    if ui.color_edit_button_rgb(&mut color).changed() {
                        light.color = Vec3::new(color[0], color[1], color[2]);
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Işık Şiddeti (Intensity):");
                    ui.add(
                        egui::Slider::new(&mut light.intensity, 0.0..=500.0)
                            .text("Lümen")
                    );
                });
                ui.label(egui::RichText::new("Güneşin açısını (gölge yönünü) değiştirmek için üst kısımdaki 'Transform' altından Rotasyon X, Y, Z değerlerini döndürün.").weak().small());
            });
        ui.separator();
    }
}


