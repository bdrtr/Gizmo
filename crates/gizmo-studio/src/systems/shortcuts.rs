use crate::state::StudioState;
use gizmo::editor::{EditorState, GizmoMode};
use gizmo::physics::components::Transform;
use gizmo::prelude::*;

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

    // ========================================
    //  CTRL + X KOMBİNASYONLARI
    // ========================================
    if ctrl_pressed {
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
            let names = world.borrow::<gizmo::core::component::EntityName>();
            let hidden = world.borrow::<gizmo::core::component::IsHidden>();
            let deleted = world.borrow::<gizmo::core::component::IsDeleted>();
            for e in world.iter_alive_entities() {
                // Editor entity'lerini ve gizli/silinmiş olanları atla
                if hidden.contains(e.id()) || deleted.contains(e.id()) {
                    continue;
                }
                if let Some(name) = names.get(e.id()) {
                    if name.0.starts_with("Editor ") || name.0 == "Highlight Box" {
                        continue;
                    }
                }
                editor_state.selection.entities.insert(e);
            }
            let count = editor_state.selection.entities.len();
            editor_state.status_message = format!("✅ {} obje seçildi", count);
        }

        return; // Ctrl kombinasyonlarından sonra diğer tuşları kontrol etme
    }

    // ========================================
    //  ALT + X KOMBİNASYONLARI
    // ========================================
    if alt_pressed {
        // Alt+H → Tüm gizli objeleri göster
        if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::KeyH as u32) {
            let hidden_ids: Vec<u32> = {
                let hidden = world.borrow::<gizmo::core::component::IsHidden>();
                let deleted = world.borrow::<gizmo::core::component::IsDeleted>();
                let names = world.borrow::<gizmo::core::component::EntityName>();
                hidden
                    .iter()
                    .filter(|(id, _)| {
                        // Editor entity'lerini gösterme, sadece kullanıcı entity'leri
                        if deleted.contains(*id) {
                            return false;
                        }
                        if let Some(name) = names.get(*id) {
                            if name.0.starts_with("Editor ") || name.0 == "Highlight Box" {
                                return false;
                            }
                        }
                        true
                    })
                    .map(|(id, _)| id)
                    .collect()
            };

            let count = hidden_ids.len();
            for id in hidden_ids {
                if let Some(ent) = world.get_entity(id) {
                    world.remove_component::<gizmo::core::component::IsHidden>(ent);
                }
            }
            if count > 0 {
                editor_state.status_message = format!("👁 {} gizli obje gösterildi", count);
            }
        }

        return;
    }

    // ========================================
    //  TEKİL TUŞLAR (Ctrl/Alt/Shift olmadan)
    // ========================================

    // Sağ tık basılıyken kamera serbest uçuş modu aktif → gizmo modu değiştirme tuşlarını engelle
    let right_click_held = input.is_mouse_button_pressed(2);

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

    // W → Translate (Taşı) Gizmo (Sağ tık + WASD kamera uçuşu sırasında tetiklenmez)
    if !right_click_held && input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::KeyW as u32) {
        editor_state.gizmo_mode = GizmoMode::Translate;
        editor_state.status_message = "🔀 Taşıma Modu (W)".to_string();
    }

    // E → Rotate (Döndür) Gizmo
    if !right_click_held && input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::KeyE as u32) {
        editor_state.gizmo_mode = GizmoMode::Rotate;
        editor_state.status_message = "🔄 Döndürme Modu (E)".to_string();
    }

    // R → Scale (Ölçekle) Gizmo
    if !right_click_held && input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::KeyR as u32) {
        editor_state.gizmo_mode = GizmoMode::Scale;
        editor_state.status_message = "📏 Ölçekleme Modu (R)".to_string();
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

    // F → Seçili objeye odaklan (Focus)
    if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::KeyF as u32)
        && !editor_state.selection.entities.is_empty()
    {
        focus_on_selection(world, editor_state, state);
    }

    // Numpad 5 → Ortho/Perspektif Geçiş (henüz tam implement değil, ileride)
    // TODO: Camera component'ine ortho modu ekle
    // if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::Numpad5 as u32) { ... }

    // Escape → Seçimi temizle
    if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::Escape as u32) {
        if !editor_state.selection.entities.is_empty() {
            editor_state.clear_selection();
            editor_state.status_message = "Seçim temizlendi".to_string();
        }
    }

    // Space → Play/Stop geçiş
    if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::Space as u32) {
        if shift_pressed {
            editor_state.toggle_play();
            let mode_str = if editor_state.is_playing() { "▶ Başladı" } else { "⏹ Durdu" };
            editor_state.status_message = format!("Simülasyon {}", mode_str);
        }
    }
}

/// Kamerayı seçili objelerin merkezine odaklar
fn focus_on_selection(
    world: &mut World,
    editor_state: &mut EditorState,
    state: &StudioState,
) {
    let transforms = world.borrow::<Transform>();
    let mut center_pos = gizmo::math::Vec3::ZERO;
    let mut count = 0.0;
    for &target_id in editor_state.selection.entities.iter() {
        if let Some(target) = transforms.get(target_id.id()) {
            center_pos += target.position;
            count += 1.0;
        }
    }

    if count > 0.0 {
        let target_pos = center_pos / count;
        drop(transforms);

        let mut t_mut = world.borrow_mut::<Transform>();
        let mut cam_mut = world.borrow_mut::<gizmo::renderer::components::Camera>();
        if let (Some(cam_t), Some(cam)) = (
            t_mut.get_mut(state.editor_camera),
            cam_mut.get_mut(state.editor_camera),
        ) {
            editor_state.prefs.camera_focus_distance = 10.0;
            let offset = cam.get_front() * -editor_state.prefs.camera_focus_distance;
            cam_t.position = target_pos + offset;
            cam_t.update_local_matrix();
        }
        editor_state.status_message = "🎯 Seçime odaklanıldı (F)".to_string();
    }
}
