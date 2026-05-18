use gizmo_core::World;
use gizmo_core::component::{MeshSource, MaterialSource};
use crate::components::{Mesh, Material, MeshRenderer};
use crate::asset::AssetManager;
use wgpu::Device;
use wgpu::Queue;
use wgpu::BindGroupLayout;
use std::sync::Arc;

/// Sürekli olarak sahnede yeni eklenen `MeshSource` ve `MaterialSource` bileşenlerini
/// tarayarak, eksik olan `Mesh` ve `Material` GPU bileşenlerini yükler.
pub fn run_asset_loading_system(
    world: &mut World,
    device: &Device,
    queue: &Queue,
    texture_bind_group_layout: &BindGroupLayout,
    asset_manager: &mut AssetManager,
) {
    let mut missing_meshes = Vec::new();
    let mut missing_materials = Vec::new();

    // Hangi Entity'lerin Mesh/Material'ı eksik bul
    {
        let mesh_sources = world.borrow::<MeshSource>();
        let meshes = world.borrow::<Mesh>();

        let material_sources = world.borrow::<MaterialSource>();
        let materials = world.borrow::<Material>();

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
                    missing_materials.push((id, src.clone()));
                }
            }
        }
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
                    let file_path = if let Some(idx) = mesh_src.find(".glb") {
                        Some(&mesh_src["gltf_mesh_".len()..idx + 4])
                    } else {
                        mesh_src.find(".gltf").map(|idx| &mesh_src["gltf_mesh_".len()..idx + 5])
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
                        tracing::info!("Scene Texture error: {}", e);
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
}
