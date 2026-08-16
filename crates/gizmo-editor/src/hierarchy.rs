//! Scene Hierarchy Panel — Sol panel'de entity ağacını gösterir

use crate::editor_state::EditorState;
use crate::theme::palette::{ACCENT, ACCENT_LIGHT, BORDER_HOT, SURFACE, TEXT_BODY, TEXT_BRIGHT, TEXT_DIM, TEXT_MUTED};
use egui;
use gizmo_core::{
    component::{Children, Parent},
    EntityName, World,
};

/// Scene Hierarchy sekmesini çizer
pub fn ui_hierarchy(ui: &mut egui::Ui, world: &World, state: &mut EditorState) {
    // The prototype's panel header: the name in letterspaced caps, the live count beside it, and
    // the panel's own actions on the right. A count in the header is not decoration — it is the
    // one number you check before asking why something is missing.
    let visible_count = world.iter_alive_entities().len();
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("H I E R A R C H Y")
                .size(10.0)
                .color(TEXT_MUTED),
        );
        ui.label(egui::RichText::new(visible_count.to_string()).size(10.0).color(TEXT_DIM));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.toggle_value(&mut state.hide_editor_entities, egui::RichText::new("krom").size(9.0))
                .on_hover_text("Editörün kendi nesnelerini gizle");
        });
    });
    ui.separator();

    // Arama kutusu
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut state.hierarchy_filter)
                .desired_width(f32::INFINITY)
                .hint_text("Filter entities…"),
        );
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
                2,
                egui::Stroke::new(2.0_f32, egui::Color32::from_rgb(100, 255, 100)),
                egui::StrokeKind::Inside,
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
                state.spawn_request = Some(crate::editor_state::SpawnKind::Empty);
                ui.close();
            }
            if ui.button("📂 Grup (Klasör)").clicked() {
                state.spawn_request = Some(crate::editor_state::SpawnKind::Group);
                ui.close();
            }
        });
        ui.menu_button("🔶 3D Primitif", |ui| {
            if ui.button("📦 Küp (Cube)").clicked() {
                state.spawn_request = Some(crate::editor_state::SpawnKind::Cube);
                ui.close();
            }
            if ui.button("🔴 Küre (Sphere)").clicked() {
                state.spawn_request = Some(crate::editor_state::SpawnKind::Sphere);
                ui.close();
            }
            if ui.button("▬ Düzlem (Plane)").clicked() {
                state.spawn_request = Some(crate::editor_state::SpawnKind::Plane);
                ui.close();
            }
            if ui.button("🔵 Silindir (Cylinder)").clicked() {
                state.spawn_request = Some(crate::editor_state::SpawnKind::Cylinder);
                ui.close();
            }
            if ui.button("💊 Kapsül (Capsule)").clicked() {
                state.spawn_request = Some(crate::editor_state::SpawnKind::Capsule);
                ui.close();
            }
        });
        ui.menu_button("💡 Işık & Kamera", |ui| {
            if ui.button("💡 Nokta Işığı (Point Light)").clicked() {
                state.spawn_request = Some(crate::editor_state::SpawnKind::PointLight);
                ui.close();
            }
            if ui.button("📷 Kamera (Camera)").clicked() {
                state.spawn_request = Some(crate::editor_state::SpawnKind::Camera);
                ui.close();
            }
        });
        ui.menu_button("✨ Efekt", |ui| {
            if ui.button("✨ Particle Emitter").clicked() {
                state.spawn_request = Some(crate::editor_state::SpawnKind::ParticleEmitter);
                ui.close();
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
                ui.close();
            }
        }
    });

    // Entity listesini oluştur
    egui::ScrollArea::vertical().show(ui, |ui| {
        let names = world.borrow::<EntityName>();
        let editor_only_markers = world.borrow::<gizmo_core::component::EditorOnly>();
        let parents = world.borrow::<Parent>();
        let children_comp = world.borrow::<Children>();
        let is_hidden_comp = world.borrow::<gizmo_core::component::IsHidden>();
        let is_deleted_comp = world.borrow::<gizmo_core::component::IsDeleted>();

        let filter_lower = state.hierarchy_filter.to_lowercase(); // Bir kez hesaplanır

        // ROOT entity'leri filtrele (Iter alive bazından cachelenir) O(N) tek geçiş
        let root_entities: Vec<gizmo_core::entity::Entity> = world
            .iter_alive_entities()
            .into_iter()
            .filter(|e| parents.get(e.id()).is_none() && is_deleted_comp.get(e.id()).is_none())
            .collect();

        // Root entity'leri çiz
        for entity in root_entities {
            // Editor-only (Hayali) objeleri hiyerarşide listeleme
            let is_editor_only = gizmo_core::component::is_editor_only(
                editor_only_markers.get(entity.id()).is_some(),
                names.get(entity.id()).map(|e| e.0.as_str()),
            );

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
                &editor_only_markers,
                &filter_lower,
            );
        }
    });

    // No total at the bottom: the header carries the count, and two of the same number in one
    // panel invites the reader to work out why they differ.
}

/// Tek bir entity node'unu recursive olarak çizer
fn draw_entity_node(
    ui: &mut egui::Ui,
    world: &World,
    entity: gizmo_core::entity::Entity,
    state: &mut EditorState,
    names: &gizmo_core::StorageView<EntityName>,
    children_comp: &gizmo_core::StorageView<Children>,
    is_hidden_comp: &gizmo_core::StorageView<gizmo_core::component::IsHidden>,
    is_deleted_comp: &gizmo_core::StorageView<gizmo_core::component::IsDeleted>,
    editor_only_markers: &gizmo_core::StorageView<gizmo_core::component::EditorOnly>,
    filter_lower: &str,
) {
    let entity_name = names
        .get(entity.id())
        .map(|n| n.0.clone())
        .unwrap_or_else(|| format!("Entity_{}", entity.id()));

    if state.hide_editor_entities
        && gizmo_core::component::is_editor_only(
            editor_only_markers.get(entity.id()).is_some(),
            Some(entity_name.as_str()),
        )
    {
        return;
    }

    if is_deleted_comp.get(entity.id()).is_some() {
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
                            editor_only_markers,
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

    let is_hidden = is_hidden_comp.get(entity.id()).is_some();

    // Düğüm Çizimi + Drag Drop Kapsüllemesi
    let mut draw_row = |ui: &mut egui::Ui| {
        // The prototype's row: an indent, a type square, the name, then the component badges
        // and a visibility dot pinned to the right edge.
        let _row_id = egui::Id::new("hierarchy_row").with(entity.id());
        let desired_size = egui::vec2(ui.available_width(), 19.0);
        let (rect, response) = ui.allocate_exact_size(desired_size, egui::Sense::click_and_drag());
        let painter = ui.painter();

        // Selection is a full-width accent fill, as in the prototype — not a tinted label. It is
        // the strongest thing on the panel because "which entity am I editing" is the question the
        // hierarchy exists to answer.
        if is_selected {
            painter.rect_filled(rect, 0.0, ACCENT);
        } else if response.hovered() {
            painter.rect_filled(rect, 0.0, SURFACE);
        }

        let dim = is_hidden || is_deleted_comp.get(entity.id()).is_some();
        let name_color = if is_selected {
            TEXT_BRIGHT
        } else if dim {
            TEXT_DIM
        } else if has_children {
            TEXT_BRIGHT
        } else {
            TEXT_BODY
        };

        // The type square. A filled corner marks a group, mirroring the prototype's disclosure.
        let sq = egui::Rect::from_center_size(
            egui::pos2(rect.left() + 12.0, rect.center().y),
            egui::vec2(7.0, 7.0),
        );
        painter.rect_stroke(
            sq,
            0.0,
            egui::Stroke::new(1.0_f32, if is_selected { TEXT_BRIGHT } else { BORDER_HOT }),
            egui::StrokeKind::Inside,
        );
        if has_children {
            painter.rect_filled(sq.shrink(2.0), 0.0, if is_selected { TEXT_BRIGHT } else { BORDER_HOT });
        }

        let font = egui::FontId::proportional(11.0);
        let name_pos = egui::pos2(rect.left() + 22.0, rect.center().y);
        // Anchor the right-hand column to the CLIP rect, not to the row rect: inside a scroll
        // area the row is allocated the content width, which is wider than what is visible, so
        // anchoring to `rect.right()` pushes the dot and the badges out past the panel edge —
        // which is exactly where they went the first time.
        let right = rect.right().min(ui.clip_rect().right() - 4.0);

        // The visibility dot, clickable. It was buried in the context menu; the prototype puts it
        // on the row because it is the one per-entity action you take constantly.
        let dot_c = egui::pos2(right - 8.0, rect.center().y);
        let dot_color = if is_selected { TEXT_BRIGHT } else if is_hidden { TEXT_DIM } else { TEXT_MUTED };
        if is_hidden {
            painter.circle_stroke(dot_c, 3.5, egui::Stroke::new(1.0_f32, dot_color));
        } else {
            painter.circle_filled(dot_c, 3.0, dot_color);
        }
        let dot_hit = egui::Rect::from_center_size(dot_c, egui::vec2(16.0, 16.0));
        if ui.rect_contains_pointer(dot_hit) && ui.input(|i| i.pointer.primary_pressed()) {
            state.toggle_visibility_requests.push(entity);
        }

        // Component badges, derived from what the entity actually has rather than from its name.
        let badge_color = if is_selected { TEXT_BRIGHT } else { ACCENT_LIGHT };
        let mut badge_x = right - 20.0;
        for tag in entity_badges(world, entity.id()).into_iter().rev() {
            let bg = painter.layout_no_wrap(tag.to_string(), egui::FontId::proportional(9.0), badge_color);
            badge_x -= bg.size().x + 6.0;
            painter.galley(egui::pos2(badge_x, rect.center().y - bg.size().y * 0.5), bg, badge_color);
        }

        // The name goes in LAST, clipped to whatever the badges left. Drawing it first is what put
        // "Directional Light" underneath its own `lit` tag: a long name in a 175 px panel runs
        // straight through the right-hand column.
        let name_max = (badge_x - 6.0 - name_pos.x).max(24.0);
        let mut job = egui::text::LayoutJob::single_section(
            entity_name.clone(),
            egui::TextFormat { font_id: font, color: name_color, ..Default::default() },
        );
        job.wrap = egui::text::TextWrapping {
            max_width: name_max,
            max_rows: 1,
            break_anywhere: true,
            overflow_character: Some('…'),
        };
        let galley = painter.layout_job(job);
        painter.galley(
            egui::pos2(name_pos.x, name_pos.y - galley.size().y * 0.5),
            galley.clone(),
            name_color,
        );
        // Hidden entities are struck through, the way the prototype shows a disabled row — a
        // greyed name alone reads as "unimportant" rather than "not in the scene right now".
        if dim {
            painter.line_segment(
                [
                    egui::pos2(name_pos.x, name_pos.y),
                    egui::pos2(name_pos.x + galley.size().x, name_pos.y),
                ],
                egui::Stroke::new(1.0_f32, TEXT_DIM),
            );
        }

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
                    2,
                    egui::Stroke::new(1.0_f32, egui::Color32::YELLOW),
                    egui::StrokeKind::Inside,
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
                ui.close();
            }

            ui.separator();

            // === DÜZENLEME ===
            if ui.button("📑 Çoğalt (Ctrl+D)").clicked() {
                state.duplicate_requests.push(entity);
                ui.close();
            }

            if ui.button("🗑 Sil (Delete)").clicked() {
                state.despawn_requests.push(entity);
                ui.close();
            }

            ui.separator();

            // === HİYERARŞİ ===
            if ui.button("➕ Çocuk Entity Ekle").clicked() {
                // Boş child entity oluştur ve bu entity'nin altına bağla
                state.spawn_request = Some(crate::editor_state::SpawnKind::Empty);
                // spawn sonrası reparent yapılacak → spawn_request işlenirken
                // parent'ı ayarlamak için pending_child_parent kullanılacak
                state.pending_child_parent = Some(entity);
                ui.close();
            }

            // Dövüş oyunu kısayolları
            if ui.button("🥊 Hitbox Ekle (Çocuk)").clicked() {
                state.spawn_request = Some(crate::editor_state::SpawnKind::Empty);
                state.pending_child_parent = Some(entity);
                state.pending_child_components.push("Hitbox".to_string());
                ui.close();
            }

            if ui.button("🛡 Hurtbox Ekle (Çocuk)").clicked() {
                state.spawn_request = Some(crate::editor_state::SpawnKind::Empty);
                state.pending_child_parent = Some(entity);
                state.pending_child_components.push("Hurtbox".to_string());
                ui.close();
            }

            ui.separator();

            if ui.button("🔗 Kökten Ayır (Unparent)").clicked() {
                state.unparent_request = Some(entity);
                ui.close();
            }

            // Seçili birden fazla obje varsa gruplama butonu
            if state.selection.entities.len() > 1
                && ui.button("📂 Seçilileri Grupla").clicked() {
                    // Boş bir parent entity oluştur, sonra seçili objeleri ona bağla. The second
                    // half is `pending_group_members`: without it this button spawned a stray
                    // empty entity and left the selection untouched, which is what it did for as
                    // long as it existed.
                    state.pending_group_members = state.selection.entities.iter().copied().collect();
                    state.spawn_request = Some(crate::editor_state::SpawnKind::Group);
                    ui.close();
                }

            ui.separator();

            // === DIŞA AKTARMA ===
            if ui.button("💾 Prefab Olarak Kaydet").clicked() {
                let path = format!(
                    "demo/assets/prefabs/{}.prefab",
                    entity_name.replace(" ", "_")
                );
                state.prefab_save_request = Some((entity, path));
                ui.close();
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
            // Painted, not a glyph. egui's bundled font renders ▼/▶ as empty boxes at this size —
            // the same missing-glyph problem the toolbar icons had — and a disclosure control that
            // shows a blank square is worse than none.
            let (tri_rect, tri_resp) =
                ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::click());
            {
                let c = tri_rect.center();
                let col = if tri_resp.hovered() { TEXT_BRIGHT } else { TEXT_MUTED };
                let pts = if is_open {
                    // ▼
                    vec![
                        egui::pos2(c.x - 3.5, c.y - 2.0),
                        egui::pos2(c.x + 3.5, c.y - 2.0),
                        egui::pos2(c.x, c.y + 2.5),
                    ]
                } else {
                    // ▶
                    vec![
                        egui::pos2(c.x - 2.0, c.y - 3.5),
                        egui::pos2(c.x - 2.0, c.y + 3.5),
                        egui::pos2(c.x + 2.5, c.y),
                    ]
                };
                ui.painter().add(egui::Shape::convex_polygon(pts, col, egui::Stroke::NONE));
            }
            if tri_resp.clicked() {
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
                                    editor_only_markers,
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


/// The short component tags shown at the right edge of a hierarchy row.
///
/// Derived from the components an entity actually carries. The prototype shows tags like `lit` and
/// `rs`; inventing them from the entity's *name* — which is what the row icon used to do, matching
/// on the substrings "camera" and "light" — would make a crate called "lightbox" claim to be a
/// lamp. Capped at two: the column is 40 px wide and a row that spells out five components stops
/// being scannable, which is the only thing this column is for.
fn entity_badges(world: &World, id: u32) -> Vec<&'static str> {
    let mut tags = Vec::new();
    if world.borrow::<gizmo_renderer::components::Camera>().get(id).is_some() {
        tags.push("cam");
    }
    if world.borrow::<gizmo_renderer::components::DirectionalLight>().get(id).is_some()
        || world.borrow::<gizmo_renderer::components::PointLight>().get(id).is_some()
        || world.borrow::<gizmo_renderer::components::SpotLight>().get(id).is_some()
    {
        tags.push("lit");
    }
    if world.borrow::<gizmo_physics_rigid::components::RigidBody>().get(id).is_some() {
        tags.push("rb");
    }
    if world.borrow::<gizmo_physics_core::Collider>().get(id).is_some() {
        tags.push("col");
    }
    tags.truncate(2);
    tags
}
