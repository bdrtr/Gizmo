//! Scene Hierarchy Panel — Sol panel'de entity ağacını gösterir

use egui;
use gizmo_core::{World, EntityName, component::{Parent, Children}};
use crate::editor_state::EditorState;

/// Scene Hierarchy panelini çizer
pub fn draw_hierarchy(ctx: &egui::Context, world: &World, state: &mut EditorState) {
    egui::SidePanel::left("hierarchy_panel")
        .default_width(220.0)
        .min_width(160.0)
        .max_width(350.0)
        .show(ctx, |ui| {
            ui.heading("🌍 Sahne Hiyerarşisi");
            ui.separator();

            // Arama kutusu
            ui.horizontal(|ui| {
                ui.label("🔍");
                ui.text_edit_singleline(&mut state.hierarchy_filter);
            });
            ui.separator();

            // Sağ tık menüsü — boşluğa tıklayınca
            ui.interact(ui.available_rect_before_wrap(), ui.id().with("hierarchy_bg"), egui::Sense::click())
                .context_menu(|ui| {
                    if ui.button("➕ Boş Entity Ekle").clicked() {
                        state.status_message = "Entity oluşturma: Update döngüsünde yapılacak".to_string();
                        ui.close_menu();
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
        });
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

    let is_selected = state.selected_entity == Some(entity_id);
    let has_children = children_comp.as_ref()
        .and_then(|c| c.get(entity_id))
        .map(|c| !c.0.is_empty())
        .unwrap_or(false);

    if has_children {
        // Katlanabilir ağaç düğümü
        let id = ui.make_persistent_id(format!("entity_{}", entity_id));
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
            .show_header(ui, |ui| {
                let label = if is_selected {
                    egui::RichText::new(format!("📦 {}", entity_name)).strong().color(egui::Color32::from_rgb(100, 200, 255))
                } else {
                    egui::RichText::new(format!("📦 {}", entity_name))
                };
                
                if ui.selectable_label(is_selected, label).clicked() {
                    state.selected_entity = Some(entity_id);
                }
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
        let label = if is_selected {
            egui::RichText::new(format!("  🔹 {}", entity_name)).strong().color(egui::Color32::from_rgb(100, 200, 255))
        } else {
            egui::RichText::new(format!("  ● {}", entity_name))
        };
        
        if ui.selectable_label(is_selected, label).clicked() {
            state.selected_entity = Some(entity_id);
        }
    }

    // Sağ tık menüsü
    // (egui context_menu entity bazlı olası düğümlerde çalışır)
}
