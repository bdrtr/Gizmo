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

/// Tek bir entity'nin in-memory yedeği
#[derive(Clone)]
pub struct EntitySnapshot {
    pub entity_id: u32,
    pub name: Option<String>,
    /// RON formatında dinamik bileşen snapshot'ı
    pub components: std::collections::BTreeMap<String, ron::Value>,
}

/// Tüm sahnenin in-memory snapshot'ı
#[derive(Clone)]
pub struct SceneSnapshot {
    pub entities: Vec<EntitySnapshot>,
    pub timestamp: std::time::Instant,
}

impl SceneSnapshot {
    /// Mevcut World durumunu in-memory snapshot olarak yakalar.
    /// `protected_ids`: Editor kamerası, highlight box gibi korunacak entity'ler
    pub fn capture(
        world: &World,
        registry: &crate::registry::SceneRegistry,
        protected_ids: &std::collections::HashSet<u32>,
    ) -> Self {
        let mut entities = Vec::new();
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
            for comp_name in registry.all_components() {
                if let Some(serializer) = registry.get_serializer(comp_name) {
                    if let Some(comp_value) = serializer(world, id) {
                        components.insert(comp_name.clone(), comp_value);
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

        Self {
            entities,
            timestamp: std::time::Instant::now(),
        }
    }

    /// Snapshot'tan geri yükleme yapar. Mevcut entity'leri siler (korunanlar hariç)
    /// ve snapshot'taki entity'leri yeniden oluşturur.
    ///
    /// **Not:** Mesh ve Material gibi GPU kaynaklarını restore etmek için
    /// tam sahne yüklemesi yapılmalıdır. Bu fonksiyon yalnızca fizik/transform
    /// bileşenlerini geri yükler.
    pub fn restore(
        &self,
        world: &mut World,
        registry: &crate::registry::SceneRegistry,
        protected_ids: &std::collections::HashSet<u32>,
    ) -> RestoreResult {
        let start = std::time::Instant::now();
        let mut restored_count = 0u32;
        let mut despawned_count = 0u32;

        // 1. Korunmayan tüm entity'leri sil
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
                to_despawn.push(*ent);
            }
        }

        for ent in to_despawn {
            world.despawn(ent);
            despawned_count += 1;
        }

        // 2. Snapshot'taki entity'leri yeniden oluştur
        for snap_entity in &self.entities {
            let ent = world.spawn();

            // İsmi geri yükle
            if let Some(ref name) = snap_entity.name {
                world.add_component(ent, gizmo_core::EntityName::new(name));
            }

            // Dinamik bileşenleri registry üzerinden geri yükle
            for (comp_name, comp_val) in &snap_entity.components {
                if let Some(deserializer) = registry.get_deserializer(comp_name) {
                    deserializer(world, ent.id(), comp_val);
                }
            }

            restored_count += 1;
        }

        RestoreResult {
            despawned: despawned_count,
            restored: restored_count,
            duration: start.elapsed(),
        }
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
#[derive(Debug, Clone)]
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
        let registry = crate::registry::SceneRegistry::with_core_components();
        let protected = std::collections::HashSet::new();

        // Entity oluştur
        let ent = world.spawn();
        world.add_component(ent, gizmo_core::EntityName::new("TestCube"));
        world.add_component(
            ent,
            gizmo_physics::components::Transform::new(
                gizmo_math::Vec3::new(1.0, 2.0, 3.0),
            ),
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
}
