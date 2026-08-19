
use crate::editor_state::EditorState;
use egui;
use gizmo_core::World;
use gizmo_math::Vec3;
use gizmo_renderer::components::{PointLight, DirectionalLight, SpotLight};


/// The point-light rows: colour, intensity and radius.
pub fn draw_point_light_section(
    ui: &mut egui::Ui,
    world: &World,
    entity_id: gizmo_core::entity::Entity,
    _state: &mut EditorState,
) {
    // SAFETY: editor UI runs single-threaded in the egui draw; no concurrent World access.
    let mut lights = unsafe { world.borrow_mut_unchecked::<PointLight>() };
    {
        if let Some(mut light) = lights.get_mut(entity_id.id()) {
            egui::CollapsingHeader::new(crate::theme::section_title("PointLight"))
                .default_open(true)
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


/// The directional-light rows: colour, intensity and the direction it points.
pub fn draw_directional_light_section(
    ui: &mut egui::Ui,
    world: &World,
    entity_id: gizmo_core::entity::Entity,
    _state: &mut EditorState,
) {
    // SAFETY: editor UI runs single-threaded in the egui draw; no concurrent World access.
    let mut lights = unsafe { world.borrow_mut_unchecked::<DirectionalLight>() };
    if let Some(mut light) = lights.get_mut(entity_id.id()) {
        egui::CollapsingHeader::new(crate::theme::section_title("Directional Light (Güneş)"))
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


/// The spot-light rows: colour, intensity, reach, and the two cone angles.
///
/// The engine has drawn spot lights and the scene file has carried them since the component
/// existed, and this section did not exist — so a spot light arriving from Rust or from a
/// hand-written scene lit the viewport and showed **not one editable row** when selected. It was
/// not reachable through the generic JSON fallback either: that one looks the component up in the
/// `ComponentRegistry`, and `SpotLight` was not in it.
///
/// Angles are edited in DEGREES and stored in radians, because a cone is something an author
/// thinks about in degrees and every other engine's spot light asks for them that way. The inner
/// angle is clamped to the outer one on every edit, which is the same rule `SpotLight::new`
/// applies and for the same reason: an inner cone wider than its outer cone inverts the falloff
/// and lights the outside of the cone.
pub fn draw_spot_light_section(
    ui: &mut egui::Ui,
    world: &World,
    entity_id: gizmo_core::entity::Entity,
    _state: &mut EditorState,
) {
    // SAFETY: editor UI runs single-threaded in the egui draw; no concurrent World access.
    let mut lights = unsafe { world.borrow_mut_unchecked::<SpotLight>() };
    if let Some(mut light) = lights.get_mut(entity_id.id()) {
        egui::CollapsingHeader::new(crate::theme::section_title("Spot Light"))
            .default_open(true)
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
                ui.horizontal(|ui| {
                    ui.label("Menzil:");
                    ui.add(
                        egui::DragValue::new(&mut light.radius)
                            .speed(0.1)
                            .range(0.001..=500.0)
                            .suffix(" m"),
                    );
                });

                let mut inner_deg = light.inner_angle.to_degrees();
                let mut outer_deg = light.outer_angle.to_degrees();
                let mut changed = false;
                ui.horizontal(|ui| {
                    ui.label("İç koni:");
                    changed |= ui
                        .add(egui::Slider::new(&mut inner_deg, 0.0..=89.0).suffix("°"))
                        .changed();
                });
                ui.horizontal(|ui| {
                    ui.label("Dış koni:");
                    changed |= ui
                        .add(egui::Slider::new(&mut outer_deg, 0.0..=89.0).suffix("°"))
                        .changed();
                });
                if changed {
                    // Whichever one moved, the invariant is the same: inner ≤ outer.
                    light.outer_angle = outer_deg.to_radians();
                    light.inner_angle = inner_deg.min(outer_deg).to_radians();
                }

                ui.label(egui::RichText::new("Koninin baktığı yön varlığın 'Transform' rotasyonundan gelir.").weak().small());
            });
        ui.separator();
    }
}


