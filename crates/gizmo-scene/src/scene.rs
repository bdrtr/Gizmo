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
    #[tracing::instrument(skip_all, name = "scene_save", fields(path = %file_path))]
    pub fn save(
        world: &World,
        file_path: &str,
        registry: &gizmo_core::registry::ComponentRegistry,
    ) -> Result<(), SceneError> {
        if let Some(parent) = std::path::Path::new(file_path).parent() {
            // Non-fatal here: `fs::write` below surfaces the real failure if the directory
            // is genuinely unusable. Log it so that later write error has a root cause.
            if let Err(e) = fs::create_dir_all(parent) {
                tracing::warn!(
                    dir = %parent.display(),
                    error = %e,
                    "[Scene] sahne dizini oluşturulamadı (kayıt yine de denenecek)",
                );
            }
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
            .map_err(|e| {
                tracing::error!(
                    error = %e,
                    entity_count = scene.entities.len(),
                    "[Scene] sahne RON serileştirmesi başarısız",
                );
                e
            })?;
        let byte_len = string_data.len();

        fs::write(file_path, string_data).map_err(|e| {
            tracing::error!(
                path = %file_path,
                error = %e,
                bytes = byte_len,
                "[Scene] sahne dosyaya yazılamadı",
            );
            e
        })?;

        tracing::info!(
            path = %file_path,
            entity_count = scene.entities.len(),
            joint_count = scene.joints.len(),
            bytes = byte_len,
            "[Scene] Sahne kaydedildi",
        );
        Ok(())
    }

    #[tracing::instrument(skip_all, name = "serialize_entities")]
    pub fn serialize_entities(
        world: &World,
        entity_ids: Vec<u32>,
        registry: &gizmo_core::registry::ComponentRegistry,
    ) -> Vec<EntityData> {
        let mut entities_data = Vec::new();
        // Aggregated over all entities: how many dynamic components were written vs. dropped
        // (a drop is silent data loss; `serde_bridge` logs the per-component reason).
        let mut component_count = 0usize;
        let mut dropped_components = 0usize;
        let names = world.borrow::<EntityName>();
        let meshes = world.borrow::<MeshSource>();
        let materials = world.borrow::<MaterialSource>();
        let parents = world.borrow::<Parent>();
        let children_store = world.borrow::<Children>();

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
                            match crate::serde_bridge::serialize_component(
                                registry, reg, type_id, ptr,
                            ) {
                                Some(string_repr) => {
                                    component_count += 1;
                                    dynamic_components.insert(reg.name.clone(), string_repr);
                                }
                                // serde_bridge already logged the concrete reason.
                                None => dropped_components += 1,
                            }
                        }
                    }
                }
            }

            // Keep a "bare" node that only carries a non-empty `Children` list: it is
            // a structural group/pivot whose presence keeps its subtree attached on
            // reload (children resolve their `parent_id` back to it). Dropping it left
            // the children pointing at a missing id → detached subtree.
            let is_group_node = children_store
                .get(id)
                .is_some_and(|c| !c.0.is_empty());

            if name.is_some()
                || mesh_source.is_some()
                || material_source.is_some()
                || parent_id.is_some()
                || !dynamic_components.is_empty()
                || is_group_node
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
        
        // Operation detail (counts), not a lifecycle event: the outer `save`/`save_prefab`
        // emit the single info! line. Warn only when a component was actually dropped.
        if dropped_components > 0 {
            tracing::warn!(
                dropped_components,
                serialized = entities_data.len(),
                "[Scene] bazı bileşenler serialize edilemedi ve kaydedilmedi (veri kaybı)",
            );
        }
        tracing::debug!(
            checked = entity_ids.len(),
            serialized = entities_data.len(),
            component_count,
            dropped_components,
            "[Scene] serialize_entities tamamlandı",
        );
        entities_data
    }

    /// JSON sahne dosyasını okuyup World'e entity olarak yükler
    #[tracing::instrument(skip_all, name = "scene_load", fields(path = %file_path))]
    pub fn load_into(
        file_path: &str,
        world: &mut World,
        registry: &gizmo_core::registry::ComponentRegistry,
    ) -> Result<(), SceneError> {
        let string_data = fs::read_to_string(file_path).map_err(|e| {
            tracing::error!(path = %file_path, error = %e, "[Scene] sahne dosyası okunamadı");
            e
        })?;

        let scene: SceneData = ron::from_str(&string_data).map_err(|e| {
            tracing::error!(
                path = %file_path,
                error = %e,
                bytes = string_data.len(),
                "[Scene] sahne RON ayrıştırma (parse) hatası",
            );
            e
        })?;

        let entities = scene.entities;
        let entity_count = entities.len();
        tracing::debug!(
            entity_count,
            joint_count = scene.joints.len(),
            "[Scene] sahne dosyası ayrıştırıldı",
        );

        let id_map = Self::instantiate_entities(
            entities,
            None,
            world,
            registry,
        );

        let mut joints_added = 0usize;
        let mut joints_skipped = 0usize;
        if let Ok(mut physics_world) = world.try_get_resource_mut::<gizmo_physics_rigid::world::PhysicsWorld>() {
            for mut joint in scene.joints {
                if let (Some(&new_a), Some(&new_b)) = (id_map.get(&joint.entity_a.id()), id_map.get(&joint.entity_b.id())) {
                    joint.entity_a = gizmo_physics_rigid::BodyHandle::from_id(new_a);
                    joint.entity_b = gizmo_physics_rigid::BodyHandle::from_id(new_b);
                    physics_world.joints.push(joint);
                    joints_added += 1;
                } else {
                    // A joint whose bodies weren't in the saved set is silently lost. On a
                    // full scene load every body should be present, so this signals a
                    // truncated/inconsistent scene rather than an expected optional case.
                    joints_skipped += 1;
                }
            }
        }
        if joints_skipped > 0 {
            tracing::warn!(
                joints_skipped,
                joints_added,
                "[Scene] bazı joint'lerin gövde handle'ları çözülemedi — atlandı (sahne eksik gövde içeriyor)",
            );
        }

        tracing::info!(
            path = %file_path,
            entity_count,
            joints_added,
            "[Scene] Sahne yüklendi",
        );
        Ok(())
    }

    #[tracing::instrument(skip_all, name = "instantiate_entities")]
    pub fn instantiate_entities(
        entities: Vec<EntityData>,
        root_parent: Option<u32>,
        world: &mut World,
        registry: &gizmo_core::registry::ComponentRegistry,
    ) -> HashMap<u32, u32> {
        let requested = entities.len();
        let mut id_map = HashMap::new();
        let mut entity_structs = HashMap::new();

        for data in &entities {
            let root_ent = world.spawn();
            id_map.insert(data.original_id, root_ent.id());
            entity_structs.insert(root_ent.id(), root_ent);
        }

        let mut children_map: HashMap<u32, Vec<u32>> = HashMap::new();
        // Per-load aggregates for the exit summary.
        let mut components_loaded = 0usize;
        let mut components_failed = 0usize;
        let mut unknown_type_names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

        for data in entities {
            let new_id = id_map[&data.original_id];
            let entity = entity_structs[&new_id];

            if let Some(n) = data.name {
                world.add_component(entity, EntityName::new(&n));
            }
            
            for (comp_name, comp_val) in &data.components {
                match registry.get_type_id(comp_name) {
                    Some(type_id) => {
                        if let Some(reg) = registry.get_registration(type_id) {
                            // Recoverable (the rest of the entity/scene still loads), so warn
                            // rather than error — but name the component + entity + reason.
                            if let Err(e) = crate::serde_bridge::deserialize_component(
                                world, entity, registry, reg, type_id, comp_val,
                            ) {
                                components_failed += 1;
                                tracing::warn!(
                                    component = %comp_name,
                                    entity = new_id,
                                    error = %e,
                                    "[Scene] bileşen deserialize edilemedi — entity'ye eklenmedi",
                                );
                            } else {
                                components_loaded += 1;
                            }
                        } else {
                            unknown_type_names.insert(comp_name.clone());
                        }
                    }
                    // Component name is not in the registry: it is silently skipped. Usually
                    // means the layer that owns the type (e.g. scripting's `Script`) wasn't
                    // registered into this registry. Collected + warned once at the end.
                    None => {
                        unknown_type_names.insert(comp_name.clone());
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

            // Resolve the parent to a freshly-spawned id. If `parent_id` names an
            // entity that isn't in the saved set (e.g. a prefab whose bare root was
            // filtered out, or a scene child of an editor-only parent), fall back to
            // `root_parent` instead of silently orphaning the entity — previously the
            // `else` fallback was skipped whenever `parent_id` was `Some` but unresolved.
            let resolved_parent = data
                .parent_id
                .and_then(|orig_parent| id_map.get(&orig_parent).copied())
                .or(root_parent);

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

        if !unknown_type_names.is_empty() {
            tracing::warn!(
                distinct_unknown_types = ?unknown_type_names,
                count = unknown_type_names.len(),
                "[Scene] sahnede registry'de OLMAYAN bileşen tipleri atlandı (eksik-tip; ilgili katman register etmemiş olabilir)",
            );
        }
        tracing::debug!(
            spawned = id_map.len(),
            requested,
            components_loaded,
            components_failed,
            "[Scene] instantiate_entities tamamlandı",
        );
        id_map
    }

    /// Prefab kaydet
    #[tracing::instrument(skip_all, name = "save_prefab", fields(path = %file_path, root = root_entity_id))]
    pub fn save_prefab(
        world: &World,
        root_entity_id: u32,
        file_path: &str,
        registry: &gizmo_core::registry::ComponentRegistry,
    ) -> Result<(), SceneError> {
        if let Some(parent) = std::path::Path::new(file_path).parent() {
            // Non-fatal: `fs::write` below surfaces a genuinely unusable directory.
            if let Err(e) = fs::create_dir_all(parent) {
                tracing::warn!(
                    dir = %parent.display(),
                    error = %e,
                    "[Scene] prefab dizini oluşturulamadı (kayıt yine de denenecek)",
                );
            }
        }

        let mut ids_to_save = vec![root_entity_id];
        let mut visited: std::collections::HashSet<u32> = std::collections::HashSet::new();
        visited.insert(root_entity_id);
        let children_storage = world.borrow::<Children>();

        let mut i = 0;
        while i < ids_to_save.len() {
            let current = ids_to_save[i];
            if let Some(children_comp) = children_storage.get(current) {
                for &child_id in &children_comp.0 {
                    // Guard the BFS with a visited set: a `Children` CYCLE (e.g. the
                    // studio lets you drag an entity onto its own descendant) would
                    // otherwise grow `ids_to_save` forever (hang/OOM), and a SHARED
                    // child on a diamond hierarchy would be emitted twice → duplicate
                    // `EntityData { original_id }` → clobbered/leaked entities on reload.
                    if visited.insert(child_id) {
                        ids_to_save.push(child_id);
                    }
                }
            }
            i += 1;
        }

        let mut entities_data = Self::serialize_entities(world, ids_to_save.clone(), registry);

        // A "bare" root (e.g. an empty group/pivot node carrying only a `Children`
        // component, with no name/mesh/material/dynamic components) is dropped by the
        // skip-filter in `serialize_entities`. Force it back in so the prefab root
        // always exists on reload — otherwise `load_prefab` can't map `root_id`, the
        // root is never spawned, and its whole subtree detaches.
        if !entities_data
            .iter()
            .any(|d| d.original_id == root_entity_id)
        {
            entities_data.push(EntityData {
                original_id: root_entity_id,
                name: None,
                mesh_source: None,
                material_source: None,
                parent_id: None,
                components: std::collections::BTreeMap::new(),
            });
        }

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
            .map_err(|e| {
                tracing::error!(
                    error = %e,
                    entity_count = prefab.entities.len(),
                    "[Scene] prefab RON serileştirmesi başarısız",
                );
                e
            })?;
        let byte_len = string_data.len();

        fs::write(file_path, string_data).map_err(|e| {
            tracing::error!(
                path = %file_path,
                error = %e,
                bytes = byte_len,
                "[Scene] prefab dosyaya yazılamadı",
            );
            e
        })?;

        tracing::info!(
            path = %file_path,
            root = root_entity_id,
            entity_count = prefab.entities.len(),
            joint_count = prefab.joints.len(),
            bytes = byte_len,
            "[Scene] Prefab kaydedildi",
        );
        Ok(())
    }

    /// Prefab yükle
    #[tracing::instrument(skip_all, name = "load_prefab", fields(path = %file_path))]
    pub fn load_prefab(
        file_path: &str,
        parent_entity: Option<u32>,
        world: &mut World,
        registry: &gizmo_core::registry::ComponentRegistry,
    ) -> Result<Option<u32>, SceneError> {
        let string_data = fs::read_to_string(file_path).map_err(|e| {
            tracing::error!(path = %file_path, error = %e, "[Scene] prefab dosyası okunamadı");
            e
        })?;

        let prefab: PrefabData = ron::from_str(&string_data).map_err(|e| {
            tracing::error!(
                path = %file_path,
                error = %e,
                bytes = string_data.len(),
                "[Scene] prefab RON ayrıştırma (parse) hatası",
            );
            e
        })?;

        let saved_root = prefab.root_id;
        let entity_count = prefab.entities.len();

        let id_map = Self::instantiate_entities(
            prefab.entities,
            parent_entity,
            world,
            registry,
        );

        let new_root_id = id_map.get(&saved_root).copied();
        if new_root_id.is_none() {
            tracing::warn!(
                root = saved_root,
                "[Scene] prefab kök id'si instantiate sonrası çözülemedi — kök/alt-ağaç bağlanamayabilir",
            );
        }

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

        let mut joints_added = 0usize;
        let mut joints_skipped = 0usize;
        if let Ok(mut physics_world) = world.try_get_resource_mut::<gizmo_physics_rigid::world::PhysicsWorld>() {
            for mut joint in prefab.joints {
                if let (Some(&new_a), Some(&new_b)) = (id_map.get(&joint.entity_a.id()), id_map.get(&joint.entity_b.id())) {
                    joint.entity_a = gizmo_physics_rigid::BodyHandle::from_id(new_a);
                    joint.entity_b = gizmo_physics_rigid::BodyHandle::from_id(new_b);
                    physics_world.joints.push(joint);
                    joints_added += 1;
                } else {
                    joints_skipped += 1;
                }
            }
        }
        if joints_skipped > 0 {
            tracing::warn!(
                joints_skipped,
                joints_added,
                "[Scene] bazı prefab joint'lerinin gövde handle'ları çözülemedi — atlandı",
            );
        }

        tracing::info!(
            path = %file_path,
            entity_count,
            joints_added,
            "[Scene] Prefab yüklendi",
        );
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

    // DEKLARATİF SAHNE (P4): bir geliştiricinin ELLE yazacağı bir RON "level" doğrudan
    // yüklenebilmeli — mevcut round-trip testleri hep önce `save` ile MAKİNE RON'u üretiyor;
    // bu test insan-yazımı akışı kanıtlar + kopyala-yapıştır ŞABLON görevi görür. Level'i
    // `load_level`'daki gibi elle spawn'lamak yerine bir dosyadan yüklemenin declarative yolu.
    #[test]
    fn hand_authored_scene_ron_loads_and_spawns() {
        use gizmo_math::Vec3;
        use gizmo_physics_core::components::Transform;

        // Bir insanın yazdığı declarative sahne (zemin + 2 kutu). Bileşen değerleri RON.
        let ron_scene = r#"(
            entities: [
                (
                    original_id: 1, name: Some("ground"), mesh_source: Some("cube"),
                    material_source: Some((albedo: (0.3, 0.3, 0.3, 1.0), roughness: 0.9, metallic: 0.0, unlit: 0.0, texture_source: None)),
                    parent_id: None,
                    components: { "Transform": "(position:(0.0,-0.5,0.0),rotation:(0.0,0.0,0.0,1.0),scale:(10.0,0.5,10.0))" },
                ),
                (
                    original_id: 2, name: Some("box_a"), mesh_source: Some("cube"), material_source: None, parent_id: None,
                    components: { "Transform": "(position:(-1.0,1.0,0.0),rotation:(0.0,0.0,0.0,1.0),scale:(1.0,1.0,1.0))" },
                ),
                (
                    original_id: 3, name: Some("box_b"), mesh_source: Some("cube"), material_source: None, parent_id: None,
                    components: { "Transform": "(position:(1.0,2.0,0.0),rotation:(0.0,0.0,0.0,1.0),scale:(1.0,1.0,1.0))" },
                ),
            ],
            joints: [],
        )"#;

        let scene: SceneData = ron::from_str(ron_scene).expect("el-yazımı sahne RON'u geçerli olmalı");
        let registry = crate::registry::default_scene_registry();
        let mut world = World::new();
        SceneData::instantiate_entities(scene.entities, None, &mut world, &registry);

        // 3 varlık isim + Transform pozisyonlarıyla spawn olmalı (declarative → gerçek entity).
        let names = world.borrow::<EntityName>();
        let transforms = world.borrow::<Transform>();
        let pos_of = |want: &str| -> Option<Vec3> {
            world
                .iter_alive_entities()
                .into_iter()
                .map(|e| e.id())
                .find(|&id| names.get(id).map(|n| n.0.as_str()) == Some(want))
                .and_then(|id| transforms.get(id).map(|t| t.position))
        };
        assert_eq!(pos_of("ground"), Some(Vec3::new(0.0, -0.5, 0.0)), "ground pozisyonu");
        assert_eq!(pos_of("box_a"), Some(Vec3::new(-1.0, 1.0, 0.0)), "box_a pozisyonu");
        assert_eq!(pos_of("box_b"), Some(Vec3::new(1.0, 2.0, 0.0)), "box_b pozisyonu");
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

    // REGRESYON: "çıplak" bir grup/pivot kök (yalnız `Children`, isim/mesh/dinamik
    // bileşen yok) prefab olarak kaydedilip yüklendiğinde kök + tüm alt-ağaç KORUNMALI.
    // Eski kodda serialize_entities çıplak kökü atlıyor, load_prefab `root_id`'yi
    // haritalayamıyor → kök hiç spawn edilmiyor, çocuklar da öksüz kalıyordu.
    #[test]
    fn prefab_roundtrip_keeps_bare_group_root_and_children() {
        let registry = crate::registry::default_scene_registry();

        let mut world = World::new();
        // Çıplak grup kökü: yalnızca Children (isim/Transform/mesh YOK → skip-filter'e takılır).
        let root = world.spawn();
        let a = world.spawn();
        let b = world.spawn();
        world.add_component(a, EntityName::new("A"));
        world.add_component(a, Parent(root.id()));
        world.add_component(b, EntityName::new("B"));
        world.add_component(b, Parent(root.id()));
        world.add_component(root, Children(vec![a.id(), b.id()]));

        let path = std::env::temp_dir()
            .join("gizmo_prefab_bare_root_test.ron")
            .to_string_lossy()
            .into_owned();
        SceneData::save_prefab(&world, root.id(), &path, &registry).expect("save_prefab başarısız");

        // Taze dünyaya bir host altına yükle.
        let mut loaded = World::new();
        let host = loaded.spawn();
        let new_root = SceneData::load_prefab(&path, Some(host.id()), &mut loaded, &registry)
            .expect("load_prefab başarısız");
        let _ = std::fs::remove_file(&path);

        // Kök round-trip'te var olmalı (fix'ten önce düşüyordu → None).
        let new_root = new_root.expect("prefab kökü reload sonrası var olmalı");

        let find = |w: &World, want: &str| -> Option<u32> {
            let names = w.borrow::<EntityName>();
            w.iter_alive_entities()
                .into_iter()
                .map(|e| e.id())
                .find(|&id| names.get(id).map(|n| n.0.as_str()) == Some(want))
        };
        let a2 = find(&loaded, "A").expect("çocuk A yüklenmedi");
        let b2 = find(&loaded, "B").expect("çocuk B yüklenmedi");

        let parents = loaded.borrow::<Parent>();
        assert_eq!(
            parents.get(a2).map(|x| x.0),
            Some(new_root),
            "A prefab köküne bağlı kalmalı (öksüz kalmamalı)"
        );
        assert_eq!(
            parents.get(b2).map(|x| x.0),
            Some(new_root),
            "B prefab köküne bağlı kalmalı"
        );
        assert_eq!(
            parents.get(new_root).map(|x| x.0),
            Some(host.id()),
            "prefab kökü istenen host'a bağlanmalı"
        );
    }

    #[test]
    fn save_prefab_dedups_shared_child_on_diamond_hierarchy() {
        let registry = crate::registry::default_scene_registry();
        let mut world = World::new();
        // Diamond: root -> {A, B}, and BOTH A and B list C as a child.
        let root = world.spawn();
        let a = world.spawn();
        let b = world.spawn();
        let c = world.spawn();
        world.add_component(root, EntityName::new("ROOT"));
        world.add_component(a, EntityName::new("A"));
        world.add_component(b, EntityName::new("B"));
        world.add_component(c, EntityName::new("C"));
        world.add_component(a, Parent(root.id()));
        world.add_component(b, Parent(root.id()));
        world.add_component(c, Parent(a.id()));
        world.add_component(root, Children(vec![a.id(), b.id()]));
        world.add_component(a, Children(vec![c.id()]));
        world.add_component(b, Children(vec![c.id()])); // shared child C (diamond)

        let path = std::env::temp_dir()
            .join("gizmo_prefab_diamond_test.ron")
            .to_string_lossy()
            .into_owned();
        SceneData::save_prefab(&world, root.id(), &path, &registry).expect("save_prefab başarısız");

        let mut loaded = World::new();
        let host = loaded.spawn();
        SceneData::load_prefab(&path, Some(host.id()), &mut loaded, &registry)
            .expect("load_prefab başarısız");
        let _ = std::fs::remove_file(&path);

        let alive = loaded.iter_alive_entities();
        let names = loaded.borrow::<EntityName>();
        // Shared child C must be emitted exactly once — the old BFS pushed it twice
        // (via A and via B) → two EntityData with the same original_id → one leaked
        // empty entity on reload.
        let c_count = alive
            .iter()
            .filter(|e| names.get(e.id()).map(|n| n.0.as_str()) == Some("C"))
            .count();
        assert_eq!(c_count, 1, "paylaşılan çocuk C tam bir kez görünmeli");
        // host + ROOT + A + B + C = 5 (eski kod 6. bir boş entity sızdırırdı).
        assert_eq!(alive.len(), 5, "diamond'da sızan yinelenmiş entity olmamalı");
    }

    #[test]
    fn save_prefab_terminates_on_children_cycle() {
        let registry = crate::registry::default_scene_registry();
        let mut world = World::new();
        let root = world.spawn();
        let x = world.spawn();
        world.add_component(root, EntityName::new("ROOT"));
        world.add_component(x, EntityName::new("X"));
        // `Children` cycle root -> x -> root. The old BFS had no visited set → grew
        // `ids_to_save` forever (hang/OOM). Completing at all is the assertion.
        world.add_component(root, Children(vec![x.id()]));
        world.add_component(x, Children(vec![root.id()]));

        let path = std::env::temp_dir()
            .join("gizmo_prefab_cycle_test.ron")
            .to_string_lossy()
            .into_owned();
        SceneData::save_prefab(&world, root.id(), &path, &registry)
            .expect("save_prefab bir döngüde sonlanmalı");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn save_keeps_bare_group_node_so_subtree_stays_attached() {
        use gizmo_math::Vec3;
        use gizmo_physics_core::components::Transform;

        let registry = crate::registry::default_scene_registry();
        let mut world = World::new();
        // Bare group/pivot node: ONLY a `Children` list (no name/mesh/material/
        // dynamic component). Its child carries real state + a Parent back to it.
        let group = world.spawn();
        let child = world.spawn();
        world.add_component(child, EntityName::new("CHILD"));
        world.add_component(child, Transform::new(Vec3::new(1.0, 2.0, 3.0)));
        world.add_component(child, Parent(group.id()));
        world.add_component(group, Children(vec![child.id()]));

        let path = std::env::temp_dir()
            .join("gizmo_scene_bare_group_test.ron")
            .to_string_lossy()
            .into_owned();
        SceneData::save(&world, &path, &registry).expect("save başarısız");

        let mut loaded = World::new();
        SceneData::load_into(&path, &mut loaded, &registry).expect("load başarısız");
        let _ = std::fs::remove_file(&path);

        // The child must remain parented — the bare group node was kept, so the
        // subtree isn't detached. Old skip-filter dropped it → child.parent_id
        // referenced a missing id → child became a detached root.
        let names = loaded.borrow::<EntityName>();
        let child2 = loaded
            .iter_alive_entities()
            .into_iter()
            .find(|e| names.get(e.id()).map(|n| n.0.as_str()) == Some("CHILD"))
            .expect("çocuk yüklenmedi");
        drop(names);
        let parents = loaded.borrow::<Parent>();
        assert!(
            parents.get(child2.id()).is_some(),
            "çocuk, korunan grup node'una bağlı kalmalı (öksüz kalmamalı)"
        );
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

    // ─────────────────────────────────────────────────────────────────────────
    //  Pure (de)serialization: the on-disk data types must survive a RON round
    //  trip losslessly. `encode → decode == identity` is the core save/load
    //  invariant, tested in isolation (no World, no disk).
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn material_data_ron_round_trip_is_lossless() {
        // Both the textured and untextured (`None`) shapes must survive intact.
        for mat in [
            MaterialData {
                albedo: [0.1, 0.2, 0.3, 0.4],
                roughness: 0.55,
                metallic: 0.25,
                unlit: 1.0,
                texture_source: Some("wood/oak.png".to_string()),
            },
            MaterialData {
                albedo: [1.0, 1.0, 1.0, 1.0],
                roughness: 0.0,
                metallic: 0.0,
                unlit: 0.0,
                texture_source: None,
            },
        ] {
            let encoded = ron::ser::to_string(&mat).expect("MaterialData serialize");
            let decoded: MaterialData = ron::from_str(&encoded).expect("MaterialData deserialize");
            assert_eq!(decoded, mat, "MaterialData RON round-trip must be bit-faithful");
        }
    }

    #[test]
    fn entity_data_ron_round_trip_is_lossless() {
        let mut components = std::collections::BTreeMap::new();
        components.insert("Transform".to_string(), "(position:(1.0,2.0,3.0))".to_string());
        components.insert("RigidBody".to_string(), "(mass:5.0)".to_string());
        let entity = EntityData {
            original_id: 42,
            name: Some("Crate".to_string()),
            mesh_source: Some("cube".to_string()),
            material_source: Some(MaterialData {
                albedo: [0.2, 0.4, 0.6, 1.0],
                roughness: 0.7,
                metallic: 0.1,
                unlit: 0.0,
                texture_source: None,
            }),
            parent_id: Some(7),
            components,
        };
        let encoded = ron::ser::to_string(&entity).expect("EntityData serialize");
        let decoded: EntityData = ron::from_str(&encoded).expect("EntityData deserialize");
        assert_eq!(decoded, entity, "EntityData RON round-trip must be bit-faithful");
    }

    // Backward-compat: `SceneData.joints` is `#[serde(default)]`, so an OLDER scene file
    // (written before joints existed) that omits the field must still load — as an empty
    // joint list, not a parse error.
    #[test]
    fn scene_data_parses_when_joints_field_is_omitted() {
        let ron = r#"(entities: [])"#;
        let scene: SceneData = ron::from_str(ron).expect("scene without `joints` must parse");
        assert!(scene.joints.is_empty(), "omitted joints must default to empty");
        assert!(scene.entities.is_empty());
    }

    // `EntityData.parent_id` and `.components` are both `#[serde(default)]`: a hand-authored
    // (or legacy) entity that lists only the required fields must load with `parent_id: None`
    // and an empty component map instead of failing to parse.
    #[test]
    fn entity_data_parses_when_default_fields_are_omitted() {
        let ron = r#"(original_id: 9, name: Some("bare"), mesh_source: None, material_source: None)"#;
        let entity: EntityData = ron::from_str(ron).expect("entity with omitted defaults must parse");
        assert_eq!(entity.original_id, 9);
        assert_eq!(entity.parent_id, None, "omitted parent_id must default to None");
        assert!(entity.components.is_empty(), "omitted components must default to empty");
    }

    // Every joint variant must survive a RON round trip through `PrefabData` with its
    // discriminant AND its distinguishing payload fields intact — the scene format is the
    // authoritative persistence path for joints, so a dropped field silently corrupts a
    // saved constraint.
    #[test]
    fn all_joint_variants_round_trip_through_prefab_ron() {
        use gizmo_math::Vec3;
        use gizmo_physics_rigid::joints::data::{D6Motion, JointData};
        use gizmo_physics_rigid::BodyHandle;

        let h1 = BodyHandle::from_id(1);
        let h2 = BodyHandle::from_id(2);
        let z = Vec3::ZERO;

        // Build one of each, tweaking a distinctive field so we can prove it survives.
        let mut hinge = Joint::hinge(h1, h2, z, z, Vec3::X);
        if let JointData::Hinge(ref mut d) = hinge.data {
            d.use_motor = true;
            d.motor_target_velocity = 3.5;
            d.lower_limit = -1.25;
        }
        let mut ball = Joint::ball_socket(h1, h2, z, z);
        if let JointData::BallSocket(ref mut d) = ball.data {
            d.use_cone_limit = true;
            d.cone_limit_angle = 0.9;
        }
        let mut slider = Joint::slider(h1, h2, z, z, Vec3::Y);
        if let JointData::Slider(ref mut d) = slider.data {
            d.use_spring = true;
            d.spring_stiffness = 250.0;
        }
        let spring = Joint::spring(h1, h2, z, z, 2.0, 100.0, 5.0);
        let rope = Joint::rope(h1, h2, z, z, 5.0);
        let mut d6 = Joint::d6(h1, h2, z, z);
        if let JointData::D6(ref mut d) = d6.data {
            d.linear[0] = D6Motion::Free;
            d.angular[1] = D6Motion::Limited { lower: -1.0, upper: 1.0 };
        }

        let prefab = PrefabData {
            root_id: 1,
            entities: vec![],
            joints: vec![hinge, ball, slider, spring, rope, d6],
        };

        let encoded = ron::ser::to_string(&prefab).expect("prefab joints serialize");
        let decoded: PrefabData = ron::from_str(&encoded).expect("prefab joints deserialize");
        assert_eq!(decoded.joints.len(), 6, "all six joints must survive");

        match &decoded.joints[0].data {
            JointData::Hinge(d) => {
                assert!(d.use_motor, "hinge motor flag lost");
                assert_eq!(d.motor_target_velocity, 3.5, "hinge motor target lost");
                assert_eq!(d.lower_limit, -1.25, "hinge limit lost");
            }
            other => panic!("joint 0 should be Hinge, got {other:?}"),
        }
        match &decoded.joints[1].data {
            JointData::BallSocket(d) => {
                assert!(d.use_cone_limit);
                assert_eq!(d.cone_limit_angle, 0.9);
            }
            other => panic!("joint 1 should be BallSocket, got {other:?}"),
        }
        match &decoded.joints[2].data {
            JointData::Slider(d) => {
                assert!(d.use_spring);
                assert_eq!(d.spring_stiffness, 250.0);
            }
            other => panic!("joint 2 should be Slider, got {other:?}"),
        }
        match &decoded.joints[3].data {
            JointData::Spring(d) => {
                assert_eq!(d.rest_length, 2.0);
                assert_eq!(d.stiffness, 100.0);
                assert_eq!(d.damping, 5.0);
            }
            other => panic!("joint 3 should be Spring, got {other:?}"),
        }
        // Rope is sugar for distance(0, len): min stays 0 (only pulls when taut), max = len.
        match &decoded.joints[4].data {
            JointData::Distance(d) => {
                assert_eq!(d.min_length, 0.0, "rope must have zero min (goes slack)");
                assert_eq!(d.max_length, 5.0);
            }
            other => panic!("joint 4 should be Distance, got {other:?}"),
        }
        match &decoded.joints[5].data {
            JointData::D6(d) => {
                assert_eq!(d.linear[0], D6Motion::Free);
                assert_eq!(d.angular[1], D6Motion::Limited { lower: -1.0, upper: 1.0 });
                assert_eq!(d.angular[0], D6Motion::Locked, "untouched DOF must stay Locked");
            }
            other => panic!("joint 5 should be D6, got {other:?}"),
        }
    }

    // `break_force`/`break_torque` are load-bearing config and must round-trip, while
    // `is_broken` is `#[serde(skip)]` runtime state that must RESET to `false` on load — a
    // saved scene should never resurrect as already-broken.
    #[test]
    fn joint_break_config_round_trips_but_runtime_broken_flag_resets() {
        use gizmo_math::Vec3;
        use gizmo_physics_rigid::BodyHandle;

        let mut joint = Joint::fixed(
            BodyHandle::from_id(1),
            BodyHandle::from_id(2),
            Vec3::ZERO,
            Vec3::ZERO,
        )
        .with_break_force(321.0, 654.0);
        joint.is_broken = true; // runtime state — must NOT persist.

        let prefab = PrefabData { root_id: 1, entities: vec![], joints: vec![joint] };
        let encoded = ron::ser::to_string(&prefab).unwrap();
        let decoded: PrefabData = ron::from_str(&encoded).unwrap();

        let j = &decoded.joints[0];
        assert_eq!(j.break_force, 321.0, "break_force must persist");
        assert_eq!(j.break_torque, 654.0, "break_torque must persist");
        assert!(!j.is_broken, "is_broken is #[serde(skip)] and must reset to false on load");
    }

    // ─────────────────────────────────────────────────────────────────────────
    //  instantiate_entities: id-remapping + parent resolution logic.
    // ─────────────────────────────────────────────────────────────────────────

    // Saved `original_id`s are remapped to freshly-spawned world ids; parent/child links
    // must be re-resolved THROUGH that remap (never left pointing at stale saved ids).
    #[test]
    fn instantiate_entities_remaps_ids_and_rewires_hierarchy() {
        let registry = crate::registry::default_scene_registry();
        let mut world = World::new();

        // Use large original ids that cannot collide with fresh spawn ids (0,1,…),
        // so a successful lookup proves the remap actually happened.
        let entities = vec![
            EntityData {
                original_id: 1000,
                name: Some("P".to_string()),
                mesh_source: None,
                material_source: None,
                parent_id: None,
                components: std::collections::BTreeMap::new(),
            },
            EntityData {
                original_id: 1001,
                name: Some("C".to_string()),
                mesh_source: None,
                material_source: None,
                parent_id: Some(1000),
                components: std::collections::BTreeMap::new(),
            },
        ];

        let id_map = SceneData::instantiate_entities(entities, None, &mut world, &registry);
        let new_p = id_map[&1000];
        let new_c = id_map[&1001];
        assert_ne!(new_p, 1000, "original id must be remapped to a fresh world id");
        assert_ne!(new_c, 1001);

        // Child's Parent points at the REMAPPED parent id.
        let parents = world.borrow::<Parent>();
        assert_eq!(
            parents.get(new_c).map(|p| p.0),
            Some(new_p),
            "child must be reparented to the remapped parent"
        );
        drop(parents);

        // And the parent's Children list gained the remapped child (back-link built).
        let children = world.borrow::<Children>();
        assert!(
            children.get(new_p).map(|c| c.0.contains(&new_c)).unwrap_or(false),
            "parent's Children must include the remapped child"
        );
    }

    // Documented fallback (scene.rs): when `parent_id` is `Some` but names an entity NOT in
    // the saved set (e.g. a child of a filtered editor-only parent), the entity must attach
    // to `root_parent` instead of being silently orphaned with a dangling parent id.
    #[test]
    fn instantiate_entities_unresolved_parent_falls_back_to_root_parent() {
        let registry = crate::registry::default_scene_registry();
        let mut world = World::new();
        let host = world.spawn();

        let entities = vec![EntityData {
            original_id: 500,
            name: Some("Orphan".to_string()),
            mesh_source: None,
            material_source: None,
            parent_id: Some(9999), // not present in the saved set → unresolvable
            components: std::collections::BTreeMap::new(),
        }];

        let id_map =
            SceneData::instantiate_entities(entities, Some(host.id()), &mut world, &registry);
        let new = id_map[&500];

        let parents = world.borrow::<Parent>();
        assert_eq!(
            parents.get(new).map(|p| p.0),
            Some(host.id()),
            "unresolved parent must fall back to root_parent, not orphan the entity"
        );
    }

    // The editor's own scaffolding entities ("Editor …", "Highlight Box") are transient UI
    // and must never be written into a saved scene.
    #[test]
    fn serialize_entities_skips_editor_and_highlight_nodes() {
        let registry = crate::registry::default_scene_registry();
        let mut world = World::new();
        let editor = world.spawn();
        world.add_component(editor, EntityName::new("Editor Camera"));
        let highlight = world.spawn();
        world.add_component(highlight, EntityName::new("Highlight Box"));
        let player = world.spawn();
        world.add_component(player, EntityName::new("Player"));

        let ids: Vec<u32> = world
            .iter_alive_entities()
            .into_iter()
            .map(|e| e.id())
            .collect();
        let data = SceneData::serialize_entities(&world, ids, &registry);
        let names: Vec<String> = data.iter().filter_map(|d| d.name.clone()).collect();

        assert!(names.contains(&"Player".to_string()), "real entity must be serialized");
        assert!(
            !names.iter().any(|n| n == "Editor Camera" || n == "Highlight Box"),
            "editor scaffolding must be filtered out of saves"
        );
    }

    // A real physics component (`Collider`, serde-serialized in every feature config) must
    // survive a full save→load through disk with its shape value intact — end-to-end proof
    // that the serde fallback path in `serde_bridge` preserves component values, not just names.
    #[test]
    fn collider_survives_full_disk_round_trip_with_shape_intact() {
        use gizmo_physics_core::components::{Collider, ColliderShape};
        use gizmo_math::Vec3;

        let registry = crate::registry::default_scene_registry();
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, EntityName::new("Wall"));
        world.add_component(e, Collider::box_collider(Vec3::new(1.5, 2.5, 3.5)));

        let path = std::env::temp_dir()
            .join("gizmo_scene_collider_roundtrip_test.ron")
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
                .find(|&id| names.get(id).map(|n| n.0.as_str()) == Some("Wall"))
                .expect("Wall entity yüklenmedi")
        };
        let colliders = loaded.borrow::<Collider>();
        let col = colliders.get(id).expect("Collider round-trip'te düştü");
        match &col.shape {
            ColliderShape::Box(b) => {
                assert_eq!(b.half_extents, Vec3::new(1.5, 2.5, 3.5), "box extents bozuldu")
            }
            other => panic!("shape Box olmalı, {other:?} bulundu"),
        }
    }

    // A joint stored in the live `PhysicsWorld` resource must be saved AND, on load into a
    // fresh world, have both of its body handles remapped to the newly-spawned entity ids
    // (never left pointing at the old ids) while its break config is preserved.
    #[test]
    fn scene_joint_round_trips_and_remaps_body_handles() {
        use gizmo_math::Vec3;
        use gizmo_physics_rigid::world::PhysicsWorld;
        use gizmo_physics_rigid::BodyHandle;

        let registry = crate::registry::default_scene_registry();
        let mut world = World::new();
        world.insert_resource(PhysicsWorld::new());
        let a = world.spawn();
        world.add_component(a, EntityName::new("BodyA"));
        let b = world.spawn();
        world.add_component(b, EntityName::new("BodyB"));
        {
            let mut pw = world
                .try_get_resource_mut::<PhysicsWorld>()
                .expect("PhysicsWorld resource");
            pw.joints.push(
                Joint::fixed(
                    BodyHandle::from_id(a.id()),
                    BodyHandle::from_id(b.id()),
                    Vec3::ZERO,
                    Vec3::ZERO,
                )
                .with_break_force(123.0, 456.0),
            );
        }

        let path = std::env::temp_dir()
            .join("gizmo_scene_joint_remap_test.ron")
            .to_string_lossy()
            .into_owned();
        SceneData::save(&world, &path, &registry).expect("save başarısız");

        let mut loaded = World::new();
        loaded.insert_resource(PhysicsWorld::new());
        SceneData::load_into(&path, &mut loaded, &registry).expect("load başarısız");
        let _ = std::fs::remove_file(&path);

        let find = |w: &World, want: &str| -> u32 {
            let names = w.borrow::<EntityName>();
            w.iter_alive_entities()
                .into_iter()
                .map(|e| e.id())
                .find(|&id| names.get(id).map(|n| n.0.as_str()) == Some(want))
                .unwrap_or_else(|| panic!("{want} yüklenmedi"))
        };
        let new_a = find(&loaded, "BodyA");
        let new_b = find(&loaded, "BodyB");

        let pw = loaded
            .try_get_resource::<PhysicsWorld>()
            .expect("PhysicsWorld resource yüklendi");
        assert_eq!(pw.joints.len(), 1, "joint round-trip'te kayboldu");
        let j = &pw.joints[0];
        assert_eq!(j.entity_a.id(), new_a, "entity_a yeni id'ye remap edilmeli");
        assert_eq!(j.entity_b.id(), new_b, "entity_b yeni id'ye remap edilmeli");
        assert_eq!(j.break_force, 123.0, "break_force korunmalı");
        assert_eq!(j.break_torque, 456.0, "break_torque korunmalı");
    }

    // Error path: loading a path that doesn't exist must surface as `SceneError::Io`
    // (the file read failure), not a panic and not a misclassified parse error.
    #[test]
    fn load_into_missing_file_returns_io_error() {
        let registry = crate::registry::default_scene_registry();
        let mut world = World::new();
        let missing = std::env::temp_dir()
            .join("gizmo_scene_definitely_missing_file_9e3.ron")
            .to_string_lossy()
            .into_owned();
        let _ = std::fs::remove_file(&missing); // ensure absent
        let err = SceneData::load_into(&missing, &mut world, &registry).unwrap_err();
        assert!(matches!(err, SceneError::Io(_)), "missing file must be an Io error");
    }

    // Error path: a file that exists but contains garbage RON must surface as
    // `SceneError::Parse`, distinguishing a corrupt scene from a missing one.
    #[test]
    fn load_into_malformed_ron_returns_parse_error() {
        let registry = crate::registry::default_scene_registry();
        let mut world = World::new();
        let path = std::env::temp_dir()
            .join("gizmo_scene_malformed_test.ron")
            .to_string_lossy()
            .into_owned();
        std::fs::write(&path, b"this is not valid ron @@@ {{{").expect("write temp");
        let err = SceneData::load_into(&path, &mut world, &registry).unwrap_err();
        let _ = std::fs::remove_file(&path);
        assert!(matches!(err, SceneError::Parse(_)), "garbage RON must be a Parse error");
    }

    // Name lookup helpers (the Lua/editor query surface): exact-match hit, miss returns
    // `None`, and `entity_names` enumerates every named entity.
    #[test]
    fn find_entity_by_name_and_entity_names_enumerate_named_entities() {
        let mut world = World::new();
        let a = world.spawn();
        world.add_component(a, EntityName::new("Alpha"));
        let b = world.spawn();
        world.add_component(b, EntityName::new("Beta"));

        assert_eq!(SceneData::find_entity_by_name(&world, "Beta"), Some(b.id()));
        assert_eq!(SceneData::find_entity_by_name(&world, "Alpha"), Some(a.id()));
        assert_eq!(
            SceneData::find_entity_by_name(&world, "Missing"),
            None,
            "a name with no match must return None"
        );

        let names = SceneData::entity_names(&world);
        assert_eq!(names.len(), 2);
        assert!(names.iter().any(|(id, n)| *id == a.id() && n == "Alpha"));
        assert!(names.iter().any(|(id, n)| *id == b.id() && n == "Beta"));
    }
}
