
use crate::editor_state::EditorState;
use egui;
use gizmo_core::World;
use gizmo_renderer::components::Camera;


pub fn draw_camera_section(
    ui: &mut egui::Ui,
    world: &World,
    entity_id: gizmo_core::entity::Entity,
    _state: &mut EditorState,
) {
    let mut cameras = world.borrow_mut::<Camera>();
    {
        if let Some(cam) = cameras.get_mut(entity_id.id()) {
            egui::CollapsingHeader::new("📷 Camera")
                .default_open(false)
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
                });
            ui.separator();
        }
    }
}


