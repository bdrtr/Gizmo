//! Scene Hierarchy Panel — Sol panel'de entity ağacını gösterir

use egui;
use gizmo_core::{World, EntityName, component::{Parent, Children}};
use crate::editor_state::EditorState;

/// Scene Hierarchy sekmesini çizer
pub fn ui_hierarchy(ui: &mut egui::Ui, world: &World, state: &mut EditorState) {
    ui.heading("🌍 Sahne Hiyerarşisi");
    ui.separator();

    // Arama kutusu
            ui.horizontal(|ui| {
                ui.label("🔍");
                ui.text_edit_singleline(&mut state.hierarchy_filter);
            });
            ui.separator();

            // Hierarchy ScrollArea'nın tamamını "Asset Drop" alanı olarak kabul edebilmek için arka planı değerlendireceğiz
            let bg_response = ui.interact(ui.available_rect_before_wrap(), ui.id().with("hierarchy_bg"), egui::Sense::click_and_drag());
            
            // Asset Drop Yakalama
            if bg_response.hovered() {
                if let Some(dragged_path) = ui.memory(|m| m.data.get_temp::<String>(egui::Id::new("dragged_asset_path"))) {
                    ui.painter().rect_stroke(bg_response.rect, 2.0, egui::Stroke::new(2.0, egui::Color32::from_rgb(100, 255, 100)));
                    if ui.input(|i| i.pointer.any_released()) {
                        state.spawn_asset_request = Some(dragged_path);
                        ui.memory_mut(|m| m.data.remove::<String>(egui::Id::new("dragged_asset_path")));
                    }
                }
            }

            // Sağ tık menüsü — boşluğa tıklayınca
            bg_response.context_menu(|ui| {
                    if ui.button("➕ Boş Entity Ekle").clicked() {
                        state.spawn_request = Some("Empty".to_string());
                        ui.close_menu();
                    }
                    if ui.button("📦 Küp Ekle").clicked() {
                        state.spawn_request = Some("Cube".to_string());
                        ui.close_menu();
                    }
                    if ui.button("🔴 Küre Ekle").clicked() {
                        state.spawn_request = Some("Sphere".to_string());
                        ui.close_menu();
                    }
                    
                    // Unparent yapabilmek için (Kök yapmak)
                    if let Some(dragged) = ui.memory(|mem| mem.data.get_temp::<u32>(egui::Id::new("dragged_ent"))) {
                        if ui.input(|i| i.pointer.any_released()) {
                            state.unparent_request = Some(dragged);
                            ui.memory_mut(|mem| mem.data.remove::<u32>(egui::Id::new("dragged_ent")));
                        }
                    }
                });

            // Entity listesini oluştur
            egui::ScrollArea::vertical().show(ui, |ui| {
                let names = world.borrow::<EntityName>();
                let parents = world.borrow::<Parent>();
                let children_comp = world.borrow::<Children>();

                // ROOT entity'leri bul (parent'ı olmayanlar)
                let mut root_entities = Vec::new();
                for entity in world.iter_alive_entities() {
                    let eid = entity.id();
                    let has_parent = parents.as_ref()
                        .map(|p| p.contains(eid))
                        .unwrap_or(false);
                    
                    if !has_parent {
                        root_entities.push(eid);
                    }
                }

                // Root entity'leri çiz
                for &eid in &root_entities {
                    // Editor-only (Hayali) objeleri hiyerarşide listeleme
                    let is_editor_only = names.as_ref()
                        .and_then(|n| n.get(eid))
                        .map(|e| e.0 == "Editor Guidelines" || e.0 == "Highlight Box")
                        .unwrap_or(false);
                        
                    if is_editor_only { continue; }

                    draw_entity_node(
                        ui,
                        world,
                        eid,
                        state,
                        &names,
                        &children_comp,
                        &state.hierarchy_filter.clone(),
                    );
                }
            });
            
            ui.separator();
            ui.label(format!("Toplam: {} entity", world.entity_count()));
}

/// Tek bir entity node'unu recursive olarak çizer
fn draw_entity_node(
    ui: &mut egui::Ui,
    world: &World,
    entity_id: u32,
    state: &mut EditorState,
    names: &Option<std::cell::Ref<'_, gizmo_core::SparseSet<EntityName>>>,
    children_comp: &Option<std::cell::Ref<'_, gizmo_core::SparseSet<Children>>>,
    filter: &str,
) {
    let entity_name = names.as_ref()
        .and_then(|n| n.get(entity_id))
        .map(|n| n.0.clone())
        .unwrap_or_else(|| format!("Entity_{}", entity_id));

    // Filtre uygulaması
    if !filter.is_empty() && !entity_name.to_lowercase().contains(&filter.to_lowercase()) {
        // Bu entity filtrede yoksa ama child'ları olabilir — onları kontrol et
        if let Some(children) = children_comp.as_ref().and_then(|c| c.get(entity_id)) {
            for &child_id in &children.0 {
                draw_entity_node(ui, world, child_id, state, names, children_comp, filter);
            }
        }
        return;
    }

    let is_selected = state.is_selected(entity_id);
    let has_children = children_comp.as_ref()
        .and_then(|c| c.get(entity_id))
        .map(|c| !c.0.is_empty())
        .unwrap_or(false);

    if has_children {
        // Katlanabilir ağaç düğümü
        let id = ui.make_persistent_id(format!("entity_{}", entity_id));
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
            .show_header(ui, |ui| {
                let is_hidden = world.borrow::<gizmo_core::component::IsHidden>().map(|hc| hc.contains(entity_id)).unwrap_or(false);
                let label_text = if is_hidden { format!("📦 {} (Gizli)", entity_name) } else { format!("📦 {}", entity_name) };
                let label = if is_selected {
                    egui::RichText::new(label_text).strong().color(egui::Color32::from_rgb(100, 200, 255))
                } else {
                    egui::RichText::new(label_text)
                };
                
                let response = ui.selectable_label(is_selected, label);
                if response.clicked() {
                    if ui.input(|i| i.modifiers.command) {
                        state.toggle_selection(entity_id);
                    } else {
                        state.select_exclusive(entity_id);
                    }
                }
                
                // --- Sürükle Bırak (Drag & Drop) ---
                let drag_id = egui::Id::new("drag_ent").with(entity_id);
                let drag_response = ui.interact(response.rect, drag_id, egui::Sense::drag());
                
                if drag_response.drag_started() {
                    ui.memory_mut(|m| m.data.insert_temp(egui::Id::new("dragged_ent"), entity_id));
                }
                
                if drag_response.hovered() {
                    if let Some(dragged) = ui.memory(|m| m.data.get_temp::<u32>(egui::Id::new("dragged_ent"))) {
                        // Vurgu rengi ile bırakılabilecek yeri göster
                        ui.painter().rect_stroke(response.rect, 2.0, egui::Stroke::new(1.0, egui::Color32::YELLOW));
                        if ui.input(|i| i.pointer.any_released()) && dragged != entity_id {
                            state.reparent_request = Some((dragged, entity_id));
                            ui.memory_mut(|m| m.data.remove::<u32>(egui::Id::new("dragged_ent")));
                        }
                    }
                }
                // ------------------------------------
                
                response.context_menu(|ui| {
                    let is_hidden = world.borrow::<gizmo_core::component::IsHidden>().map(|c| c.contains(entity_id)).unwrap_or(false);
                    let hide_text = if is_hidden { "👁 Görünür Yap (Göster)" } else { "🙈 Gizle (Sakla)" };
                    if ui.button(hide_text).clicked() {
                        state.toggle_visibility_request = Some(entity_id);
                        ui.close_menu();
                    }
                    if ui.button("💾 Prefab Olarak Kaydet").clicked() {
                        let path = format!("demo/assets/prefabs/{}.prefab", entity_name.replace(" ", "_"));
                        state.prefab_save_request = Some((entity_id, path));
                        ui.close_menu();
                    }
                    if ui.button("📑 Çoğalt (Duplicate)").clicked() {
                        state.duplicate_request = Some(entity_id);
                        ui.close_menu();
                    }
                    if ui.button("🗑 Sil").clicked() {
                        state.despawn_request = Some(entity_id);
                        ui.close_menu();
                    }
                });
            })
            .body(|ui| {
                if let Some(children) = children_comp.as_ref().and_then(|c| c.get(entity_id)) {
                    for &child_id in &children.0 {
                        draw_entity_node(ui, world, child_id, state, names, children_comp, filter);
                    }
                }
            });
    } else {
        // Yaprak düğümü (çocuğu yok)
        let is_hidden = world.borrow::<gizmo_core::component::IsHidden>().map(|hc| hc.contains(entity_id)).unwrap_or(false);
        let label_text = if is_hidden { format!("  ● {} (Gizli)", entity_name) } else { format!("  ● {}", entity_name) };
        let label = if is_selected {
            egui::RichText::new(label_text).strong().color(egui::Color32::from_rgb(100, 200, 255))
        } else {
            egui::RichText::new(label_text)
        };
        
        let response = ui.selectable_label(is_selected, label);
        
        // --- Sürükle Bırak (Drag & Drop) ---
        let drag_id = egui::Id::new("drag_ent").with(entity_id);
        let drag_response = ui.interact(response.rect, drag_id, egui::Sense::drag());
        
        if drag_response.drag_started() {
            ui.memory_mut(|m| m.data.insert_temp(egui::Id::new("dragged_ent"), entity_id));
        }
        
        if drag_response.hovered() {
            if let Some(dragged) = ui.memory(|m| m.data.get_temp::<u32>(egui::Id::new("dragged_ent"))) {
                ui.painter().rect_stroke(response.rect, 2.0, egui::Stroke::new(1.0, egui::Color32::YELLOW));
                if ui.input(|i| i.pointer.any_released()) && dragged != entity_id {
                    state.reparent_request = Some((dragged, entity_id));
                    ui.memory_mut(|m| m.data.remove::<u32>(egui::Id::new("dragged_ent")));
                }
            }
        }
        // ------------------------------------

        if response.clicked() {
            if ui.input(|i| i.modifiers.command) {
                state.toggle_selection(entity_id);
            } else {
                state.select_exclusive(entity_id);
            }
        }
        
        response.context_menu(|ui| {
            let is_hidden = world.borrow::<gizmo_core::component::IsHidden>().map(|c| c.contains(entity_id)).unwrap_or(false);
            let hide_text = if is_hidden { "👁 Görünür Yap (Göster)" } else { "🙈 Gizle (Sakla)" };
            if ui.button(hide_text).clicked() {
                state.toggle_visibility_request = Some(entity_id);
                ui.close_menu();
            }
            if ui.button("💾 Prefab Olarak Kaydet").clicked() {
                let path = format!("demo/assets/prefabs/{}.prefab", entity_name.replace(" ", "_"));
                state.prefab_save_request = Some((entity_id, path));
                ui.close_menu();
            }
            if ui.button("📑 Çoğalt (Duplicate)").clicked() {
                state.duplicate_request = Some(entity_id);
                ui.close_menu();
            }
            if ui.button("🗑 Sil").clicked() {
                state.despawn_request = Some(entity_id);
                ui.close_menu();
            }
        });
    }
}

