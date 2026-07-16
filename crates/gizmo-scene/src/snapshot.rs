//! SceneSnapshot — Play/Stop Modu için in-memory sahne yedeği
//!
//! Editörde "Play" butonuna basıldığında tüm sahne durumu (entity'ler, fizik
//! bileşenleri, transform'lar) belleğe snapshot olarak alınır. "Stop"
//! butonuna basıldığında bu snapshot'tan geri yükleme yapılır.
//!
//! Avantajlar:
//! - Disk I/O yok → anlık snapshot/restore (~mikrosaniye)
//! - Fizik state korunur (velocity, angular_velocity, sleep state)
//! - Serileştirme gerektirmeyen bileşenler de yedeklenir

use gizmo_core::World;

#[cfg(target_arch = "wasm32")]
use web_time::Instant;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

/// Tek bir entity'nin in-memory yedeği
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct EntitySnapshot {
    pub entity_id: u32,
    pub name: Option<String>,
    pub components: std::collections::BTreeMap<String, String>,
}

/// Tüm sahnenin in-memory snapshot'ı
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct SceneSnapshot {
    pub entities: Vec<EntitySnapshot>,
    pub timestamp: Instant,
}

impl SceneSnapshot {
    /// Mevcut World durumunu in-memory snapshot olarak yakalar.
    /// `protected_ids`: Editor kamerası, highlight box gibi korunacak entity'ler
    #[tracing::instrument(skip_all, name = "snapshot_capture")]
    pub fn capture(
        world: &World,
        registry: &crate::registry::SceneRegistry,
        protected_ids: &std::collections::HashSet<u32>,
    ) -> Self {
        let start = Instant::now();
        let mut entities = Vec::new();
        // Aggregates: a dropped component here means Play→Stop will silently lose that state.
        let mut component_count = 0usize;
        let mut dropped_components = 0usize;
        let names = world.borrow::<gizmo_core::EntityName>();

        for ent in world.iter_alive_entities() {
            let id = ent.id();
            if protected_ids.contains(&id) {
                continue;
            }

            // Editor internal entity'lerini atla
            if let Some(name) = names.get(id) {
                if name.0.starts_with("Editor ") || name.0 == "Highlight Box" {
                    continue;
                }
            }

            let name = names.get(id).map(|n| n.0.clone());

            // Registry üzerinden tüm dinamik bileşenleri RON AST'sine dönüştür
            let mut components = std::collections::BTreeMap::new();

            // `ent`'in GERÇEK generation'ını kullan. `Entity::new(id, 0)` ile sabit
            // generation-0 lookup'ı, id slotu yeniden kullanılmış (despawn→spawn,
            // generation ≥ 1) her entity için `is_alive` kontrolünü geçemez →
            // `entity_component_types` boş döner ve TÜM dinamik bileşenler (Transform,
            // RigidBody, Collider…) sessizce kaybolurdu.
            let entity = ent;
            let types = world.entity_component_types(entity);
            for type_id in types {
                if let Some(reg) = registry.get_registration(type_id) {
                    if let Some(ptr) = world.get_component_ptr(entity, type_id) {
                        match crate::serde_bridge::serialize_component(registry, reg, type_id, ptr) {
                            Some(string_repr) => {
                                component_count += 1;
                                components.insert(reg.name.clone(), string_repr);
                            }
                            // serde_bridge already logged the concrete reason.
                            None => dropped_components += 1,
                        }
                    }
                }
            }

            // Yalnızca bir bileşeni olan entity'leri kaydet
            if name.is_some() || !components.is_empty() {
                entities.push(EntitySnapshot {
                    entity_id: id,
                    name,
                    components,
                });
            }
        }

        let snapshot = Self {
            entities,
            timestamp: Instant::now(),
        };
        if dropped_components > 0 {
            tracing::warn!(
                dropped_components,
                entity_count = snapshot.entities.len(),
                "[Scene] snapshot capture sırasında bazı bileşenler serialize edilemedi (Play→Stop'ta state kaybı riski)",
            );
        }
        // Play is a mode change → info! is the right level for the one-shot completion.
        tracing::info!(
            entity_count = snapshot.entities.len(),
            component_count,
            duration_ms = start.elapsed().as_secs_f64() * 1000.0,
            "[Scene] Snapshot alındı (Play)",
        );
        snapshot
    }

    /// Snapshot'tan geri yükleme yapar. Mevcut entity'leri siler (korunanlar hariç)
    /// ve snapshot'taki entity'leri yeniden oluşturur.
    ///
    /// **Not:** Mesh ve Material gibi GPU kaynaklarını restore etmek için
    /// tam sahne yüklemesi yapılmalıdır. Bu fonksiyon yalnızca fizik/transform
    /// bileşenlerini geri yükler.
    #[tracing::instrument(skip_all, name = "snapshot_restore")]
    pub fn restore(
        &self,
        world: &mut World,
        registry: &crate::registry::SceneRegistry,
        protected_ids: &std::collections::HashSet<u32>,
    ) -> RestoreResult {
        let start = Instant::now();
        let mut restored_count = 0u32;
        let mut despawned_count = 0u32;
        // A failed/unknown component here means Stop leaves the entity with less state than
        // it had at Play — worth surfacing, since the whole point of restore is fidelity.
        let mut failed_components = 0usize;
        let mut unknown_types = 0usize;

        let mut snap_ids = std::collections::HashSet::new();
        for snap_ent in &self.entities {
            snap_ids.insert(snap_ent.entity_id);
        }

        // 1. Korunmayan ve snapshot'ta OLMAYAN (Play sırasında oluşturulmuş) entity'leri sil
        let alive = world.iter_alive_entities();
        let mut to_despawn = Vec::new();

        {
            let names = world.borrow::<gizmo_core::EntityName>();
            for ent in &alive {
                let id = ent.id();
                if protected_ids.contains(&id) {
                    continue;
                }
                if let Some(name) = names.get(id) {
                    if name.0.starts_with("Editor ") || name.0 == "Highlight Box" {
                        continue;
                    }
                }
                
                if !snap_ids.contains(&id) {
                    to_despawn.push(*ent);
                }
            }
        }

        for ent in to_despawn {
            world.despawn(ent);
            despawned_count += 1;
        }

        // 2. Snapshot'taki entity'lerin bileşenlerini mevcutların üzerine yaz
        for snap_entity in &self.entities {
            // Eğer entity silinmişse yeni bir tane oluştur (Not: Mesh gibi GPU verileri kaybolur, ideal değil ama çökmez)
            let ent = if let Some(e) = world.get_entity(snap_entity.entity_id) {
                if world.is_alive(e) {
                    e
                } else {
                    world.spawn()
                }
            } else {
                world.spawn()
            };

            if let Some(ref name) = snap_entity.name {
                world.add_component(ent, gizmo_core::EntityName::new(name));
            }

            for (comp_name, comp_val) in &snap_entity.components {
                match registry.get_type_id(comp_name) {
                    Some(type_id) => {
                        if let Some(reg) = registry.get_registration(type_id) {
                            // Was `let _ = …`: a deserialize failure silently vanished, so a
                            // component the editor captured at Play never came back at Stop.
                            if let Err(e) = crate::serde_bridge::deserialize_component(
                                world, ent, registry, reg, type_id, comp_val,
                            ) {
                                failed_components += 1;
                                tracing::warn!(
                                    component = %comp_name,
                                    entity = snap_entity.entity_id,
                                    error = %e,
                                    "[Scene] snapshot restore: bileşen deserialize edilemedi (Stop'ta state eksik)",
                                );
                            }
                        } else {
                            unknown_types += 1;
                        }
                    }
                    None => unknown_types += 1,
                }
            }

            restored_count += 1;
        }

        let result = RestoreResult {
            despawned: despawned_count,
            restored: restored_count,
            duration: start.elapsed(),
        };
        if failed_components > 0 || unknown_types > 0 {
            tracing::warn!(
                failed_components,
                unknown_types,
                "[Scene] snapshot restore sırasında bazı bileşenler geri yüklenemedi (Stop sonrası state eksik olabilir)",
            );
        }
        // Stop is a mode change → info! for the one-shot completion.
        tracing::info!(
            despawned = result.despawned,
            restored = result.restored,
            duration_ms = result.duration.as_secs_f64() * 1000.0,
            "[Scene] Snapshot geri yüklendi (Stop)",
        );
        result
    }

    /// Snapshot'taki entity sayısı
    pub fn entity_count(&self) -> usize {
        self.entities.len()
    }

    /// Snapshot'ın alındığı zamandan bu yana geçen süre
    pub fn age(&self) -> std::time::Duration {
        self.timestamp.elapsed()
    }
}

/// Geri yükleme sonucu istatistikleri
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct RestoreResult {
    pub despawned: u32,
    pub restored: u32,
    pub duration: std::time::Duration,
}

impl std::fmt::Display for RestoreResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Restore: {} entity silindi, {} entity geri yüklendi ({:.2}ms)",
            self.despawned,
            self.restored,
            self.duration.as_secs_f64() * 1000.0,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scene_snapshot_capture_empty_world() {
        let world = World::new();
        let registry = crate::registry::SceneRegistry::new();
        let protected = std::collections::HashSet::new();

        let snapshot = SceneSnapshot::capture(&world, &registry, &protected);
        assert_eq!(snapshot.entity_count(), 0);
    }

    #[test]
    fn test_scene_snapshot_round_trip() {
        let mut world = World::new();
        let registry = crate::registry::default_scene_registry();
        let protected = std::collections::HashSet::new();

        // Entity oluştur
        let ent = world.spawn();
        world.add_component(ent, gizmo_core::EntityName::new("TestCube"));
        world.add_component(
            ent,
            gizmo_physics_core::Transform::new(gizmo_math::Vec3::new(1.0, 2.0, 3.0)),
        );

        // Snapshot al
        let snapshot = SceneSnapshot::capture(&world, &registry, &protected);
        assert_eq!(snapshot.entity_count(), 1);
        assert_eq!(snapshot.entities[0].name.as_deref(), Some("TestCube"));
        assert!(snapshot.entities[0].components.contains_key("Transform"));

        // Entity'yi sil
        world.despawn(ent);
        assert_eq!(world.iter_alive_entities().len(), 0);

        // Restore et
        let result = snapshot.restore(&mut world, &registry, &protected);
        assert_eq!(result.restored, 1);

        // İsmi kontrol et
        let names = world.borrow::<gizmo_core::EntityName>();
        let alive = world.iter_alive_entities();
        assert_eq!(alive.len(), 1);
        let restored_name = names.get(alive[0].id());
        assert!(restored_name.is_some());
        assert_eq!(restored_name.unwrap().0, "TestCube");
    }

    // REGRESYON (audit 2026-06-29): id slotu yeniden kullanılmış entity'lerin
    // (despawn→spawn, generation ≥ 1) dinamik bileşenleri capture'da kaybolmamalı.
    // Eski kod `Entity::new(id, 0)` ile lookup yaptığından `entity_component_types`'ın
    // generation kontrolü başarısız olur, boş döner ve Transform sessizce düşerdi
    // (editör Play→Stop akışında fizik/transform kaybı).
    #[test]
    fn capture_preserves_components_for_recycled_id_entity() {
        let mut world = World::new();
        let registry = crate::registry::default_scene_registry();
        let protected = std::collections::HashSet::new();

        // id 0'ı yak → sonraki spawn onu generation 1 ile geri kullanır.
        let burned = world.spawn();
        let burned_id = burned.id();
        world.despawn(burned);

        let ent = world.spawn();
        assert_eq!(ent.id(), burned_id, "ön koşul: id yeniden kullanılmalı");
        assert_ne!(ent.generation(), 0, "ön koşul: recycled entity generation ≥ 1");
        world.add_component(ent, gizmo_core::EntityName::new("Recycled"));
        world.add_component(
            ent,
            gizmo_physics_core::Transform::new(gizmo_math::Vec3::new(7.0, 8.0, 9.0)),
        );

        let snapshot = SceneSnapshot::capture(&world, &registry, &protected);
        assert_eq!(snapshot.entity_count(), 1);
        assert!(
            snapshot.entities[0].components.contains_key("Transform"),
            "recycled-id entity'nin Transform'ı capture'da DÜŞTÜ (generation-0 lookup bug)"
        );
    }

    // Capture must honor `protected_ids` (editor camera, gizmos, …): a protected entity is
    // never pulled into the Play snapshot, so Stop can't clobber/duplicate it.
    #[test]
    fn capture_skips_protected_ids() {
        let mut world = World::new();
        let registry = crate::registry::default_scene_registry();

        let keep = world.spawn();
        world.add_component(keep, gizmo_core::EntityName::new("EditorCam"));
        let normal = world.spawn();
        world.add_component(normal, gizmo_core::EntityName::new("Cube"));

        let mut protected = std::collections::HashSet::new();
        protected.insert(keep.id());

        let snapshot = SceneSnapshot::capture(&world, &registry, &protected);
        assert_eq!(snapshot.entity_count(), 1, "only the unprotected entity is captured");
        assert_eq!(snapshot.entities[0].name.as_deref(), Some("Cube"));
    }

    // Capture must also filter the editor's name-tagged scaffolding ("Editor …",
    // "Highlight Box") even when they aren't in the explicit protected set.
    #[test]
    fn capture_skips_editor_named_scaffolding() {
        let mut world = World::new();
        let registry = crate::registry::default_scene_registry();
        let protected = std::collections::HashSet::new();

        let ed = world.spawn();
        world.add_component(ed, gizmo_core::EntityName::new("Editor Grid"));
        let hl = world.spawn();
        world.add_component(hl, gizmo_core::EntityName::new("Highlight Box"));
        let real = world.spawn();
        world.add_component(real, gizmo_core::EntityName::new("Enemy"));

        let snapshot = SceneSnapshot::capture(&world, &registry, &protected);
        let names: Vec<_> = snapshot.entities.iter().filter_map(|e| e.name.clone()).collect();
        assert_eq!(names, vec!["Enemy".to_string()], "only the real entity is snapshotted");
    }

    // The full Play→Stop contract: `restore` must (a) DESPAWN entities spawned during play
    // that aren't in the snapshot, and (b) preserve the snapshot entities. The `despawned`
    // count is the load-bearing statistic the editor reports.
    #[test]
    fn restore_despawns_play_time_entities_not_in_snapshot() {
        let mut world = World::new();
        let registry = crate::registry::default_scene_registry();
        let protected = std::collections::HashSet::new();

        let original = world.spawn();
        world.add_component(original, gizmo_core::EntityName::new("Original"));

        // Snapshot the pre-Play state (only `Original` exists).
        let snapshot = SceneSnapshot::capture(&world, &registry, &protected);
        assert_eq!(snapshot.entity_count(), 1);

        // Simulate Play: a bullet is spawned at runtime.
        let bullet = world.spawn();
        world.add_component(bullet, gizmo_core::EntityName::new("Bullet"));
        assert_eq!(world.iter_alive_entities().len(), 2);

        // Stop: restore must remove the play-time bullet, keep Original.
        let result = snapshot.restore(&mut world, &registry, &protected);
        assert_eq!(result.despawned, 1, "the play-time bullet must be despawned");
        assert_eq!(result.restored, 1, "the one snapshot entity must be restored");

        let names = world.borrow::<gizmo_core::EntityName>();
        let alive_names: Vec<_> = world
            .iter_alive_entities()
            .into_iter()
            .filter_map(|e| names.get(e.id()).map(|n| n.0.clone()))
            .collect();
        assert!(alive_names.contains(&"Original".to_string()));
        assert!(!alive_names.contains(&"Bullet".to_string()), "bullet must be gone after Stop");
    }

    // Restore must reinstate component VALUES, not just names — the whole point of the
    // in-memory snapshot is that physics/transform state (position, …) is exactly recovered
    // on Stop. Round-trips the value through the serde bridge (capture → despawn → restore).
    #[test]
    fn restore_reinstates_component_values() {
        use gizmo_math::Vec3;
        use gizmo_physics_core::Transform;

        let mut world = World::new();
        let registry = crate::registry::default_scene_registry();
        let protected = std::collections::HashSet::new();

        let ent = world.spawn();
        world.add_component(ent, gizmo_core::EntityName::new("Mover"));
        world.add_component(ent, Transform::new(Vec3::new(3.0, -7.0, 11.0)));

        let snapshot = SceneSnapshot::capture(&world, &registry, &protected);

        // Play destroyed the entity; Stop must bring it back WITH its transform value.
        world.despawn(ent);
        snapshot.restore(&mut world, &registry, &protected);

        let names = world.borrow::<gizmo_core::EntityName>();
        let id = world
            .iter_alive_entities()
            .into_iter()
            .map(|e| e.id())
            .find(|&id| names.get(id).map(|n| n.0.as_str()) == Some("Mover"))
            .expect("Mover restore edilmeli");
        let transforms = world.borrow::<Transform>();
        assert_eq!(
            transforms.get(id).map(|t| t.position),
            Some(Vec3::new(3.0, -7.0, 11.0)),
            "restore must recover the snapshotted transform value, not just the name"
        );
    }

    // `RestoreResult`'s Display is surfaced in the editor status line; its formatting
    // (counts + milliseconds to 2 dp) is a stable contract.
    #[test]
    fn restore_result_display_formats_counts_and_millis() {
        let result = RestoreResult {
            despawned: 2,
            restored: 3,
            duration: std::time::Duration::from_millis(5),
        };
        let s = result.to_string();
        assert!(s.contains("2 entity silindi"), "despawn count missing: {s}");
        assert!(s.contains("3 entity geri yüklendi"), "restore count missing: {s}");
        // 5 ms → "5.00ms" (secs_f64 * 1000, {:.2}).
        assert!(s.contains("5.00ms"), "duration ms formatting wrong: {s}");
    }

    // A snapshot of an entity carrying ONLY a name (no registered components) is still
    // captured and restored — name-only entities (empty groups/markers) must not vanish.
    #[test]
    fn name_only_entity_is_captured_and_restored() {
        let mut world = World::new();
        let registry = crate::registry::default_scene_registry();
        let protected = std::collections::HashSet::new();

        let marker = world.spawn();
        world.add_component(marker, gizmo_core::EntityName::new("SpawnMarker"));

        let snapshot = SceneSnapshot::capture(&world, &registry, &protected);
        assert_eq!(snapshot.entity_count(), 1);
        assert!(
            snapshot.entities[0].components.is_empty(),
            "no registered components expected on a bare marker"
        );

        world.despawn(marker);
        snapshot.restore(&mut world, &registry, &protected);

        let names = world.borrow::<gizmo_core::EntityName>();
        assert!(
            world
                .iter_alive_entities()
                .into_iter()
                .any(|e| names.get(e.id()).map(|n| n.0.as_str()) == Some("SpawnMarker")),
            "name-only entity must be recreated on restore"
        );
    }
}
