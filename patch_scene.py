import re

with open("crates/gizmo-scene/src/scene.rs", "r") as f:
    text = f.read()

target = """                } else if mesh_src.starts_with("gltf_mesh_") {
                    if let Some(cached) = asset_manager.get_cached_mesh(&mesh_src) {
                        cached
                    } else {
                        // Eğer GLTF RAM'de yoksa fail-safe oluştur (oyuncu GLTF'yi silmiş olabilir vs)
                        asset_manager.loading_placeholder_mesh(device)
                    }
                } else {"""

replacement = """                } else if mesh_src.starts_with("gltf_mesh_") {
                    if let Some(cached) = asset_manager.get_cached_mesh(&mesh_src) {
                        cached
                    } else {
                        // RAM'de yoksa path'i cikar ve parse edip Cache'e almayi dene
                        let file_path = if let Some(idx) = mesh_src.find(".glb") {
                            Some(&mesh_src["gltf_mesh_".len()..idx + 4])
                        } else if let Some(idx) = mesh_src.find(".gltf") {
                            Some(&mesh_src["gltf_mesh_".len()..idx + 5])
                        } else {
                            None
                        };

                        if let Some(path) = file_path {
                            let _ = asset_manager.load_gltf_scene(
                                device,
                                queue,
                                texture_bind_group_layout,
                                default_texture_bind_group.clone(),
                                path,
                            );
                            if let Some(cached) = asset_manager.get_cached_mesh(&mesh_src) {
                                cached
                            } else {
                                asset_manager.loading_placeholder_mesh(device)
                            }
                        } else {
                            asset_manager.loading_placeholder_mesh(device)
                        }
                    }
                } else {"""

if target in text:
    text = text.replace(target, replacement)
    with open("crates/gizmo-scene/src/scene.rs", "w") as f:
        f.write(text)
    print("Patched successfully")
else:
    print("Could not find target block")
