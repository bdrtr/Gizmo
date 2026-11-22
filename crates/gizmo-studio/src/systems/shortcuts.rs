use crate::state::StudioState;
use gizmo::editor::EditorState;
use gizmo::prelude::*;

/// Applies the studio's keyboard shortcuts for this frame: the Ctrl, the Alt and the single keys.
pub fn handle_editor_shortcuts(
    world: &mut World,
    editor_state: &mut EditorState,
    state: &StudioState,
    input: &Input,
) {
    // --- MODİFİER TUŞLARI ---
    let ctrl_pressed = input.is_key_pressed(gizmo::winit::keyboard::KeyCode::ControlLeft as u32)
        || input.is_key_pressed(gizmo::winit::keyboard::KeyCode::ControlRight as u32);
    let shift_pressed = input.is_key_pressed(gizmo::winit::keyboard::KeyCode::ShiftLeft as u32)
        || input.is_key_pressed(gizmo::winit::keyboard::KeyCode::ShiftRight as u32);
    let alt_pressed = input.is_key_pressed(gizmo::winit::keyboard::KeyCode::AltLeft as u32)
        || input.is_key_pressed(gizmo::winit::keyboard::KeyCode::AltRight as u32);


    if ctrl_pressed {
        ctrl_shortcuts(world, editor_state, input, shift_pressed);
    } else if alt_pressed {
        alt_shortcuts(world, editor_state, input);
    } else {
        plain_shortcuts(world, editor_state, state, input, shift_pressed);
    }
}

/// Ctrl + a key: undo and redo, copy, paste and duplicate, save, a new scene, select all.
fn ctrl_shortcuts(world: &mut World, editor_state: &mut EditorState, input: &Input, shift_pressed: bool) {
    // Ctrl+Z → Undo / Ctrl+Shift+Z → Redo
    if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::KeyZ as u32) {
        if shift_pressed {
            editor_state.history.redo(world);
            editor_state.status_message = "↩ Redo".to_string();
        } else {
            editor_state.history.undo(world);
            editor_state.status_message = "↪ Undo".to_string();
        }
    }

    // Ctrl+Y → Redo (Alternatif)
    if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::KeyY as u32) {
        editor_state.history.redo(world);
        editor_state.status_message = "↩ Redo".to_string();
    }


    // Ctrl+C → Kopyala (Copy)
    if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::KeyC as u32) {
        editor_state.clipboard_entities.clear();
        for &entity in editor_state.selection.entities.iter() {
            editor_state.clipboard_entities.push(entity);
        }
        if !editor_state.clipboard_entities.is_empty() {
            editor_state.status_message = format!(
                "📋 {} obje panoya kopyalandı",
                editor_state.clipboard_entities.len()
            );
        }
    }

    // Ctrl+V → Yapıştır (Paste)
    if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::KeyV as u32) {
        let count = editor_state.clipboard_entities.len();
        for &entity in &editor_state.clipboard_entities {
            editor_state.duplicate_requests.push(entity);
        }
        if count > 0 {
            editor_state.status_message = format!(
                "📥 {} obje yapıştırıldı",
                count
            );
        }
    }

    // Ctrl+D → Çoğalt (Duplicate)
    if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::KeyD as u32) {
        for &entity in editor_state.selection.entities.iter() {
            editor_state.duplicate_requests.push(entity);
        }
        if !editor_state.selection.entities.is_empty() {
            editor_state.status_message = format!(
                "📋 {} obje çoğaltılıyor…",
                editor_state.selection.entities.len()
            );
        }
    }

    // Ctrl+S → Hızlı Kaydet (yol varsa) / Dialog aç (yol yoksa)
    if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::KeyS as u32) {
        if !editor_state.scene_path.is_empty() {
            editor_state.scene.save_request = Some(editor_state.scene_path.clone());
            editor_state.status_message = format!("💾 Kaydediliyor: {}", editor_state.scene_path);
        } else {
            // Sahne yolu yoksa save dialog isteği işaretle
            // (lib.rs'deki draw_editor fonksiyonu bu flag'i okuyup dialog açar)
            editor_state.scene.request_save_dialog = true;
            editor_state.status_message = "💾 Kaydetme penceresi açılıyor...".to_string();
        }
    }

    // Ctrl+N → Yeni Sahne
    if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::KeyN as u32) {
        editor_state.scene.clear_request = true;
        editor_state.status_message = "🗑️ Sahne temizleniyor…".to_string();
    }

    // Ctrl+A → Tümünü Seç
    if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::KeyA as u32) {
        let names = world.query::<&gizmo::core::component::EntityName>();
        let markers = world.query::<&gizmo::core::component::EditorOnly>();
        let hidden = world.query::<&gizmo::core::component::IsHidden>();
        let deleted = world.query::<&gizmo::core::component::IsDeleted>();
        for e in world.iter_alive_entities() {
            // Editor entity'lerini ve gizli/silinmiş olanları atla
            if hidden.contains(e) || deleted.contains(e) {
                continue;
            }
            if gizmo::core::component::is_editor_only(
                markers.get(e).is_some(),
                names.get(e).map(|n| n.0.as_str()),
            ) {
                continue;
            }
            editor_state.selection.entities.insert(e);
        }
        // The inspector shows the primary, so a selection needs one: the lowest id, for want of a
        // last click. Ctrl+A left none, and the inspector showed nothing for a full selection.
        if editor_state.selection.primary.is_none_or(|p| !editor_state.selection.entities.contains(&p)) {
            editor_state.selection.primary = editor_state.selection.entities.iter().copied().min_by_key(|e| e.id());
        }
        let count = editor_state.selection.entities.len();
        editor_state.status_message = format!("✅ {} obje seçildi", count);
    }
}

/// Alt + a key: Alt+H shows every hidden object.
fn alt_shortcuts(world: &mut World, editor_state: &mut EditorState, input: &Input) {
    // Alt+H → Tüm gizli objeleri göster
    if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::KeyH as u32) {
        let hidden_ids: Vec<Entity> = {
            let deleted = world.query::<&gizmo::core::component::IsDeleted>();
            let names = world.query::<&gizmo::core::component::EntityName>();
            let markers = world.query::<&gizmo::core::component::EditorOnly>();
            world.query::<(Entity, &gizmo::core::component::IsHidden)>().iter()
                .filter(|(id, _)| {
                    // Editor entity'lerini gösterme, sadece kullanıcı entity'leri
                    if deleted.contains(*id) {
                        return false;
                    }
                    if gizmo::core::component::is_editor_only(
                        markers.get(*id).is_some(),
                        names.get(*id).map(|n| n.0.as_str()),
                    ) {
                        return false;
                    }
                    true
                })
                .map(|(entity, _)| entity)
                .collect()
        };

        let count = hidden_ids.len();
        for id in hidden_ids {
            if let Some(ent) = world.live(id) {
                world.remove_component::<gizmo::core::component::IsHidden>(ent);
            }
        }
        if count > 0 {
            editor_state.status_message = format!("👁 {} gizli obje gösterildi", count);
        }
    }
}

/// A key alone: delete, hide, the orthographic toggle, clear the selection, Shift+Space for play.
///
/// The tool keys (Q W E R) and F are the editor's (`gizmo_editor::draw_editor`, the scene view),
/// bound once there. They were bound here too: W, E and R a second time and F with a different
/// meaning, an instant jump to the selection's local positions at a reset distance, on the same
/// press as the scene view's smooth focus.
fn plain_shortcuts(world: &mut World, editor_state: &mut EditorState, state: &StudioState, input: &Input, shift_pressed: bool) {
        // Delete → Seçili objeleri sil
        if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::Delete as u32) {
            let count = editor_state.selection.entities.len();
            for &entity in editor_state.selection.entities.iter() {
                editor_state.despawn_requests.push(entity);
            }
            editor_state.clear_selection();
            if count > 0 {
                editor_state.status_message = format!("🗑️ {} obje silindi", count);
            }
        }

        // H → Seçili objeleri gizle
        if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::KeyH as u32) {
            let selected: Vec<gizmo::core::entity::Entity> =
                editor_state.selection.entities.iter().copied().collect();
            let count = selected.len();
            for entity in selected {
                editor_state.toggle_visibility_requests.push(entity);
            }
            editor_state.clear_selection();
            if count > 0 {
                editor_state.status_message = format!("👁‍🗨 {} obje gizlendi (Alt+H ile göster)", count);
            }
        }

        // Numpad 5 → Ortho/Perspektif geçişi (Blender tarzı)
        if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::Numpad5 as u32) {
            let mut cam_mut = world.query_mut::<&mut gizmo::renderer::components::Camera>();
            if let Some(mut cam) = cam_mut.get_mut(state.editor_camera) {
                cam.toggle_projection(editor_state.prefs.camera_focus_distance.max(1.0));
                let mode = if cam.projection.is_orthographic() { "Ortografik" } else { "Perspektif" };
                editor_state.status_message = format!("📐 Kamera: {} (Numpad5)", mode);
            }
        }

        // Escape → Seçimi temizle
        if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::Escape as u32)
            && !editor_state.selection.entities.is_empty() {
                editor_state.clear_selection();
                editor_state.status_message = "Seçim temizlendi".to_string();
            }

        // Space → Play/Stop geçiş
        if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::Space as u32)
            && shift_pressed {
                editor_state.toggle_play();
                let mode_str = if editor_state.is_playing() { "▶ Başladı" } else { "⏹ Durdu" };
                editor_state.status_message = format!("Simülasyon {}", mode_str);
            }
}


#[cfg(test)]
mod tests {
    use super::*;

    fn studio_state() -> StudioState {
        StudioState {
            current_fps: 0.0,
            actual_dt: 0.016,
            editor_camera: Entity::new(0, 0),
            game_camera: Entity::new(0, 0),
            do_raycast: false,
            play: gizmo::systems::PlayLoop::new(),
            asset_watcher: None,
            gc_timer: 0.0,
            autosave_timer: 0.0,
            visible_entity_count: 0,
            draw_call_count: 0,
            session: None,
            game: None,
        }
    }

    /// Ctrl+A selects the scene and gives the selection a primary — the entity the inspector shows.
    /// It left none, and the inspector showed nothing for a full selection.
    #[test]
    fn select_all_has_a_primary() {
        use gizmo::winit::keyboard::KeyCode;
        let mut world = World::new();
        let ids: Vec<Entity> = (0..3).map(|_| world.spawn()).collect();
        let mut ed = EditorState::default();
        let mut input = Input::default();
        input.on_key_pressed(KeyCode::ControlLeft as u32);
        input.on_key_pressed(KeyCode::KeyA as u32);
        handle_editor_shortcuts(&mut world, &mut ed, &studio_state(), &input);
        assert_eq!(ed.selection.entities.len(), 3);
        assert_eq!(ed.selection.primary, Some(ids[0]), "the lowest id, for want of a last click");
    }
}
