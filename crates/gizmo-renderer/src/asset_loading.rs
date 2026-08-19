use gizmo_core::World;
use gizmo_core::component::{MeshSource, MaterialSource};
use crate::components::{Mesh, Material, MeshRenderer};
use crate::asset::AssetManager;
use wgpu::Device;
use wgpu::Queue;
use wgpu::BindGroupLayout;

/// Continuously scans the scene for newly added `MeshSource` and `MaterialSource` components and
/// loads the `Mesh` and `Material` GPU components they are missing.
///
/// **A [`MaterialDesc`](crate::components::MaterialDesc) outranks a `MaterialSource`.** Both can
/// build a material and they do not carry the same thing: a source holds albedo, roughness,
/// metallic, an unlit flag and a texture path, while a description holds every field a `Material`
/// has. A scene row saved after descriptions existed carries both — the source because it was
/// authored before, the description because a save writes one for every live material — and if
/// this system won, the richer record would be discarded and then *overwritten* by the poorer one
/// on the next save. So an entity that has a description is left for
/// `material_sync::resolve_material_descriptions`; it still gets its `MeshRenderer` here, because
/// that pairing is this system's to guarantee.
#[tracing::instrument(skip_all, level = "trace")]
pub fn run_asset_loading_system(
    world: &mut World,
    device: &Device,
    queue: &Queue,
    texture_bind_group_layout: &BindGroupLayout,
    asset_manager: &mut AssetManager,
) {
    let mut missing_meshes = Vec::new();
    let mut missing_materials = Vec::new();
    // Entities whose material a DESCRIPTION will build (see this function's docs) but whose
    // `MeshRenderer` is still this system's to add.
    let mut renderer_only = Vec::new();

    // Hangi Entity'lerin Mesh/Material'ı eksik bul
    {
        let mesh_sources = world.borrow::<MeshSource>();
        let meshes = world.borrow::<Mesh>();

        let material_sources = world.borrow::<MaterialSource>();
        let materials = world.borrow::<Material>();
        let descs = world.borrow::<crate::components::MaterialDesc>();

        for e in world.iter_alive_entities() {
            let id = e.id();

            // MeshSource var ama GPU Mesh yok mu?
            if let Some(src) = mesh_sources.get(id) {
                if meshes.get(id).is_none() {
                    missing_meshes.push((id, src.0.clone()));
                }
            }

            // MaterialSource var ama GPU Material yok mu?
            if let Some(src) = material_sources.get(id) {
                if materials.get(id).is_none() {
                    if descs.get(id).is_some() {
                        renderer_only.push(id);
                    } else {
                        missing_materials.push((id, src.clone()));
                    }
                }
            }
        }
    }

    // Idle frames find nothing missing and stay silent; only log when there is
    // actual GPU upload work this frame (avoids per-frame noise on a steady scene).
    if !missing_meshes.is_empty() || !missing_materials.is_empty() {
        tracing::debug!(
            meshes = missing_meshes.len(),
            materials = missing_materials.len(),
            "[AssetLoading] uploading missing GPU components"
        );
    }

    // Default beyaz doku oluştur (Texture yüklenemezse veya yoksa kullanılır)
    let default_texture_bind_group = asset_manager.create_white_texture(device, queue, texture_bind_group_layout);

    // Eksik Mesh'leri yükle ve world'e ekle
    for (id, mesh_src) in missing_meshes {
        if let Some(entity) = world.get_entity(id) {
            let mesh = if mesh_src == "inverted_cube" {
                AssetManager::create_inverted_cube(device)
            } else if mesh_src == "plane" {
                AssetManager::create_plane(device, 200.0)
            } else if mesh_src == "standard_cube" {
                AssetManager::create_cube(device)
            } else if mesh_src == "sphere" {
                AssetManager::create_sphere(device, 1.0, 16, 16)
            } else if mesh_src == "sprite_quad" {
                AssetManager::create_sprite_quad(device, 1.0, 1.0)
            } else if mesh_src.starts_with("gltf_mesh_") {
                if let Some(cached) = asset_manager.get_cached_mesh(&mesh_src) {
                    cached
                } else {
                    // The one parse of this key shape in the workspace, shared with the scene
                    // format's identity repair — see `MeshSource::split_gltf_key`. Two copies of
                    // it is how the loader and the saver come to disagree about where the file
                    // name ends, which for a node called `wheel_front_left` is not hypothetical.
                    let file_path = gizmo_core::component::MeshSource::split_gltf_key(&mesh_src)
                        .map(|(path, _)| path);

                    if let Some(path) = file_path {
                        if let Err(e) = asset_manager.load_gltf_scene(
                            device,
                            queue,
                            texture_bind_group_layout,
                            default_texture_bind_group.clone(),
                            path,
                        ) {
                            // Previously swallowed with `let _`: on failure the mesh
                            // silently became a placeholder with no explanation.
                            tracing::warn!(
                                path,
                                mesh_source = %mesh_src,
                                error = %e,
                                "[AssetLoading] glTF scene load failed; using placeholder mesh"
                            );
                        }
                        if let Some(cached) = asset_manager.get_cached_mesh(&mesh_src) {
                            cached
                        } else {
                            asset_manager.loading_placeholder_mesh(device)
                        }
                    } else {
                        asset_manager.loading_placeholder_mesh(device)
                    }
                }
            } else if mesh_src.starts_with("obj:") {
                let path = mesh_src.trim_start_matches("obj:");
                asset_manager.load_obj(device, path)
            } else {
                asset_manager.load_obj(device, &mesh_src)
            };

            world.add_component(entity, mesh);
        }
    }

    // Eksik Material'ları yükle ve world'e ekle
    for (id, mat_data) in missing_materials {
        if let Some(entity) = world.get_entity(id) {
            let bind_group = if let Some(tex_path) = &mat_data.texture_source {
                asset_manager
                    .load_material_texture(device, queue, texture_bind_group_layout, tex_path)
                    .unwrap_or_else(|e| {
                        // Recoverable: falls back to the default white texture, but the
                        // requested texture is missing/undecodable — a real problem to surface.
                        tracing::warn!(
                            texture = %tex_path,
                            error = %e,
                            "[AssetLoading] material texture load failed; using default white texture"
                        );
                        default_texture_bind_group.clone()
                    })
            } else {
                default_texture_bind_group.clone()
            };

            let mut mat = Material::new(bind_group);
            mat.albedo = gizmo_math::Vec4::from(mat_data.albedo);
            mat.roughness = mat_data.roughness;
            mat.metallic = mat_data.metallic;
            mat.material_type = if mat_data.unlit > 1.5 {
                crate::components::MaterialType::Skybox
            } else if mat_data.unlit > 0.5 {
                crate::components::MaterialType::Unlit
            } else {
                crate::components::MaterialType::Pbr
            };
            mat.texture_source = mat_data.texture_source;

            world.add_component(entity, mat);
            // Her Material'ın yanında bir MeshRenderer olmalıdır (Render Pipeline için)
            world.add_component(entity, MeshRenderer::new());
        }
    }

    // The description-owned entities: their material arrives from `material_sync`, but the
    // `MeshRenderer` that has to sit beside it is still this system's guarantee. Added only when
    // missing, so an entity that already carries one keeps whatever flags it was given.
    if !renderer_only.is_empty() {
        let needed: Vec<u32> = {
            let renderers = world.borrow::<MeshRenderer>();
            renderer_only
                .into_iter()
                .filter(|id| renderers.get(*id).is_none())
                .collect()
        };
        for id in needed {
            if let Some(e) = world.get_entity(id) {
                world.add_component(e, MeshRenderer::new());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    /// Source with its comments removed — a `contains` over raw source is satisfied by the
    /// paragraph explaining the rule, which is exactly what this must not accept.
    fn code_only(src: &str) -> String {
        src.lines()
            .map(|line| {
                let bytes = line.as_bytes();
                let mut end = line.len();
                let mut i = 0;
                while i + 1 < bytes.len() {
                    if bytes[i] == b'/' && bytes[i + 1] == b'/' && (i == 0 || bytes[i - 1] != b':') {
                        end = i;
                        break;
                    }
                    i += 1;
                }
                &line[..end]
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// **This system must consult `MaterialDesc` before building a material from a source.**
    ///
    /// Two records can build one material and they do not carry the same thing: a `MaterialSource`
    /// holds albedo, roughness, metallic, an unlit flag and a texture path; a `MaterialDesc` holds
    /// every field a `Material` has. A scene row saved after descriptions existed carries both, and
    /// this system runs *before* the draw pass that resolves descriptions — so without the check a
    /// user's emissive, transparency, double-sidedness and the rest are rebuilt from the poorer
    /// record and then written back over the richer one by the next save.
    ///
    /// A source-shape guard because the behavioural version needs a GPU: building a `Material`
    /// means building a bind group. Comments are cut first.
    #[test]
    fn a_description_outranks_a_source_when_building_a_material() {
        let code: String = code_only(include_str!("asset_loading.rs"))
            .split("#[cfg(test)]")
            .next()
            .unwrap_or("")
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();

        assert!(
            code.contains("MaterialDesc"),
            "the asset-loading scan no longer looks at `MaterialDesc` — a scene authored after \
             2026-08-19 will have its material rebuilt from the older, poorer `MaterialSource` and \
             then overwritten by it on the next save"
        );
        assert!(
            code.contains("descs.get(id).is_some()"),
            "the check that defers to a description is gone; a `MaterialDesc` mentioned but not \
             consulted is the same defect with a comment on top"
        );
    }
}
