use serde::{Serialize, Deserialize};
use gizmo_core::{World, EntityName};
use gizmo_physics::components::{Transform, Velocity, RigidBody};
use gizmo_physics::shape::Collider;
use gizmo_renderer::components::{Camera, PointLight, Material, Mesh, MeshRenderer};
use gizmo_renderer::asset::AssetManager;
use gizmo_math::Vec4;
use std::fs;
use std::sync::Arc;

use std::collections::HashMap;
use gizmo_core::component::{Parent, Children};

/// Tam sahne verisi — tüm entity'ler ve bileşenleri
#[derive(Serialize, Deserialize, Clone)]
pub struct SceneData {
    pub entities: Vec<EntityData>,
}

/// Prefab verisi — Tıpkı SceneData gibi ama kök entity'si var
#[derive(Serialize, Deserialize, Clone)]
pub struct PrefabData {
    pub root_id: u32,
    pub entities: Vec<EntityData>,
}

/// Tek bir entity'nin serileştirilebilir verisi
#[derive(Serialize, Deserialize, Clone)]
pub struct EntityData {
    pub original_id: u32,
    pub name: Option<String>,
    pub transform: Option<Transform>,
    pub velocity: Option<Velocity>,
    pub rigid_body: Option<RigidBody>,
    pub collider: Option<Collider>,
    pub camera: Option<Camera>,
    pub point_light: Option<PointLight>,
    pub directional_light: Option<gizmo_renderer::components::DirectionalLight>,
    pub mesh_source: Option<String>,
    pub material_source: Option<MaterialData>,
    #[serde(default)]
    pub parent_id: Option<u32>,
    #[serde(default)]
    pub script_path: Option<String>,
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
        let entities_data = Self::serialize_entities(world, world.iter_alive_entities().into_iter().map(|e| e.id()).collect());

        let scene = SceneData { entities: entities_data };
        let json = serde_json::to_string_pretty(&scene).expect("Scene Serialize Hatası!");
        fs::write(file_path, json).expect("Sahne disk üzerine yazılamadı!");
        println!("✅ Sahne kaydedildi → {}", file_path);
    }

    /// Belirtilen entity ID'lerini serileştirir
    pub fn serialize_entities(world: &World, entity_ids: Vec<u32>) -> Vec<EntityData> {
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
        let parents = world.borrow::<Parent>();
        let dir_lights = world.borrow::<gizmo_renderer::components::DirectionalLight>();

        for &id in &entity_ids {
            let name = names.as_ref().and_then(|s| s.get(id)).map(|n| n.0.clone());
            let transform = transforms.as_ref().and_then(|s| s.get(id)).copied();
            let velocity = velocities.as_ref().and_then(|s| s.get(id)).copied();
            let rigid_body = rigidbodies.as_ref().and_then(|s| s.get(id)).copied();
            let collider = colliders.as_ref().and_then(|s| s.get(id)).cloned();
            let camera = cameras.as_ref().and_then(|s| s.get(id)).copied();
            let point_light = point_lights.as_ref().and_then(|s| s.get(id)).copied();
            let directional_light = dir_lights.as_ref().and_then(|s| s.get(id)).copied();
            let mesh_source = meshes.as_ref().and_then(|s| s.get(id)).map(|m| m.source.clone());
            let material_source = materials.as_ref().and_then(|s| s.get(id)).map(|m| MaterialData {
                albedo: m.albedo,
                roughness: m.roughness,
                metallic: m.metallic,
                unlit: m.unlit,
                texture_source: m.texture_source.clone(),
            });
            let parent_id = parents.as_ref().and_then(|s| s.get(id)).map(|p| p.0);
            let script_path = None; // TODO: Script bileşeni okuması eklenebilir

            if name.is_some() || transform.is_some() || velocity.is_some() || rigid_body.is_some() ||
               collider.is_some() || camera.is_some() || point_light.is_some() || directional_light.is_some() ||
               mesh_source.is_some() || material_source.is_some() {
                entities_data.push(EntityData {
                    original_id: id, name, transform, velocity, rigid_body, collider, camera, point_light, directional_light,
                    mesh_source, material_source, parent_id, script_path,
                });
            }
        }
        entities_data
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

        Self::instantiate_entities(
            scene.entities, None, world, device, queue, texture_bind_group_layout, asset_manager, &default_texture_bind_group
        );
        
        println!("✅ Sahne yüklendi ← {}", file_path);
        true
    }

    /// Verilen entity listesini instantiate eder, id eşleştirmelerini yapar ve gerekirse root bir parent'a bağlar
    pub fn instantiate_entities(
        entities: Vec<EntityData>,
        root_parent: Option<u32>,
        world: &mut World,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture_bind_group_layout: &wgpu::BindGroupLayout,
        asset_manager: &mut AssetManager,
        default_texture_bind_group: &Arc<wgpu::BindGroup>,
    ) -> HashMap<u32, u32> {
        let mut id_map = HashMap::new(); // original_id -> new_entity_id
        let mut entity_structs = HashMap::new();

        // Entity'leri oluştur ve id haritasını çıkar
        for data in &entities {
            let root_ent = world.spawn();
            id_map.insert(data.original_id, root_ent.id());
            entity_structs.insert(root_ent.id(), root_ent);
        }

        // Parent-Child ilişkilerini toplayacağımız geçici yapı (new_parent -> [new_children])
        let mut children_map: HashMap<u32, Vec<u32>> = HashMap::new();

        for data in entities {
            let new_id = id_map[&data.original_id];
            let entity = entity_structs[&new_id];
            
            if let Some(n) = data.name {
                world.add_component(entity, EntityName::new(&n));
            }
            if let Some(t) = data.transform { world.add_component(entity, t); }
            if let Some(v) = data.velocity { world.add_component(entity, v); }
            if let Some(r) = data.rigid_body { world.add_component(entity, r); }
            if let Some(c) = data.collider { world.add_component(entity, c); }
            if let Some(cam) = data.camera { world.add_component(entity, cam); }
            if let Some(pl) = data.point_light { world.add_component(entity, pl); }
            if let Some(dl) = data.directional_light { world.add_component(entity, dl); }
            
            if let Some(mesh_src) = data.mesh_source {
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
                } else if mesh_src.starts_with("obj:") {
                    let path = mesh_src.trim_start_matches("obj:");
                    asset_manager.load_obj(device, path)
                } else {
                    // Fail-safe obj loading
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

            // Hiyerarşi Bağlantıları
            let mut resolved_parent = None;
            if let Some(orig_parent) = data.parent_id {
                // Kendi içerisinde (bu sahnede/prefabda) kaydedilmiş bir parent varsa ona bağla
                if let Some(&p_id) = id_map.get(&orig_parent) {
                    resolved_parent = Some(p_id);
                }
            } else {
                // Eğer entity'nin kendi parent'ı yoksa, root_parent verilmişse ona tak
                resolved_parent = root_parent;
            }

            if let Some(p_id) = resolved_parent {
                world.add_component(entity, Parent(p_id));
                children_map.entry(p_id).or_insert_with(Vec::new).push(new_id);
            }
        }

        // Tüm Children bileşenlerini ilgili parent'lara ekle
        for (p_id, c_list) in children_map {
            if let Some(&p_ent) = entity_structs.get(&p_id) {
                // Not: Eğer önceden spawn edilmiş bir root_parent ise entity_structs'ta olmayacaktır.
                // Bu durumda onu ayrıca ele alıp, children pushlamamız gerek! (Şimdilik scene için geçerli basit yoldayız)
                world.add_component(p_ent, Children(c_list));
            } else if let Some(root_p_id) = root_parent {
                if root_p_id == p_id {
                    // Mevcut parent'ı sonradan alıp ekleyeceğiz
                    // Burada küçük bir hack ile halledilebilir ama şimdilik bırakıyoruz.
                }
            }
        }

        id_map
    }

    /// Prefab kaydet (Verilen entity ve tüm alt çocukları)
    pub fn save_prefab(world: &World, root_entity_id: u32, file_path: &str) {
        let mut entity_ids = vec![root_entity_id];
        
        // Tüm alt çocukları (recursive olarak) bul
        let mut i = 0;
        while i < entity_ids.len() {
            let e = entity_ids[i];
            if let Some(children) = world.borrow::<Children>().and_then(|c| c.get(e).cloned()) {
                entity_ids.extend(children.0);
            }
            i += 1;
        }

        let mut entities_data = Self::serialize_entities(world, entity_ids);
        
        // Prefab'ın root entity'sinin parent'ını kopar ki bağımsız yüklensin
        if let Some(root_data) = entities_data.iter_mut().find(|d| d.original_id == root_entity_id) {
            root_data.parent_id = None;
        }

        let prefab = PrefabData {
            root_id: root_entity_id,
            entities: entities_data,
        };

        let json = serde_json::to_string_pretty(&prefab).expect("Prefab Serialize Hatası!");
        fs::write(file_path, json).expect("Prefab diske yazılamadı!");
        println!("✅ Prefab kaydedildi → {}", file_path);
    }

    /// Prefab yükle
    pub fn load_prefab(
        file_path: &str,
        parent_entity: Option<u32>,
        world: &mut World,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture_bind_group_layout: &wgpu::BindGroupLayout,
        asset_manager: &mut AssetManager,
        default_texture_bind_group: Arc<wgpu::BindGroup>,
    ) -> Option<u32> {
        let json = match fs::read_to_string(file_path) {
            Ok(content) => content,
            Err(_) => return None,
        };

        let prefab: PrefabData = match serde_json::from_str(&json) {
            Ok(p) => p,
            Err(e) => {
                println!("❌ Prefab dosyası geçersiz ({}): {}", file_path, e);
                return None;
            }
        };

        let id_map = Self::instantiate_entities(
            prefab.entities, parent_entity, world, device, queue, texture_bind_group_layout, asset_manager, &default_texture_bind_group
        );

        let new_root_id = id_map.get(&prefab.root_id).copied();
        
        if let (Some(new_r), Some(p_id)) = (new_root_id, parent_entity) {
            // Root entity'i existing parent'a (daha önce Children var mı yok mu bakarak) ekle!
            let mut children_list = Vec::new();
            if let Some(existing_children) = world.borrow::<Children>().and_then(|c| c.get(p_id).cloned()) {
                children_list = existing_children.0;
            }
            children_list.push(new_r);
            world.add_component(gizmo_core::entity::Entity::new(p_id, 0), Children(children_list)); // Not strict generation safe but reasonable fallback
        }

        println!("✅ Prefab yüklendi ← {}", file_path);
        new_root_id
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
