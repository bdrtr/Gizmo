
use crate::editor_state::EditorState;
use egui;
use gizmo_core::World;
use gizmo_math::Vec4;
use gizmo_renderer::components::Material;


pub fn draw_material_section(
    ui: &mut egui::Ui,
    world: &World,
    entity_id: gizmo_core::entity::Entity,
    _state: &mut EditorState,
) {
    let mut materials = world.borrow_mut::<Material>();
    {
        if let Some(mat) = materials.get_mut(entity_id.id()) {
            egui::CollapsingHeader::new("🎨 Material Properties")
                .default_open(true)
                .show(ui, |ui| {
                    ui.group(|ui| {
                        // --- Material Type Dropdown ---
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new("Shader Mode:").strong());
                            
                            let mut current_type = mat.material_type;
                            egui::ComboBox::from_id_source(format!("mat_type_{}", entity_id.id()))
                                .selected_text(format!("{:?}", current_type))
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut current_type, gizmo_renderer::components::MaterialType::Pbr, "PBR (Physically Based)");
                                    ui.selectable_value(&mut current_type, gizmo_renderer::components::MaterialType::Unlit, "Unlit (No Lighting)");
                                    ui.selectable_value(&mut current_type, gizmo_renderer::components::MaterialType::Water, "Water (Fluid Surface)");
                                    ui.selectable_value(&mut current_type, gizmo_renderer::components::MaterialType::Skybox, "Skybox");
                                    ui.selectable_value(&mut current_type, gizmo_renderer::components::MaterialType::Grid, "Grid");
                                });
                            mat.material_type = current_type;
                        });
                        
                        ui.separator();
                        
                        // --- Albedo Color ---
                        ui.horizontal(|ui| {
                            ui.label("Base Color (Albedo):");
                            let mut color = [mat.albedo.x, mat.albedo.y, mat.albedo.z, mat.albedo.w];
                            if ui.color_edit_button_rgba_premultiplied(&mut color).changed() {
                                mat.albedo = Vec4::new(color[0], color[1], color[2], color[3]);
                                // Oto saydamlık kontrolü
                                if color[3] < 1.0 {
                                    mat.is_transparent = true;
                                }
                            }
                        });

                        ui.add_space(5.0);

                        // --- PBR Sliders ---
                        if mat.material_type == gizmo_renderer::components::MaterialType::Pbr {
                            ui.horizontal(|ui| {
                                ui.label("Metallic:  ");
                                ui.add(
                                    egui::Slider::new(&mut mat.metallic, 0.0..=1.0)
                                        .text("Metal")
                                        .show_value(true)
                                );
                            });
                            
                            ui.horizontal(|ui| {
                                ui.label("Roughness:");
                                ui.add(
                                    egui::Slider::new(&mut mat.roughness, 0.0..=1.0)
                                        .text("Pürüz")
                                        .show_value(true)
                                );
                            });
                        }
                        
                        ui.separator();

                        // --- Render Flags ---
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut mat.is_transparent, "Transparent").on_hover_text("Alpha blending etkinleştirir.");
                            ui.checkbox(&mut mat.is_double_sided, "Double Sided").on_hover_text("Backface culling'i kapatır.");
                        });

                        // --- Texture Source ---
                        let (rect, response) = ui.allocate_exact_size(
                            egui::vec2(ui.available_width(), 30.0),
                            egui::Sense::hover(),
                        );
                        let mut stroke_color = egui::Color32::from_rgb(50, 50, 50);

                        // Sürükle Bırak (Drag & Drop) Doku (Texture) atama
                        if let Some(dragged_path) = _state.dragged_asset.clone() {
                            if dragged_path.ends_with(".png") || dragged_path.ends_with(".jpg") || dragged_path.ends_with(".jpeg") {
                                if response.hovered() {
                                    stroke_color = egui::Color32::YELLOW;
                                    if ui.input(|i| i.pointer.any_released()) {
                                        _state.log_info(&format!("Doku atandı: {}", dragged_path));
                                        mat.texture_source = Some(dragged_path);
                                        mat.material_type = gizmo_renderer::components::MaterialType::Pbr;
                                        // TODO: We need to load this texture actually via a system.
                                        _state.dragged_asset = None;
                                    }
                                }
                            }
                        }

                        ui.painter().rect_stroke(rect, 2.0, egui::Stroke::new(1.0, stroke_color));
                        let text_pos = rect.left_center() + egui::vec2(8.0, 0.0);
                        
                        if let Some(src) = &mat.texture_source {
                            ui.painter().text(
                                text_pos,
                                egui::Align2::LEFT_CENTER,
                                format!("Texture: {}", src),
                                egui::FontId::proportional(12.0),
                                egui::Color32::from_rgb(100, 200, 255),
                            );
                        } else {
                            ui.painter().text(
                                text_pos,
                                egui::Align2::LEFT_CENTER,
                                "Texture: (Doku Sürükle & Bırak)",
                                egui::FontId::proportional(12.0),
                                egui::Color32::GRAY,
                            );
                        }
                    });
                });
            ui.add_space(4.0);
        }
    }
}


