/// Dosya değişikliklerini izler ve texture'ları yeniden yükler.
use gizmo::prelude::*;
use crate::GameState;

pub fn poll_hot_reload(world: &mut World, state: &mut GameState) {
    let watcher = match &state.asset_watcher {
        Some(w) => w,
        None => return,
    };

    let changes = watcher.poll_changes();
    if changes.is_empty() { return; }

    for changed_path in &changes {
        let path_str = changed_path.to_string_lossy().to_string();
        let is_image = path_str.ends_with(".jpg")
            || path_str.ends_with(".png")
            || path_str.ends_with(".jpeg");
        if !is_image { continue; }

        println!("🔥 Hot-Reload: Texture değişti → {}", path_str);

        // Bu texture'ı kullanan tüm materyalleri bul
        if let Some(materials) = world.borrow::<Material>() {
            let mut targets = Vec::new();
            for &entity_id in &materials.entity_dense {
                if let Some(mat) = materials.get(entity_id) {
                    if let Some(src) = &mat.texture_source {
                        if changed_path.ends_with(src.as_str())
                            || src.contains(&path_str)
                            || path_str.contains(src.as_str())
                        {
                            targets.push(entity_id);
                        }
                    }
                }
            }
            drop(materials);
            for entity_id in targets {
                if let Some(mut events) = world.get_resource_mut::<gizmo::core::event::Events<crate::state::TextureLoadEvent>>() {
                    events.push(crate::state::TextureLoadEvent { entity_id, path: path_str.clone() });
                }
            }
        }
    }
}
