
use crate::editor_state::EditorState;
use egui;
use gizmo_core::World;
use gizmo_math::Vec4;
use gizmo_renderer::components::Material;


/// The material rows: albedo, metallic, roughness, emissive and the texture slots.
pub fn draw_material_section(
    ui: &mut egui::Ui,
    world: &World,
    entity_id: gizmo_core::entity::Entity,
    _state: &mut EditorState,
) {
    // SAFETY: editor UI runs single-threaded in the egui draw; no concurrent World access.
    let mut materials = unsafe { world.query_unchecked::<&mut Material>() };
    {
        if let Some(mut mat) = materials.get_mut(entity_id) {
            egui::CollapsingHeader::new(crate::theme::section_title("Material Properties"))
                .default_open(true)
                .show(ui, |ui| {
                    ui.group(|ui| {
                        // --- Material Type Dropdown ---
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new("Shader Mode:").strong());
                            
                            let mut current_type = mat.material_type;
                            egui::ComboBox::from_id_salt(format!("mat_type_{}", entity_id.id()))
                                .selected_text(format!("{:?}", current_type))
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut current_type, gizmo_renderer::components::MaterialType::Pbr, "PBR (Physically Based)");
                                    ui.selectable_value(&mut current_type, gizmo_renderer::components::MaterialType::Unlit, "Unlit (No Lighting)");
                                    ui.selectable_value(&mut current_type, gizmo_renderer::components::MaterialType::Water, "Water (Fluid Surface)");
                                    ui.selectable_value(&mut current_type, gizmo_renderer::components::MaterialType::Skybox, "Skybox (Procedural Sky)");
                                    // Boyalı arka plan: mesh'in kendi dokusu/vertex rengi, kameraya kilitli, derinlik yazmaz.
                                    ui.selectable_value(&mut current_type, gizmo_renderer::components::MaterialType::Backdrop, "Backdrop (Painted Sky/Panorama)");
                                    // Aynısı ama kameraya kilitsiz: seviyeye yerleştirilmiş uzak geometri.
                                    ui.selectable_value(&mut current_type, gizmo_renderer::components::MaterialType::BackdropPlaced, "Backdrop Placed (Stays In World)");
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

                        // --- Blend Mode --- Alpha dışındaki her kip malzemeyi saydam yapar
                        // (`Material::with_blend_mode` ile aynı kural).
                        ui.horizontal(|ui| {
                            use gizmo_renderer::components::BlendMode;
                            ui.label("Blend:");
                            let mut mode = mat.blend_mode;
                            egui::ComboBox::from_id_salt(format!("mat_blend_{}", entity_id.id()))
                                .selected_text(format!("{mode:?}"))
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut mode, BlendMode::Alpha, "Alpha");
                                    ui.selectable_value(&mut mode, BlendMode::Additive, "Additive (ateş, parıltı)");
                                    ui.selectable_value(&mut mode, BlendMode::Multiply, "Multiply (renkli cam, leke)");
                                    ui.selectable_value(&mut mode, BlendMode::Premultiplied, "Premultiplied");
                                });
                            if mode != mat.blend_mode {
                                mat.blend_mode = mode;
                                if mode != BlendMode::Alpha {
                                    mat.is_transparent = true;
                                }
                            }
                        });

                        // --- Textures: the base colour and the maps beside it ---
                        // A change to any of them is all it takes: `material_sync` builds the bind
                        // group again from the new sources the next frame.
                        ui.label(egui::RichText::new("Textures").strong());
                        if let Some(path) = texture_slot(ui, _state, "Base Color", mat.texture_source.as_deref()) {
                            mat.texture_source = path;
                            mat.material_type = gizmo_renderer::components::MaterialType::Pbr;
                        }
                        let mut maps = mat.maps.clone();
                        if let Some(path) = texture_slot(ui, _state, "Normal", maps.normal.as_deref()) {
                            maps.normal = path;
                        }
                        if maps.normal.is_some() {
                            ui.add(egui::Slider::new(&mut maps.normal_scale, 0.0..=2.0).text("Normal strength"));
                        }
                        if let Some(path) = texture_slot(ui, _state, "Metallic-Roughness", maps.metallic_roughness.as_deref()) {
                            maps.metallic_roughness = path;
                        }
                        if let Some(path) = texture_slot(ui, _state, "Emissive", maps.emissive.as_deref()) {
                            maps.emissive = path;
                        }
                        if maps.emissive.is_some() {
                            ui.horizontal(|ui| {
                                ui.label("Emissive factor:");
                                let mut factor = maps.emissive_factor.to_array();
                                if ui.color_edit_button_rgb(&mut factor).changed() {
                                    maps.emissive_factor = gizmo_math::Vec3::from_array(factor);
                                }
                            });
                        }
                        if let Some(path) = texture_slot(ui, _state, "Occlusion", maps.occlusion.as_deref()) {
                            maps.occlusion = path;
                        }
                        if maps.occlusion.is_some() {
                            ui.add(egui::Slider::new(&mut maps.occlusion_strength, 0.0..=1.0).text("Occlusion strength"));
                        }
                        // Baked light, read at the mesh's second UV set (`MaterialMaps::lightmap`).
                        if let Some(path) = texture_slot(ui, _state, "Lightmap", maps.lightmap.as_deref()) {
                            maps.lightmap = path;
                        }
                        if maps.lightmap.is_some() {
                            ui.add(egui::Slider::new(&mut maps.lightmap_intensity, 0.0..=8.0).text("Lightmap intensity"));
                        }
                        if maps != mat.maps {
                            mat.maps = maps;
                        }
                    });
                });
            ui.add_space(4.0);
        }
    }
}



/// One texture slot: its name and path, a drop target for an image dragged from the asset browser,
/// and a button that clears it. `Some(new)` when the slot changed this frame.
fn texture_slot(ui: &mut egui::Ui, state: &mut EditorState, label: &str, current: Option<&str>) -> Option<Option<String>> {
    let mut result = None;
    ui.horizontal(|ui| {
        let width = (ui.available_width() - 28.0).max(60.0);
        let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 24.0), egui::Sense::hover());
        let mut stroke = egui::Color32::from_rgb(50, 50, 50);
        if let Some(dragged) = state.dragged_asset.clone() {
            let image = [".png", ".jpg", ".jpeg"].iter().any(|ext| dragged.to_lowercase().ends_with(ext));
            if image && response.hovered() {
                stroke = egui::Color32::YELLOW;
                if ui.input(|i| i.pointer.any_released()) {
                    state.log_info(&format!("{label} dokusu atandı: {dragged}"));
                    state.dragged_asset = None;
                    result = Some(Some(dragged));
                }
            }
        }
        ui.painter().rect_stroke(rect, 2, egui::Stroke::new(1.0_f32, stroke), egui::StrokeKind::Inside);
        let (text, color) = match current {
            Some(path) => (format!("{label}: {path}"), egui::Color32::from_rgb(100, 200, 255)),
            None => (format!("{label}: (Doku Sürükle & Bırak)"), egui::Color32::GRAY),
        };
        ui.painter().with_clip_rect(rect).text(
            rect.left_center() + egui::vec2(8.0, 0.0),
            egui::Align2::LEFT_CENTER,
            text,
            egui::FontId::proportional(12.0),
            color,
        );
        if ui.add_enabled(current.is_some(), egui::Button::new("×").small()).on_hover_text("Clear").clicked() {
            result = Some(None);
        }
    });
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_core::world::World;
    use std::sync::Arc;

    /// A `Material` over an empty bind group, made on wgpu's noop backend: no GPU needed.
    fn material() -> Material {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::NOOP,
            backend_options: wgpu::BackendOptions {
                noop: wgpu::NoopBackendOptions { enable: true, ..Default::default() },
                ..Default::default()
            },
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).expect("the noop adapter");
        let (device, _queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("a noop device");
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: None, entries: &[] });
        Material::new(Arc::new(device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &layout, entries: &[] })))
    }

    fn texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<String> {
        fn walk(shape: &egui::Shape, out: &mut Vec<String>) {
            match shape {
                egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
        }
        let mut out = Vec::new();
        shapes.iter().for_each(|c| walk(&c.shape, &mut out));
        out
    }

    /// One frame of the material section, with the pointer at `pointer` (released there, if
    /// `release`), and the texts it painted.
    fn frame(ctx: &egui::Context, world: &World, entity: gizmo_core::entity::Entity, state: &mut EditorState, pointer: Option<egui::Pos2>, release: bool) -> (Vec<String>, Vec<(String, egui::Rect)>) {
        let mut input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(420.0, 2000.0))),
            ..Default::default()
        };
        if let Some(p) = pointer {
            input.events.push(egui::Event::PointerMoved(p));
            if release {
                input.events.push(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
            }
        }
        let mut slots = Vec::new();
        let output = ctx.run_ui(input, |ui| {
            draw_material_section(ui, world, entity, state);
        });
        fn walk(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
            match shape {
                egui::Shape::Text(t) => out.push((t.galley.text().to_string(), shape.visual_bounding_rect())),
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
        }
        output.shapes.iter().for_each(|c| walk(&c.shape, &mut slots));
        let t = texts(&output.shapes);
        output.drop_without_applying_deltas();
        (t, slots)
    }

    /// **A texture dropped on the Lightmap slot is the material's lightmap, and its intensity is
    /// offered then.** Every map slot is drawn, the lightmap's among them; before the drop there is
    /// no intensity slider, after it the material names the dropped file as its lightmap and the
    /// slider is there.
    #[test]
    fn a_texture_dropped_on_the_lightmap_slot_is_the_materials_lightmap() {
        let mut world = World::new();
        let entity = world.spawn();
        world.add_component(entity, material());
        let mut state = EditorState::default();
        let ctx = egui::Context::default();
        // A first frame to open the section (egui lays a collapsing header out over two frames).
        frame(&ctx, &world, entity, &mut state, None, false);
        let (before, slots) = frame(&ctx, &world, entity, &mut state, None, false);
        for slot in ["Normal", "Metallic-Roughness", "Emissive", "Occlusion", "Lightmap"] {
            assert!(before.iter().any(|t| t.starts_with(&format!("{slot}:"))), "no {slot} slot in the material section: {before:?}");
        }
        assert!(!before.iter().any(|t| t.contains("Lightmap intensity")), "an intensity for a lightmap the material has not got");

        let at = slots.iter().find(|(t, _)| t.starts_with("Lightmap:")).map(|(_, r)| r.center()).expect("the lightmap slot");
        state.dragged_asset = Some("demo/assets/baked/room_lightmap.png".into());
        frame(&ctx, &world, entity, &mut state, Some(at), false);
        frame(&ctx, &world, entity, &mut state, Some(at), true);
        let maps = world.query::<&Material>().get(entity).unwrap().maps.clone();
        assert_eq!(maps.lightmap.as_deref(), Some("demo/assets/baked/room_lightmap.png"), "the drop did not set the lightmap");
        let (after, _) = frame(&ctx, &world, entity, &mut state, None, false);
        assert!(after.iter().any(|t| t.contains("Lightmap intensity")), "no intensity for the lightmap the material has now: {after:?}");
    }
}
