use serde::{Serialize, Deserialize};
use gizmo_core::{World, EntityName};
use gizmo_physics::components::{Transform, Velocity, RigidBody};
use gizmo_physics::shape::Collider;
use gizmo_renderer::components::{Camera, PointLight, Material, Mesh, MeshRenderer};
use gizmo_renderer::asset::AssetManager;
use gizmo_math::Vec4;
use std::fs;
use std::sync::Arc;

/// Tam sahne verisi — tüm entity'ler ve bileşenleri
#[derive(Serialize, Deserialize)]
pub struct SceneData {
    pub entities: Vec<EntityData>,
}

/// Tek bir entity'nin serileştirilebilir verisi
#[derive(Serialize, Deserialize)]
pub struct EntityData {
    pub name: Option<String>,
    pub transform: Option<Transform>,
    pub velocity: Option<Velocity>,
    pub rigid_body: Option<RigidBody>,
    pub collider: Option<Collider>,
    pub camera: Option<Camera>,
    pub point_light: Option<PointLight>,
    pub mesh_source: Option<String>,
    pub material_source: Option<MaterialData>,
    #[serde(default)]
    pub parent_index: Option<usize>,  // Sahne dosyası içindeki parent entity'nin indeksi
    #[serde(default)]
    pub script_path: Option<String>,  // Lua script dosya yolu
}

/// Material serileştirme verisi (GPU bind group'u diske yazılamaz)
#[derive(Serialize, Deserialize, Clone)]
pub struct MaterialData {
    pub albedo: Vec4,
    pub roughness: f32,
    pub metallic: f32,
    pub unlit: f32,
    pub texture_source: Option<String>,
}

impl SceneData {
    /// Mevcut World durumunu JSON dosyası olarak diske kaydeder
    pub fn save(world: &World, file_path: &str) {
        let mut entities_data = Vec::new();
        
        let names = world.borrow::<EntityName>();
        let transforms = world.borrow::<Transform>();
        let velocities = world.borrow::<Velocity>();
        let rigidbodies = world.borrow::<RigidBody>();
        let colliders = world.borrow::<Collider>();
        let cameras = world.borrow::<Camera>();
        let point_lights = world.borrow::<PointLight>();
        let meshes = world.borrow::<Mesh>();
        let materials = world.borrow::<Material>();

        for entity in world.iter_alive_entities() {
            let id = entity.id();
            let name = names.as_ref().and_then(|s| s.get(id)).map(|n| n.0.clone());
            let transform = transforms.as_ref().and_then(|s| s.get(id)).copied();
            let velocity = velocities.as_ref().and_then(|s| s.get(id)).copied();
            let rigid_body = rigidbodies.as_ref().and_then(|s| s.get(id)).copied();
            let collider = colliders.as_ref().and_then(|s| s.get(id)).cloned();
            let camera = cameras.as_ref().and_then(|s| s.get(id)).copied();
            let point_light = point_lights.as_ref().and_then(|s| s.get(id)).copied();
            let mesh_source = meshes.as_ref().and_then(|s| s.get(id)).map(|m| m.source.clone());
            let material_source = materials.as_ref().and_then(|s| s.get(id)).map(|m| MaterialData {
                albedo: m.albedo,
                roughness: m.roughness,
                metallic: m.metallic,
                unlit: m.unlit,
                texture_source: m.texture_source.clone(),
            });

            // Lua script bileşenini de kaydet
            let script_path = None; // Script component'i eklendiğinde burası güncellenecek

            if name.is_some() || transform.is_some() || velocity.is_some() || rigid_body.is_some() ||
               collider.is_some() || camera.is_some() || point_light.is_some() ||
               mesh_source.is_some() || material_source.is_some() {
                entities_data.push(EntityData {
                    name, transform, velocity, rigid_body, collider, camera, point_light, 
                    mesh_source, material_source, parent_index: None, script_path,
                });
            }
        }

        let scene = SceneData { entities: entities_data };
        let json = serde_json::to_string_pretty(&scene).expect("Scene Serialize Hatası!");
        fs::write(file_path, json).expect("Sahne disk üzerine yazılamadı!");
        println!("✅ Sahne kaydedildi → {}", file_path);
    }

    /// JSON sahne dosyasını okuyup World'e entity olarak yükler
    pub fn load_into(
        file_path: &str,
        world: &mut World,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture_bind_group_layout: &wgpu::BindGroupLayout,
        asset_manager: &mut AssetManager,
        default_texture_bind_group: Arc<wgpu::BindGroup>,
    ) -> bool {
        let json = match fs::read_to_string(file_path) {
            Ok(content) => content,
            Err(_) => return false,
        };

        let scene: SceneData = match serde_json::from_str(&json) {
            Ok(s) => s,
            Err(e) => {
                println!("❌ Sahne dosyası geçersiz ({}): {}", file_path, e);
                return false;
            }
        };

        for data in scene.entities {
            let entity = world.spawn();
            
            if let Some(n) = data.name {
                world.add_component(entity, EntityName::new(&n));
            }
            if let Some(t) = data.transform { world.add_component(entity, t); }
            if let Some(v) = data.velocity { world.add_component(entity, v); }
            if let Some(r) = data.rigid_body { world.add_component(entity, r); }
            if let Some(c) = data.collider { world.add_component(entity, c); }
            if let Some(cam) = data.camera { world.add_component(entity, cam); }
            if let Some(pl) = data.point_light { world.add_component(entity, pl); }
            
            if let Some(mesh_src) = data.mesh_source {
                let mesh = if mesh_src == "inverted_cube" {
                    AssetManager::create_inverted_cube(device)
                } else if mesh_src == "plane" {
                    AssetManager::create_plane(device, 50.0)
                } else if mesh_src.starts_with("obj:") {
                    let path = mesh_src.trim_start_matches("obj:");
                    asset_manager.load_obj(device, path)
                } else {
                    asset_manager.load_obj(device, &mesh_src)
                };
                world.add_component(entity, mesh);
            }

            if let Some(mat_data) = data.material_source {
                let bind_group = if let Some(tex_path) = &mat_data.texture_source {
                    asset_manager.load_material_texture(device, queue, texture_bind_group_layout, tex_path)
                        .unwrap_or_else(|e| {
                            println!("Scene Texture error: {}", e);
                            default_texture_bind_group.clone()
                        })
                } else {
                    default_texture_bind_group.clone()
                };
                
                let mut mat = Material::new(bind_group);
                mat.albedo = mat_data.albedo;
                mat.roughness = mat_data.roughness;
                mat.metallic = mat_data.metallic;
                mat.unlit = mat_data.unlit;
                mat.texture_source = mat_data.texture_source;
                world.add_component(entity, mat);
                world.add_component(entity, MeshRenderer::new());
            }
        }
        
        println!("✅ Sahne yüklendi ← {}", file_path);
        true
    }

    /// Entity listesini döndürür (Lua API'si için)
    pub fn get_entity_names(world: &World) -> Vec<(u32, String)> {
        let mut result = Vec::new();
        if let Some(names) = world.borrow::<EntityName>() {
            for &entity_id in &names.entity_dense {
                if let Some(name) = names.get(entity_id) {
                    result.push((entity_id, name.0.clone()));
                }
            }
        }
        result
    }

    /// İsme göre entity bul
    pub fn find_entity_by_name(world: &World, target_name: &str) -> Option<u32> {
        if let Some(names) = world.borrow::<EntityName>() {
            for &entity_id in &names.entity_dense {
                if let Some(name) = names.get(entity_id) {
                    if name.0 == target_name {
                        return Some(entity_id);
                    }
                }
            }
        }
        None
    }
}
