//! Scene Hierarchy Panel — Sol panel'de entity ağacını gösterir

use crate::editor_state::EditorState;
use egui;
use gizmo_core::{
    component::{Children, Parent},
    EntityName, World,
};

/// Scene Hierarchy sekmesini çizer
pub fn ui_hierarchy(ui: &mut egui::Ui, world: &World, state: &mut EditorState) {
    ui.heading("🌍 Sahne Hiyerarşisi");
    ui.separator();

    // Arama kutusu
    ui.horizontal(|ui| {
        ui.label("🔍");
        ui.add(egui::TextEdit::singleline(&mut state.hierarchy_filter).desired_width(120.0));
        ui.checkbox(&mut state.hide_editor_entities, "Gizle");
    });
    ui.separator();

    // Hierarchy ScrollArea'nın tamamını "Asset Drop" alanı olarak kabul edebilmek için arka planı değerlendireceğiz
    let bg_response = ui.interact(
        ui.available_rect_before_wrap(),
        ui.id().with("hierarchy_bg"),
        egui::Sense::click_and_drag(),
    );

    // Asset Drop Yakalama
    if bg_response.hovered() {
        if let Some(dragged_path) = state.dragged_asset.clone() {
            ui.painter().rect_stroke(
                bg_response.rect,
                2.0,
                egui::Stroke::new(2.0, egui::Color32::from_rgb(100, 255, 100)),
            );
            if ui.input(|i| i.pointer.any_released()) {
                state.spawn_asset_request = Some(dragged_path);
                state.dragged_asset = None;
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
        if let Some(dragged) =
            ui.memory(|mem| mem.data.get_temp::<gizmo_core::entity::Entity>(egui::Id::new("dragged_ent")))
        {
            if ui.button("Düzene Geri Al (Unparent)").clicked() { // Düzeltildi
                state.unparent_request = Some(dragged);
                ui.memory_mut(|mem| mem.data.remove::<gizmo_core::entity::Entity>(egui::Id::new("dragged_ent")));
                ui.close_menu();
            }
        }
    });

    // Entity listesini oluştur
    egui::ScrollArea::vertical().show(ui, |ui| {
        let names = world.borrow::<EntityName>().expect("ECS Aliasing Error");
        let parents = world.borrow::<Parent>().expect("ECS Aliasing Error");
        let children_comp = world.borrow::<Children>().expect("ECS Aliasing Error");
        let is_hidden_comp = world.borrow::<gizmo_core::component::IsHidden>().expect("ECS Aliasing Error");

        let filter_lower = state.hierarchy_filter.to_lowercase(); // Bir kez hesaplanır

        // ROOT entity'leri filtrele (Iter alive bazından cachelenir) O(N) tek geçiş
        let root_entities: Vec<gizmo_core::entity::Entity> = world.iter_alive_entities()
            .filter(|e| !parents.contains(e.id()))
            .collect();

        // Root entity'leri çiz
        for entity in root_entities {
            // Editor-only (Hayali) objeleri hiyerarşide listeleme
            let is_editor_only = names
                .get(entity.id())
                .map(|e| e.0.starts_with("Editor ") || e.0 == "Highlight Box")
                .unwrap_or(false);

            if state.hide_editor_entities && is_editor_only {
                continue;
            }

            draw_entity_node(
                ui,
                world,
                entity,
                state,
                &names,
                &children_comp,
                &is_hidden_comp,
                &filter_lower,
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
    entity: gizmo_core::entity::Entity,
    state: &mut EditorState,
    names: &gizmo_core::storage::StorageView<EntityName>,
    children_comp: &gizmo_core::storage::StorageView<Children>,
    is_hidden_comp: &gizmo_core::storage::StorageView<gizmo_core::component::IsHidden>,
    filter_lower: &str,
) {
    let entity_name = names
        .get(entity.id())
        .map(|n| n.0.clone())
        .unwrap_or_else(|| format!("Entity_{}", entity.id()));

    // Editor objelerini Hiyerarşiden tamamen gizle (eğer ayar açıksa)
    if state.hide_editor_entities && (entity_name.starts_with("Editor ") || entity_name == "Highlight Box") {
        return;
    }

    // Filtre uygulaması
    if !filter_lower.is_empty() && !entity_name.to_lowercase().contains(filter_lower) {
        // Bu entity filtrede yoksa ama child'ları olabilir — onları kontrol et
        if let Some(children) = children_comp.get(entity.id()) {
            for &child_id in &children.0 {
                // Generation güvenliği sağlandı, world üzerinden çekildi
                if let Some(child_ent) = world.get_entity(child_id) {
                    if world.is_alive(child_ent) {
                        draw_entity_node(ui, world, child_ent, state, names, children_comp, is_hidden_comp, filter_lower);
                    }
                }
            }
        }
        return;
    }

    let is_selected = state.is_selected(entity);
    let has_children = children_comp
        .get(entity.id())
        .map(|c| !c.0.is_empty())
        .unwrap_or(false);

    let is_hidden = is_hidden_comp.contains(entity.id());

    // Düğüm Çizimi + Drag Drop Kapsüllemesi (Satır Duplicate Engellendi)
    let mut draw_row = |ui: &mut egui::Ui| {
        let label_text = if is_hidden {
            format!("📦 {} (Gizli)", entity_name)
        } else {
            format!("📦 {}", entity_name)
        };
        
        let label = if is_selected {
            egui::RichText::new(label_text).strong().color(egui::Color32::from_rgb(100, 200, 255))
        } else {
            egui::RichText::new(label_text)
        };

        let response = ui.selectable_label(is_selected, label);
        
        if response.clicked() {
            if ui.input(|i| i.modifiers.command) {
                state.toggle_selection(entity);
            } else {
                state.select_exclusive(entity);
            }
        }

        // --- Sürükle Bırak (Drag & Drop) ---
        let drag_id = egui::Id::new("drag_ent").with(entity.id());
        let drag_response = ui.interact(response.rect, drag_id, egui::Sense::drag());

        if drag_response.drag_started() {
            ui.memory_mut(|m| m.data.insert_temp(egui::Id::new("dragged_ent"), entity));
        }

        if drag_response.hovered() {
            if let Some(dragged) = ui.memory(|m| m.data.get_temp::<gizmo_core::entity::Entity>(egui::Id::new("dragged_ent"))) {
                // Vurgu rengi ile bırakılabilecek yeri göster
                ui.painter().rect_stroke(response.rect, 2.0, egui::Stroke::new(1.0, egui::Color32::YELLOW));
                if ui.input(|i| i.pointer.any_released()) && dragged != entity {
                    state.reparent_request = Some((dragged, entity));
                    ui.memory_mut(|m| m.data.remove::<gizmo_core::entity::Entity>(egui::Id::new("dragged_ent")));
                }
            }
        }

        response.context_menu(|ui| {
            let hide_text = if is_hidden { "👁 Görünür Yap (Göster)" } else { "🙈 Gizle (Sakla)" };
            if ui.button(hide_text).clicked() { state.toggle_visibility_requests.push(entity); ui.close_menu(); }
            if ui.button("💾 Prefab Olarak Kaydet").clicked() {
                // Asset path yönetimi standardize edilmeli, şimdilik prefix dinamik
                let path = format!("demo/assets/prefabs/{}.prefab", entity_name.replace(" ", "_"));
                state.prefab_save_request = Some((entity, path));
                ui.close_menu();
            }
            if ui.button("📑 Çoğalt (Duplicate)").clicked() { state.duplicate_requests.push(entity); ui.close_menu(); }
            if ui.button("🗑 Sil").clicked() { state.despawn_requests.push(entity); ui.close_menu(); }
        });
    };

    if has_children {
        // Katlanabilir ağaç düğümü
        let id = ui.make_persistent_id(format!("entity_{}", entity.id()));
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
            .show_header(ui, |ui| { draw_row(ui); })
            .body(|ui| {
                if let Some(children) = children_comp.get(entity.id()) {
                    for &child_id in &children.0 {
                        if let Some(child_ent) = world.get_entity(child_id) {
                            if world.is_alive(child_ent) {
                                draw_entity_node(ui, world, child_ent, state, names, children_comp, is_hidden_comp, filter_lower);
                            }
                        }
                    }
                }
            });
    } else {
        // Alt elemanı olmayan düz düğüm
        draw_row(ui);
    }
}
