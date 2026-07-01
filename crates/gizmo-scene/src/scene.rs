use crate::error::SceneError;
use gizmo_core::{EntityName, World};
use gizmo_core::component::{MeshSource, MaterialSource};
use serde::{Deserialize, Serialize};
use std::fs;

use gizmo_core::component::{Children, Parent};
use std::collections::HashMap;

/// Full scene data — all entities together with their components.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[non_exhaustive]
pub struct SceneData {
    pub entities: Vec<EntityData>,
    #[serde(default)]
    pub joints: Vec<gizmo_physics_rigid::joints::Joint>,
}

/// Prefab data — like [`SceneData`] but anchored to a root entity.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[non_exhaustive]
pub struct PrefabData {
    pub root_id: u32,
    pub entities: Vec<EntityData>,
    #[serde(default)]
    pub joints: Vec<gizmo_physics_rigid::joints::Joint>,
}

/// Serializable data for a single entity.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
#[non_exhaustive]
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

/// Serializable material data.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
#[non_exhaustive]
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
        registry: &gizmo_core::registry::ComponentRegistry,
    ) -> Result<(), SceneError> {
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

        let string_data = ron::ser::to_string_pretty(&scene, ron::ser::PrettyConfig::default())?;

        fs::write(file_path, string_data)?;

        tracing::info!("✅ Sahne kaydedildi → {}", file_path);
        Ok(())
    }

    pub fn serialize_entities(
        world: &World,
        entity_ids: Vec<u32>,
        registry: &gizmo_core::registry::ComponentRegistry,
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

            // Entity'nin GERÇEK generation'ı ile lookup yap. Sabit `Entity::new(id, 0)`,
            // id slotu yeniden kullanılmış (despawn→spawn, generation ≥ 1) entity'lerde
            // `entity_component_types`'ın `is_alive` generation kontrolünü geçemez →
            // dinamik bileşenler (Transform, RigidBody, Collider…) diske kaydedilirken
            // sessizce kaybolurdu. Name/Mesh/Material/Parent raw-id ile okunduğu için
            // etkilenmez.
            if let Some(entity) = world.entity(id) {
                let types = world.entity_component_types(entity);
                for type_id in types {
                    if let Some(reg) = registry.get_registration(type_id) {
                        if let Some(ptr) = world.get_component_ptr(entity, type_id) {
                            if let Some(string_repr) =
                                crate::serde_bridge::serialize_component(registry, reg, type_id, ptr)
                            {
                                dynamic_components.insert(reg.name.clone(), string_repr);
                            }
                        }
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
        registry: &gizmo_core::registry::ComponentRegistry,
    ) -> Result<(), SceneError> {
        let string_data = fs::read_to_string(file_path)?;

        let scene: SceneData = ron::from_str(&string_data)?;

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
                    joint.entity_a = gizmo_physics_rigid::BodyHandle::from_id(new_a);
                    joint.entity_b = gizmo_physics_rigid::BodyHandle::from_id(new_b);
                    physics_world.joints.push(joint);
                }
            }
        }

        tracing::info!("✅ Sahne yüklendi ← {}", file_path);
        Ok(())
    }

    pub fn instantiate_entities(
        entities: Vec<EntityData>,
        root_parent: Option<u32>,
        world: &mut World,
        registry: &gizmo_core::registry::ComponentRegistry,
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
                if let Some(type_id) = registry.get_type_id(comp_name) {
                    if let Some(reg) = registry.get_registration(type_id) {
                        if let Err(e) = crate::serde_bridge::deserialize_component(
                            world, entity, registry, reg, type_id, comp_val,
                        ) {
                            tracing::error!("Failed to deserialize component {}: {}", comp_name, e);
                        }
                    }
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
        registry: &gizmo_core::registry::ComponentRegistry,
    ) -> Result<(), SceneError> {
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

        let string_data = ron::ser::to_string_pretty(&prefab, ron::ser::PrettyConfig::default())?;

        fs::write(file_path, string_data)?;

        tracing::info!("✅ Prefab kaydedildi → {}", file_path);
        Ok(())
    }

    /// Prefab yükle
    pub fn load_prefab(
        file_path: &str,
        parent_entity: Option<u32>,
        world: &mut World,
        registry: &gizmo_core::registry::ComponentRegistry,
    ) -> Result<Option<u32>, SceneError> {
        let string_data = fs::read_to_string(file_path)?;

        let prefab: PrefabData = ron::from_str(&string_data)?;

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
                // Idempotent: `instantiate_entities` may already have linked the root into
                // the parent's Children when it set the root's Parent(p_id). Pushing again
                // duplicated the child. Only add if not already present.
                if !children_list.contains(&new_r) {
                    children_list.push(new_r);
                    world.add_component(p_ent, Children(children_list));
                }
            }
        }

        if let Ok(mut physics_world) = world.try_get_resource_mut::<gizmo_physics_rigid::world::PhysicsWorld>() {
            for mut joint in prefab.joints {
                if let (Some(&new_a), Some(&new_b)) = (id_map.get(&joint.entity_a.id()), id_map.get(&joint.entity_b.id())) {
                    joint.entity_a = gizmo_physics_rigid::BodyHandle::from_id(new_a);
                    joint.entity_b = gizmo_physics_rigid::BodyHandle::from_id(new_b);
                    physics_world.joints.push(joint);
                }
            }
        }

        tracing::info!("✅ Prefab yüklendi ← {}", file_path);
        Ok(new_root_id)
    }

    /// Entity listesini döndürür (Lua API'si için)
    pub fn entity_names(world: &World) -> Vec<(u32, String)> {
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
    use gizmo_physics_rigid::joints::data::Joint;

    #[test]
    fn test_prefab_joint_serialization() {
        let mut world = World::new();
        let ent1 = world.spawn();
        let ent2 = world.spawn();

        let joint = Joint::fixed(
            gizmo_physics_rigid::BodyHandle::from_id(ent1.id()),
            gizmo_physics_rigid::BodyHandle::from_id(ent2.id()),
            gizmo_math::Vec3::ZERO,
            gizmo_math::Vec3::ZERO,
        )
        .with_break_force(1000.0, 1000.0);

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

    // Faz 5 — sahne kaydet/yükle GÜVENİLİRLİĞİ: bir dünya kaydedilip TAZE bir dünyaya
    // yüklendiğinde isim + bileşen değerleri + hiyerarşi KORUNMALI (round-trip sadakati).
    #[test]
    fn scene_save_load_roundtrip_preserves_components_and_hierarchy() {
        use gizmo_physics_core::components::Transform;
        use gizmo_math::Vec3;

        // Gerçek registry ile (reflect AÇIK ise reflect serileştirmesi, KAPALI ise serde
        // fallback — her iki yolda da Transform değerleri round-trip'te korunmalı).
        let registry = crate::registry::default_scene_registry();

        // Kaynak dünya: isimli ebeveyn + çocuk, bilinen Transform değerleriyle.
        let mut world = World::new();
        let parent = world.spawn();
        world.add_component(parent, EntityName::new("Parent"));
        world.add_component(parent, Transform::new(Vec3::new(1.0, 2.0, 3.0)));
        let child = world.spawn();
        world.add_component(child, EntityName::new("Child"));
        world.add_component(child, Transform::new(Vec3::new(-4.0, 5.0, -6.0)));
        world.add_component(child, Parent(parent.id()));

        let path = std::env::temp_dir()
            .join("gizmo_scene_roundtrip_test.ron")
            .to_string_lossy()
            .into_owned();
        SceneData::save(&world, &path, &registry).expect("save başarısız");

        // TAZE dünyaya yükle.
        let mut loaded = World::new();
        SceneData::load_into(&path, &mut loaded, &registry).expect("load başarısız");
        let _ = std::fs::remove_file(&path);

        // İsme göre entity bul.
        let find = |w: &World, want: &str| -> Option<u32> {
            let names = w.borrow::<EntityName>();
            w.iter_alive_entities()
                .into_iter()
                .map(|e| e.id())
                .find(|&id| names.get(id).map(|n| n.0.as_str()) == Some(want))
        };

        let p = find(&loaded, "Parent").expect("Parent yüklenmedi");
        let c = find(&loaded, "Child").expect("Child yüklenmedi");

        // Transform değerleri korunmalı.
        {
            let ts = loaded.borrow::<Transform>();
            let pt = ts.get(p).expect("Parent Transform yok (reflect round-trip bozuk)");
            let ct = ts.get(c).expect("Child Transform yok");
            assert_eq!(pt.position, Vec3::new(1.0, 2.0, 3.0), "Parent pozisyonu round-trip'te bozuldu");
            assert_eq!(ct.position, Vec3::new(-4.0, 5.0, -6.0), "Child pozisyonu round-trip'te bozuldu");
        }

        // Hiyerarşi korunmalı: Child'ın Parent'ı yeni p id'sine çözülmeli.
        {
            let parents = loaded.borrow::<Parent>();
            assert_eq!(parents.get(c).map(|x| x.0), Some(p), "ebeveyn-çocuk ilişkisi round-trip'te bozuldu");
        }
    }

    // REGRESYON (audit 2026-06-29): id slotu yeniden kullanılmış (despawn→spawn,
    // generation ≥ 1) bir entity'nin dinamik bileşenleri sahne diske kaydedilirken
    // kaybolmamalı. Eski kod `Entity::new(id, 0)` ile lookup yaptığından
    // `entity_component_types`'ın `is_alive` generation kontrolü başarısız olur,
    // boş döner ve Transform sessizce düşerdi. Name/Mesh/Material/Parent raw-id ile
    // okunduğu için hayatta kalırdı → kısmi, sinsi bozulma.
    #[test]
    fn save_preserves_components_for_recycled_id_entity() {
        use gizmo_physics_core::components::Transform;
        use gizmo_math::Vec3;

        let registry = crate::registry::default_scene_registry();
        let mut world = World::new();

        // id 0'ı yak → sonraki spawn onu generation 1 ile geri kullanır.
        let burned = world.spawn();
        let burned_id = burned.id();
        world.despawn(burned);

        let e = world.spawn();
        assert_eq!(e.id(), burned_id, "ön koşul: id yeniden kullanılmalı");
        assert_ne!(e.generation(), 0, "ön koşul: recycled entity generation ≥ 1");
        world.add_component(e, EntityName::new("Recycled"));
        world.add_component(e, Transform::new(Vec3::new(7.0, 8.0, 9.0)));

        let path = std::env::temp_dir()
            .join("gizmo_scene_recycled_id_test.ron")
            .to_string_lossy()
            .into_owned();
        SceneData::save(&world, &path, &registry).expect("save başarısız");

        let mut loaded = World::new();
        SceneData::load_into(&path, &mut loaded, &registry).expect("load başarısız");
        let _ = std::fs::remove_file(&path);

        let id = {
            let names = loaded.borrow::<EntityName>();
            loaded
                .iter_alive_entities()
                .into_iter()
                .map(|x| x.id())
                .find(|&id| names.get(id).map(|n| n.0.as_str()) == Some("Recycled"))
                .expect("Recycled entity yüklenmedi")
        };
        let ts = loaded.borrow::<Transform>();
        let t = ts
            .get(id)
            .expect("recycled-id entity'nin Transform'ı kaydedilirken DÜŞTÜ (generation-0 lookup bug)");
        assert_eq!(t.position, Vec3::new(7.0, 8.0, 9.0), "Transform değeri korunmadı");
    }
}
