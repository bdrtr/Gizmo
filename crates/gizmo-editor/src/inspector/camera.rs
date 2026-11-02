
use crate::editor_state::EditorState;
use egui;
use gizmo_core::World;
use gizmo_renderer::components::Camera;


/// The camera rows: field of view, clip planes, exposure and whether this is the primary camera.
pub fn draw_camera_section(
    ui: &mut egui::Ui,
    world: &World,
    entity_id: gizmo_core::entity::Entity,
    _state: &mut EditorState,
) {
    // SAFETY: editor UI runs single-threaded in the egui draw; no concurrent World access.
    let mut cameras = unsafe { world.borrow_mut_unchecked::<Camera>() };
    {
        if let Some(mut cam) = cameras.get_mut(entity_id.id()) {
            egui::CollapsingHeader::new(crate::theme::section_title("Camera"))
                .default_open(true)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("FOV:");
                        let mut fov_deg = cam.fov.to_degrees();
                        if ui
                            .add(
                                egui::DragValue::new(&mut fov_deg)
                                    .speed(1.0)
                                    .range(10.0..=120.0)
                                    .suffix("°"),
                            )
                            .changed()
                        {
                            cam.fov = fov_deg.to_radians();
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Near:");
                        ui.add(
                            egui::DragValue::new(&mut cam.near)
                                .speed(0.01)
                                .range(0.001..=10.0),
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.label("Far:");
                        ui.add(
                            egui::DragValue::new(&mut cam.far)
                                .speed(10.0)
                                .range(10.0..=50000.0),
                        );
                    });
                    ui.checkbox(&mut cam.primary, "Ana Kamera");
                    // `Camera::clear_color`: off keeps the render path's own background.
                    ui.horizontal(|ui| {
                        let mut on = cam.clear_color.is_some();
                        if ui.checkbox(&mut on, "Arka plan rengi").changed() {
                            cam.clear_color = on.then_some(gizmo_math::Vec3::new(0.02, 0.02, 0.03));
                        }
                        if let Some(c) = cam.clear_color {
                            let mut rgb = [c.x, c.y, c.z];
                            if ui.color_edit_button_rgb(&mut rgb).changed() {
                                cam.clear_color = Some(gizmo_math::Vec3::from_array(rgb));
                            }
                        }
                    });
                });
            ui.separator();
        }
    }
}


