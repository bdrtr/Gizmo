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
    if let Some(dragged_path) = state.dragged_asset.clone() {
        let latest_pos = ui.input(|i| i.pointer.latest_pos());
        let in_hierarchy = latest_pos.map(|p| bg_response.rect.contains(p)).unwrap_or(false);
        
        if in_hierarchy {
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
        ui.menu_button("➕ Boş Obje", |ui| {
            if ui.button("📦 Boş Entity").clicked() {
                state.spawn_request = Some("Empty".to_string());
                ui.close_menu();
            }
            if ui.button("📂 Grup (Klasör)").clicked() {
                state.spawn_request = Some("Group".to_string());
                ui.close_menu();
            }
        });
        ui.menu_button("🔶 3D Primitif", |ui| {
            if ui.button("📦 Küp (Cube)").clicked() {
                state.spawn_request = Some("Cube".to_string());
                ui.close_menu();
            }
            if ui.button("🔴 Küre (Sphere)").clicked() {
                state.spawn_request = Some("Sphere".to_string());
                ui.close_menu();
            }
            if ui.button("▬ Düzlem (Plane)").clicked() {
                state.spawn_request = Some("Plane".to_string());
                ui.close_menu();
            }
            if ui.button("🔵 Silindir (Cylinder)").clicked() {
                state.spawn_request = Some("Cylinder".to_string());
                ui.close_menu();
            }
            if ui.button("💊 Kapsül (Capsule)").clicked() {
                state.spawn_request = Some("Capsule".to_string());
                ui.close_menu();
            }
        });
        ui.menu_button("💡 Işık & Kamera", |ui| {
            if ui.button("💡 Nokta Işığı (Point Light)").clicked() {
                state.spawn_request = Some("PointLight".to_string());
                ui.close_menu();
            }
            if ui.button("📷 Kamera (Camera)").clicked() {
                state.spawn_request = Some("Camera".to_string());
                ui.close_menu();
            }
        });
        ui.menu_button("✨ Efekt", |ui| {
            if ui.button("✨ Particle Emitter").clicked() {
                state.spawn_request = Some("ParticleEmitter".to_string());
                ui.close_menu();
            }
        });
        ui.separator();

        // Unparent yapabilmek için (Kök yapmak)
        if let Some(dragged) = ui.memory(|mem| {
            mem.data
                .get_temp::<gizmo_core::entity::Entity>(egui::Id::new("dragged_ent"))
        }) {
            if ui.button("🔗 Kökten Ayır (Unparent)").clicked() {
                state.unparent_request = Some(dragged);
                ui.memory_mut(|mem| {
                    mem.data
                        .remove::<gizmo_core::entity::Entity>(egui::Id::new("dragged_ent"))
                });
                ui.close_menu();
            }
        }
    });

    // Entity listesini oluştur
    egui::ScrollArea::vertical().show(ui, |ui| {
        let names = world.borrow::<EntityName>();
        let parents = world.borrow::<Parent>();
        let children_comp = world.borrow::<Children>();
        let is_hidden_comp = world.borrow::<gizmo_core::component::IsHidden>();
        let is_deleted_comp = world.borrow::<gizmo_core::component::IsDeleted>();

        let filter_lower = state.hierarchy_filter.to_lowercase(); // Bir kez hesaplanır

        // ROOT entity'leri filtrele (Iter alive bazından cachelenir) O(N) tek geçiş
        let root_entities: Vec<gizmo_core::entity::Entity> = world
            .iter_alive_entities()
            .into_iter()
            .filter(|e| !parents.contains(e.id()) && !is_deleted_comp.contains(e.id()))
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
                &is_deleted_comp,
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
    is_deleted_comp: &gizmo_core::storage::StorageView<gizmo_core::component::IsDeleted>,
    filter_lower: &str,
) {
    let entity_name = names
        .get(entity.id())
        .map(|n| n.0.clone())
        .unwrap_or_else(|| format!("Entity_{}", entity.id()));

    if state.hide_editor_entities
        && (entity_name.starts_with("Editor ") || entity_name == "Highlight Box")
    {
        return;
    }

    if is_deleted_comp.contains(entity.id()) {
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
                        draw_entity_node(
                            ui,
                            world,
                            child_ent,
                            state,
                            names,
                            children_comp,
                            is_hidden_comp,
                            is_deleted_comp,
                            filter_lower,
                        );
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

    // Düğüm Çizimi + Drag Drop Kapsüllemesi
    let mut draw_row = |ui: &mut egui::Ui| {
        let icon = if entity_name.to_lowercase().contains("camera") {
            "📷"
        } else if entity_name.to_lowercase().contains("light") {
            "💡"
        } else {
            "📦"
        };

        let label_text = if is_hidden {
            format!("{} {} (Gizli)", icon, entity_name)
        } else {
            format!("{} {}", icon, entity_name)
        };

        // Tek bir interaction alanı: hem click hem drag (önceki ui.interact çakışmasını önler)
        let _row_id = egui::Id::new("hierarchy_row").with(entity.id());
        let desired_size = egui::vec2(ui.available_width(), ui.spacing().interact_size.y);
        let (rect, response) = ui.allocate_exact_size(desired_size, egui::Sense::click_and_drag());

        // Arka plan — seçili ise vurgu
        if is_selected || response.hovered() {
            let bg_color = if is_selected {
                ui.style().visuals.selection.bg_fill
            } else {
                egui::Color32::from_white_alpha(8)
            };
            ui.painter().rect_filled(rect, 2.0, bg_color);
        }

        // Metin
        let text_color = if is_selected {
            egui::Color32::from_rgb(100, 200, 255)
        } else {
            ui.style().visuals.text_color()
        };
        let font = if is_selected {
            egui::FontId::proportional(13.0)
        } else {
            egui::FontId::proportional(13.0)
        };
        ui.painter().text(
            rect.left_center() + egui::vec2(4.0, 0.0),
            egui::Align2::LEFT_CENTER,
            &label_text,
            font,
            text_color,
        );

        // Tıklama — seçim
        if response.clicked() {
            state.log_info(&format!("Hiyerarşiden tıklandı: {}", entity_name));
            if ui.input(|i| i.modifiers.command) {
                state.toggle_selection(entity);
            } else {
                state.select_exclusive(entity);
            }
        }

        // --- Sürükle Bırak (Drag & Drop) --- aynı response üzerinden
        if response.drag_started() {
            ui.memory_mut(|m| m.data.insert_temp(egui::Id::new("dragged_ent"), entity));
        }

        if response.hovered() {
            if let Some(dragged) = ui.memory(|m| {
                m.data
                    .get_temp::<gizmo_core::entity::Entity>(egui::Id::new("dragged_ent"))
            }) {
                // Vurgu rengi ile bırakılabilecek yeri göster
                ui.painter().rect_stroke(
                    rect,
                    2.0,
                    egui::Stroke::new(1.0, egui::Color32::YELLOW),
                );
                if ui.input(|i| i.pointer.any_released()) && dragged != entity {
                    state.reparent_request = Some((dragged, entity));
                    ui.memory_mut(|m| {
                        m.data
                            .remove::<gizmo_core::entity::Entity>(egui::Id::new("dragged_ent"))
                    });
                }
            }
        }

        response.context_menu(|ui| {
            // === GÖRÜNÜRLÜK ===
            let hide_text = if is_hidden {
                "👁 Görünür Yap (Göster)"
            } else {
                "🙈 Gizle (H)"
            };
            if ui.button(hide_text).clicked() {
                state.toggle_visibility_requests.push(entity);
                ui.close_menu();
            }

            ui.separator();

            // === DÜZENLEME ===
            if ui.button("📑 Çoğalt (Ctrl+D)").clicked() {
                state.duplicate_requests.push(entity);
                ui.close_menu();
            }

            if ui.button("🗑 Sil (Delete)").clicked() {
                state.despawn_requests.push(entity);
                ui.close_menu();
            }

            ui.separator();

            // === HİYERARŞİ ===
            if ui.button("➕ Çocuk Entity Ekle").clicked() {
                // Boş child entity oluştur ve bu entity'nin altına bağla
                state.spawn_request = Some("Empty".to_string());
                // spawn sonrası reparent yapılacak → spawn_request işlenirken
                // parent'ı ayarlamak için pending_child_parent kullanılacak
                state.pending_child_parent = Some(entity);
                ui.close_menu();
            }

            // Dövüş oyunu kısayolları
            if ui.button("🥊 Hitbox Ekle (Çocuk)").clicked() {
                state.spawn_request = Some("Empty".to_string());
                state.pending_child_parent = Some(entity);
                state.pending_child_components.push("Hitbox".to_string());
                ui.close_menu();
            }

            if ui.button("🛡 Hurtbox Ekle (Çocuk)").clicked() {
                state.spawn_request = Some("Empty".to_string());
                state.pending_child_parent = Some(entity);
                state.pending_child_components.push("Hurtbox".to_string());
                ui.close_menu();
            }

            ui.separator();

            if ui.button("🔗 Kökten Ayır (Unparent)").clicked() {
                state.unparent_request = Some(entity);
                ui.close_menu();
            }

            // Seçili birden fazla obje varsa gruplama butonu
            if state.selection.entities.len() > 1 {
                if ui.button("📂 Seçilileri Grupla").clicked() {
                    // Boş bir parent entity oluştur, sonra seçili objeleri ona bağla
                    state.spawn_request = Some("Group".to_string());
                    ui.close_menu();
                }
            }

            ui.separator();

            // === DIŞA AKTARMA ===
            if ui.button("💾 Prefab Olarak Kaydet").clicked() {
                let path = format!(
                    "demo/assets/prefabs/{}.prefab",
                    entity_name.replace(" ", "_")
                );
                state.prefab_save_request = Some((entity, path));
                ui.close_menu();
            }
        });
    };

    if has_children {
        // Katlanabilir ağaç düğümü — Toggle ve Label ayrı click alanı
        let id = ui.make_persistent_id(format!("entity_{}", entity.id()));
        let mut collapsing_state =
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true);
        let is_open = collapsing_state.is_open();

        // Yatay satır: [▼ toggle] [seçilebilir label]
        let _header_res = ui.horizontal(|ui| {
            // Küçük üçgen toggle butonu (sadece bu alana tıklanınca aç/kapa)
            let triangle = if is_open { "▼" } else { "▶" };
            if ui
                .add(egui::Button::new(
                    egui::RichText::new(triangle).size(10.0),
                ).frame(false).min_size(egui::vec2(14.0, 14.0)))
                .clicked()
            {
                collapsing_state.toggle(ui);
            }
            // Seçilebilir label (ayrı click alanı — seçim burada)
            draw_row(ui);
        });

        collapsing_state.store(ui.ctx());

        // Açıksa çocukları girintili çiz
        if is_open {
            ui.indent(id, |ui| {
                if let Some(children) = children_comp.get(entity.id()) {
                    for &child_id in &children.0 {
                        if let Some(child_ent) = world.get_entity(child_id) {
                            if world.is_alive(child_ent) {
                                draw_entity_node(
                                    ui,
                                    world,
                                    child_ent,
                                    state,
                                    names,
                                    children_comp,
                                    is_hidden_comp,
                                    is_deleted_comp,
                                    filter_lower,
                                );
                            }
                        }
                    }
                }
            });
        }
    } else {
        // Alt elemanı olmayan düz düğüm
        draw_row(ui);
    }
}
