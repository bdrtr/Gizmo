//! Garbage Collection System — Soft-deleted entity'leri düzenli aralıklarla temizler
//!
//! Blender/Unity gibi motorlar entity silindiğinde hemen GPU kaynakları bırakmaz.
//! Bu sistem, IsDeleted bayrağı taşıyan entity'leri 3 saniyelik bir gecikmeyle
//! topluca temizleyerek hem Undo güvenliğini korur hem de RAM sızıntısını önler.

use crate::state::StudioState;
use gizmo::editor::EditorState;
use gizmo::prelude::*;

/// Garbage collection aralığı (saniye)
const GC_INTERVAL: f32 = 3.0;

/// Auto-save aralığı (saniye) — 5 dakikada bir
const AUTOSAVE_INTERVAL: f32 = 300.0;

/// Soft-deleted entity'leri temizler ve GPU kaynaklarını serbest bırakır
pub fn garbage_collection_system(
    world: &mut World,
    state: &mut StudioState,
    editor_state: &mut EditorState,
    dt: f32,
) {
    // --- GARBAGE COLLECTION ---
    state.gc_timer += dt;
    if state.gc_timer >= GC_INTERVAL {
        state.gc_timer = 0.0;

        // Soft-deleted entity'leri topla
        let mut to_despawn = Vec::new();
        {
            let deleted = world.borrow::<gizmo::core::component::IsDeleted>();
            for (id, _) in deleted.iter() {
                to_despawn.push(id);
            }
        }

        if !to_despawn.is_empty() {
            let count = to_despawn.len();
            for id in to_despawn {
                // BFS ile tüm torunları (cascade) bul
                let mut all_ids = vec![id];
                {
                    let children = world.borrow::<gizmo::core::component::Children>();
                    let mut i = 0;
                    while i < all_ids.len() {
                        let current = all_ids[i];
                        if let Some(c) = children.get(current) {
                            for &child_id in &c.0 {
                                all_ids.push(child_id);
                            }
                        }
                        i += 1;
                    }
                }

                // Tüm torunları ve kendisini sil (ters sıra — yapraklardan başla)
                for &del_id in all_ids.iter().rev() {
                    if let Some(ent) = world.get_entity(del_id) {
                        world.despawn(ent);
                    }
                }
            }

            // GPU Memory GC
            let mut freed_gpu = 0;
            if let Ok(mut asset_manager) = world.try_get_resource_mut::<AssetManager>() {
                freed_gpu = asset_manager.garbage_collect();
            }

            // RAM Memory Defragmentation
            world.compact();

            editor_state.log_info(&format!(
                "♻ GC: {} soft-deleted entity ve {} GPU asset'i temizlendi (RAM/VRAM serbest bırakıldı).",
                count, freed_gpu
            ));
        }
    }

    // --- AUTO-SAVE ---
    if editor_state.is_editing() && !editor_state.scene_path.is_empty() {
        state.autosave_timer += dt;
        if state.autosave_timer >= AUTOSAVE_INTERVAL {
            state.autosave_timer = 0.0;

            let autosave_path = format!("{}.autosave", editor_state.scene_path);
            let _ = gizmo::scene::SceneData::save(
                world,
                &autosave_path,
                &gizmo::scene::SceneRegistry::default(),
            );
            editor_state.log_info(&format!("💾 Auto-Save: {}", autosave_path));
        }
    } else {
        state.autosave_timer = 0.0; // Play modundayken veya sahne yolu boşken sıfırla
    }
}
