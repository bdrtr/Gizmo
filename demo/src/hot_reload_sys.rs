use crate::GameState;
/// Dosya değişikliklerini izler ve texture'ları yeniden yükler.
use gizmo::prelude::*;

pub fn poll_hot_reload(world: &mut World, state: &mut GameState) {
    let watcher = match &state.asset_watcher {
        Some(w) => w,
        None => return,
    };

    let changes = watcher.poll_changes();
    if changes.is_empty() {
        return;
    }

    for changed_path in &changes {
        let path_str = changed_path.to_string_lossy().to_string();
        let is_image =
            path_str.ends_with(".jpg") || path_str.ends_with(".png") || path_str.ends_with(".jpeg");

        let is_script = path_str.ends_with(".lua");
        let is_shader = path_str.ends_with(".wgsl");

        if is_script {
            println!("🔥 Script Hot-Reload: {}", path_str);
            if let Some(mut engine) = world.get_resource_mut::<gizmo::scripting::ScriptEngine>() {
                if let Err(e) = engine.load_script(&path_str) {
                    println!("    ❌ Script yüklenemedi: {}", e);
                }
            }
            continue;
        }

        if is_shader {
            println!("🔥 Shader Hot-Reload: {}", path_str);
            let has_events = world
                .get_resource::<gizmo::core::event::Events<crate::state::ShaderReloadEvent>>()
                .is_some();
            if !has_events {
                world.insert_resource(
                    gizmo::core::event::Events::<crate::state::ShaderReloadEvent>::new(),
                );
            }
            if let Some(mut events) = world
                .get_resource_mut::<gizmo::core::event::Events<crate::state::ShaderReloadEvent>>()
            {
                events.push(crate::state::ShaderReloadEvent);
            }
            continue;
        }

        if !is_image {
            continue;
        }

        println!("🔥 Hot-Reload: Texture değişti → {}", path_str);

        // Bu texture'ı kullanan tüm materyalleri bul
        if let Some(materials) = world.borrow::<Material>() {
            let mut targets = Vec::new();
            for entity_id in materials.dense.iter().map(|e| e.entity) {
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
                if let Some(mut events) = world
                    .get_resource_mut::<gizmo::core::event::Events<crate::state::TextureLoadEvent>>(
                    )
                {
                    events.push(crate::state::TextureLoadEvent {
                        entity_id,
                        path: path_str.clone(),
                    });
                }
            }
        }
    }
}
