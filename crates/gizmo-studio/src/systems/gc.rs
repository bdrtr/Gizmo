//! The garbage-collection system — clears soft-deleted entities at a regular interval.
//!
//! Engines like Blender and Unity do not release GPU resources the moment an entity is deleted.
//! This system sweeps entities carrying the IsDeleted flag in a batch, after a 3-second delay,
//! which keeps undo safe and stops the memory from leaking.

use crate::state::StudioState;
use gizmo::editor::EditorState;
use gizmo::prelude::*;
use gizmo::core::HierarchyExt;

/// The garbage-collection interval, in seconds
const GC_INTERVAL: f32 = 3.0;

/// The auto-save interval, in seconds — every 5 minutes
const AUTOSAVE_INTERVAL: f32 = 300.0;

/// Clears soft-deleted entities and releases their GPU resources
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
                // BFS ile tüm torunları (cascade) bul — cycle-safe ve tekrarsız.
                // Unguarded until 2026-08-30: this walked an index cursor along a vector it
                // never drained, so a cycle anywhere under an `IsDeleted` entity grew `all_ids`
                // without bound until the process was killed.
                let all_ids = world.descendants_inclusive(id);

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
            // The result used to be dropped and the success line printed regardless, which is the
            // worst place in the editor to do that: a person watching "💾 Auto-Save" tick past
            // every interval concludes their work is safe. An unwritable path, a full disk or a
            // serialisation failure all produced the same reassuring line and no file.
            match gizmo::scene::SceneData::save(
                world,
                &autosave_path,
                &gizmo::full_scene_registry(),
            ) {
                Ok(_) => editor_state.log_info(&format!("💾 Auto-Save: {}", autosave_path)),
                Err(e) => editor_state.log_error(&format!(
                    "❌ Auto-Save BAŞARISIZ: {} — {}. Çalışmanız KAYDEDİLMEDİ.",
                    autosave_path, e
                )),
            }
        }
    } else {
        state.autosave_timer = 0.0; // Play modundayken veya sahne yolu boşken sıfırla
    }
}
