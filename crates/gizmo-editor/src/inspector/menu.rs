
use crate::editor_state::EditorState;
use egui;
use gizmo_core::World;


/// The `➕ Add component` menu: what can be added to the selection, and the request each entry
/// raises.
pub fn draw_add_component_menu(
    ui: &mut egui::Ui,
    world: &World,
    entity_id: gizmo_core::entity::Entity,
    state: &mut EditorState,
) {
    ui.group(|ui| {
        ui.label("Eklenebilecek Bileşenler");
        ui.separator();

        if let Some(registry) = world.get_resource::<gizmo_core::ComponentRegistry>() {
            let names = registry.all_names();
            for comp_name in names {
                // TODO: Entity üzerinde component olup olmadığını gizmo_core registry üzerinden checkle.
                if ui.button(format!("🔹 {}", comp_name)).clicked() {
                    state.add_component_request = Some((entity_id, comp_name.to_string()));
                    state.add_component_open = false;
                }
            }
        } else {
            ui.label(
                egui::RichText::new("ComponentRegistry bulunamadi!").color(egui::Color32::RED),
            );
        }
    });
}


