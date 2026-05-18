use gizmo_core::{EntityName, World};
use gizmo_core::component::{MeshSource, MaterialSource};
use serde::{Deserialize, Serialize};
use std::fs;

use gizmo_core::component::{Children, Parent};
use std::collections::HashMap;

/// Tam sahne verisi — tüm entity'ler ve bileşenleri
#[derive(Serialize, Deserialize, Clone)]
pub struct SceneData {
    pub entities: Vec<EntityData>,
    #[serde(default)]
    pub joints: Vec<gizmo_physics_rigid::joints::Joint>,
}

/// Prefab verisi — Tıpkı SceneData gibi ama kök entity'si var
#[derive(Serialize, Deserialize, Clone)]
pub struct PrefabData {
    pub root_id: u32,
    pub entities: Vec<EntityData>,
    #[serde(default)]
    pub joints: Vec<gizmo_physics_rigid::joints::Joint>,
}

/// Tek bir entity'nin serileştirilebilir verisi
#[derive(Serialize, Deserialize, Clone)]
pub struct EntityData {
    pub original_id: u32,
    pub name: Option<String>,
    pub mesh_source: Option<String>,
    pub material_source: Option<MaterialData>,
    #[serde(default)]
    pub parent_id: Option<u32>,
    #[serde(default)]
    pub components: std::collections::BTreeMap<String, String>,
}

/// Material serileştirme verisi
#[derive(Serialize, Deserialize, Clone)]
pub struct MaterialData {
    pub albedo: [f32; 4],
    pub roughness: f32,
    pub metallic: f32,
    pub unlit: f32,
    pub texture_source: Option<String>,
}

impl SceneData {
    /// Mevcut World durumunu JSON dosyası olarak diske kaydeder
    pub fn save(
        world: &World,
        file_path: &str,
        registry: &crate::registry::SceneRegistry,
    ) -> Result<(), String> {
        if let Some(parent) = std::path::Path::new(file_path).parent() {
            let _ = fs::create_dir_all(parent);
        }
        let entities_data = Self::serialize_entities(
            world,
            world
                .iter_alive_entities()
                .into_iter()
                .map(|e| e.id())
                .collect(),
            registry,
        );

        let mut joints = Vec::new();
        if let Ok(physics_world) = world.try_get_resource::<gizmo_physics_rigid::world::PhysicsWorld>() {
            joints = physics_world.joints.clone();
        }

        let scene = SceneData {
            entities: entities_data,
            joints,
        };

        let string_data = ron::ser::to_string_pretty(&scene, ron::ser::PrettyConfig::default())
            .map_err(|e| format!("[SceneData::save] Serileştirme hatası: {}", e))?;

        fs::write(file_path, string_data)
            .map_err(|e| format!("[SceneData::save] Dosya yazma hatası: {}", e))?;

        tracing::info!("✅ Sahne kaydedildi → {}", file_path);
        Ok(())
    }

    /// Belirtilen entity ID'lerini serileştirir
    pub fn serialize_entities(
        world: &World,
        entity_ids: Vec<u32>,
        registry: &crate::registry::SceneRegistry,
    ) -> Vec<EntityData> {
        let mut entities_data = Vec::new();
        let names = world.borrow::<EntityName>();
        let meshes = world.borrow::<MeshSource>();
        let materials = world.borrow::<MaterialSource>();
        let parents = world.borrow::<Parent>();

        for &id in &entity_ids {
            let name = names.get(id).map(|n| n.0.clone());

            // Gizmo Studio'nun içsel araçlarını kaydetme
            if let Some(ref n) = name {
                if n.starts_with("Editor ") || n == "Highlight Box" {
                    continue;
                }
            }

            let mesh_source = meshes.get(id).map(|m| m.0.clone());
            let material_source = materials.get(id).map(|m| MaterialData {
                albedo: m.albedo,
                roughness: m.roughness,
                metallic: m.metallic,
                unlit: m.unlit,
                texture_source: m.texture_source.clone(),
            });
            let parent_id = parents.get(id).map(|p| p.0);

            let mut dynamic_components = std::collections::BTreeMap::new();
            for comp_name in registry.all_components() {
                if let Some(serializer) = registry.get_serializer(comp_name) {
                    if let Some(comp_value) = serializer(world, id) {
                        dynamic_components.insert(comp_name.clone(), comp_value);
                    }
                }
            }

            if name.is_some()
                || mesh_source.is_some()
                || material_source.is_some()
                || parent_id.is_some()
                || !dynamic_components.is_empty()
            {
                entities_data.push(EntityData {
                    original_id: id,
                    name,
                    mesh_source,
                    material_source,
                    parent_id,
                    components: dynamic_components,
                });
            }
        }
        
        tracing::info!(">>> serialize_entities: total entities checked: {}, serialized: {}", entity_ids.len(), entities_data.len());
        entities_data
    }

    /// JSON sahne dosyasını okuyup World'e entity olarak yükler
    pub fn load_into(
        file_path: &str,
        world: &mut World,
        registry: &crate::registry::SceneRegistry,
    ) -> bool {
        let string_data = match fs::read_to_string(file_path) {
            Ok(content) => content,
            Err(_) => return false,
        };

        let scene: SceneData = match ron::from_str(&string_data) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("❌ Sahne dosyası geçersiz ({}): {}", file_path, e);
                return false;
            }
        };

        let entities = scene.entities;
        tracing::info!(">>> load_into: Read {} entities from file", entities.len());

        let id_map = Self::instantiate_entities(
            entities,
            None,
            world,
            registry,
        );

        if let Ok(mut physics_world) = world.try_get_resource_mut::<gizmo_physics_rigid::world::PhysicsWorld>() {
            for mut joint in scene.joints {
                if let (Some(&new_a), Some(&new_b)) = (id_map.get(&joint.entity_a.id()), id_map.get(&joint.entity_b.id())) {
                    joint.entity_a = gizmo_core::entity::Entity::new(new_a, 0);
                    joint.entity_b = gizmo_core::entity::Entity::new(new_b, 0);
                    physics_world.joints.push(joint);
                }
            }
        }

        tracing::info!("✅ Sahne yüklendi ← {}", file_path);
        true
    }

    /// Verilen entity listesini instantiate eder
    pub fn instantiate_entities(
        entities: Vec<EntityData>,
        root_parent: Option<u32>,
        world: &mut World,
        registry: &crate::registry::SceneRegistry,
    ) -> HashMap<u32, u32> {
        let mut id_map = HashMap::new(); 
        let mut entity_structs = HashMap::new();

        for data in &entities {
            let root_ent = world.spawn();
            id_map.insert(data.original_id, root_ent.id());
            entity_structs.insert(root_ent.id(), root_ent);
        }

        let mut children_map: HashMap<u32, Vec<u32>> = HashMap::new();

        for data in entities {
            let new_id = id_map[&data.original_id];
            let entity = entity_structs[&new_id];

            if let Some(n) = data.name {
                world.add_component(entity, EntityName::new(&n));
            }
            
            for (comp_name, comp_val) in &data.components {
                if let Some(deserializer) = registry.get_deserializer(comp_name) {
                    deserializer(world, new_id, comp_val);
                }
            }

            if let Some(mesh_src) = data.mesh_source {
                world.add_component(entity, MeshSource(mesh_src));
            }

            if let Some(mat_data) = data.material_source {
                world.add_component(entity, MaterialSource {
                    albedo: mat_data.albedo,
                    roughness: mat_data.roughness,
                    metallic: mat_data.metallic,
                    unlit: mat_data.unlit,
                    texture_source: mat_data.texture_source,
                });
            }

            let mut resolved_parent = None;
            if let Some(orig_parent) = data.parent_id {
                if let Some(&p_id) = id_map.get(&orig_parent) {
                    resolved_parent = Some(p_id);
                }
            } else {
                resolved_parent = root_parent;
            }

            if let Some(p_id) = resolved_parent {
                world.add_component(entity, Parent(p_id));
                children_map.entry(p_id).or_default().push(new_id);
            }
        }

        for (p_id, mut c_list) in children_map {
            if let Some(&p_ent) = entity_structs.get(&p_id) {
                world.add_component(p_ent, Children(c_list));
            } else if let Some(p_ent) = world.get_entity(p_id) {
                let existing: Vec<u32> = world
                    .borrow::<Children>()
                    .get(p_id)
                    .map(|c| c.0.clone())
                    .unwrap_or_default();
                let mut merged = existing;
                merged.append(&mut c_list);
                world.add_component(p_ent, Children(merged));
            }
        }

        id_map
    }

    /// Prefab kaydet
    pub fn save_prefab(
        world: &World,
        root_entity_id: u32,
        file_path: &str,
        registry: &crate::registry::SceneRegistry,
    ) -> Result<(), String> {
        if let Some(parent) = std::path::Path::new(file_path).parent() {
            let _ = fs::create_dir_all(parent);
        }

        let mut ids_to_save = vec![root_entity_id];
        let children_storage = world.borrow::<Children>();

        let mut i = 0;
        while i < ids_to_save.len() {
            let current = ids_to_save[i];
            if let Some(children_comp) = children_storage.get(current) {
                for &child_id in &children_comp.0 {
                    ids_to_save.push(child_id);
                }
            }
            i += 1;
        }

        let mut entities_data = Self::serialize_entities(world, ids_to_save.clone(), registry);

        if let Some(root_data) = entities_data
            .iter_mut()
            .find(|d| d.original_id == root_entity_id)
        {
            root_data.parent_id = None;
        }

        let mut joints = Vec::new();
        if let Ok(physics_world) = world.try_get_resource::<gizmo_physics_rigid::world::PhysicsWorld>() {
            for joint in &physics_world.joints {
                if ids_to_save.contains(&joint.entity_a.id()) && ids_to_save.contains(&joint.entity_b.id()) {
                    joints.push(joint.clone());
                }
            }
        }

        let prefab = PrefabData {
            root_id: root_entity_id,
            entities: entities_data,
            joints,
        };

        let string_data = ron::ser::to_string_pretty(&prefab, ron::ser::PrettyConfig::default())
            .map_err(|e| format!("[SceneData::save_prefab] Serileştirme hatası: {}", e))?;

        fs::write(file_path, string_data)
            .map_err(|e| format!("[SceneData::save_prefab] Dosya yazma hatası: {}", e))?;

        tracing::info!("✅ Prefab kaydedildi → {}", file_path);
        Ok(())
    }

    /// Prefab yükle
    pub fn load_prefab(
        file_path: &str,
        parent_entity: Option<u32>,
        world: &mut World,
        registry: &crate::registry::SceneRegistry,
    ) -> Option<u32> {
        let string_data = match fs::read_to_string(file_path) {
            Ok(content) => content,
            Err(_) => return None,
        };

        let prefab: PrefabData = match ron::from_str(&string_data) {
            Ok(p) => p,
            Err(e) => {
                tracing::info!("❌ Prefab dosyası geçersiz ({}): {}", file_path, e);
                return None;
            }
        };

        let id_map = Self::instantiate_entities(
            prefab.entities,
            parent_entity,
            world,
            registry,
        );

        let new_root_id = id_map.get(&prefab.root_id).copied();

        if let (Some(new_r), Some(p_id)) = (new_root_id, parent_entity) {
            if let Some(p_ent) = world.get_entity(p_id) {
                let mut children_list = world
                    .borrow::<Children>()
                    .get(p_id)
                    .map(|c| c.0.clone())
                    .unwrap_or_default();
                children_list.push(new_r);
                world.add_component(p_ent, Children(children_list));
            }
        }

        if let Ok(mut physics_world) = world.try_get_resource_mut::<gizmo_physics_rigid::world::PhysicsWorld>() {
            for mut joint in prefab.joints {
                if let (Some(&new_a), Some(&new_b)) = (id_map.get(&joint.entity_a.id()), id_map.get(&joint.entity_b.id())) {
                    joint.entity_a = gizmo_core::entity::Entity::new(new_a, 0);
                    joint.entity_b = gizmo_core::entity::Entity::new(new_b, 0);
                    physics_world.joints.push(joint);
                }
            }
        }

        tracing::info!("✅ Prefab yüklendi ← {}", file_path);
        new_root_id
    }

    /// Entity listesini döndürür (Lua API'si için)
    pub fn get_entity_names(world: &World) -> Vec<(u32, String)> {
        let mut result = Vec::new();
        let names = world.borrow::<EntityName>();
        for (entity_id, _) in names.iter() {
            if let Some(name) = names.get(entity_id) {
                result.push((entity_id, name.0.clone()));
            }
        }
        result
    }

    /// İsme göre entity bul
    pub fn find_entity_by_name(world: &World, target_name: &str) -> Option<u32> {
        let names = world.borrow::<EntityName>();
        for (entity_id, _) in names.iter() {
            if let Some(name) = names.get(entity_id) {
                if name.0 == target_name {
                    return Some(entity_id);
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_core::World;
    use gizmo_physics_rigid::joints::data::{Joint, JointType};

    #[test]
    fn test_prefab_joint_serialization() {
        let mut world = World::new();
        let ent1 = world.spawn();
        let ent2 = world.spawn();

        let joint = Joint {
            entity_a: ent1,
            entity_b: ent2,
            local_anchor_a: gizmo_math::Vec3::ZERO,
            local_anchor_b: gizmo_math::Vec3::ZERO,
            break_force: 1000.0,
            break_torque: 1000.0,
            is_broken: false,
            collision_enabled: false,
            data: gizmo_physics_rigid::joints::data::JointData::Fixed,
        };

        let prefab_data = PrefabData {
            root_id: ent1.id(),
            entities: vec![],
            joints: vec![joint.clone()],
        };

        let serialized = ron::ser::to_string(&prefab_data).unwrap();
        assert!(serialized.contains("Fixed"));

        let deserialized: PrefabData = ron::from_str(&serialized).unwrap();
        assert_eq!(deserialized.joints.len(), 1);
        assert!(matches!(deserialized.joints[0].data, gizmo_physics_rigid::joints::data::JointData::Fixed));
    }
}
