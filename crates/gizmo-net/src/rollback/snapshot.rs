use gizmo_core::{Entity, World};
use gizmo_math::{Quat, Vec3};
use serde::{Deserialize, Serialize};

/// Tek bir objenin fiziki durumu (hızlı kopyalanabilir ve ağdan gönderilebilir)
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct EntityState {
    /// The entity this snapshot describes.
    pub entity: Entity,
    /// World-space position.
    pub position: Vec3,
    /// World-space orientation.
    pub rotation: Quat,
    /// Linear velocity.
    pub linear_velocity: Vec3,
    /// Angular velocity.
    pub angular_velocity: Vec3,
    /// Whether the rigid body was asleep (skipped by the solver) at capture time.
    pub is_sleeping: bool,
}

impl Default for EntityState {
    fn default() -> Self {
        Self {
            entity: Entity::INVALID,
            position: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            linear_velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            is_sleeping: false,
        }
    }
}

/// Tüm dünyadaki fizik objelerinin anlık yedeği
#[derive(Clone, Default, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PhysicsStateSnapshot {
    /// Simulation tick this snapshot was captured at.
    pub tick: u64,
    /// Per-entity physics state at this tick.
    pub states: Vec<EntityState>,
    // İleride ArticulatedTree (Multibody) state'leri de eklenebilir
}

impl PhysicsStateSnapshot {
    /// O(N) hızında belleğe kopyalama işlemi yapar
    pub fn capture(world: &World, tick: u64) -> Self {
        use gizmo_physics_core::components::transform::Transform;
        use gizmo_physics_rigid::components::velocity::Velocity;
        use gizmo_physics_rigid::components::rigid_body::RigidBody;

        let transforms = world.borrow::<Transform>();
        let velocities = world.borrow::<Velocity>();
        let rigid_bodies = world.borrow::<RigidBody>();

        let mut states = Vec::with_capacity(128); // Tahmini

        for ent in world.iter_alive_entities() {
            let id = ent.id();
            
            // Sadece Transform ve Velocity'si olan (hareketli) objeleri snapshot al
            if let (Some(t), Some(v)) = (transforms.get(id), velocities.get(id)) {
                let is_sleeping = rigid_bodies.get(id).is_some_and(|rb| rb.is_sleeping);

                states.push(EntityState {
                    entity: ent,
                    position: t.position,
                    rotation: t.rotation,
                    linear_velocity: v.linear,
                    angular_velocity: v.angular,
                    is_sleeping,
                });
            }
        }

        Self { tick, states }
    }

    /// Snapshot'u mevcut dünyaya anında geri yükler (Restore / Rollback)
    pub fn restore(&self, world: &mut World) {
        use gizmo_physics_core::components::transform::Transform;
        use gizmo_physics_rigid::components::velocity::Velocity;
        use gizmo_physics_rigid::components::rigid_body::RigidBody;

        // SAFETY: exclusive `&mut World`; Transform/Velocity/RigidBody are distinct component
        // types, so these three mutable queries never alias the same storage.
        let mut transforms = unsafe { world.borrow_mut_unchecked::<Transform>() };
        let mut velocities = unsafe { world.borrow_mut_unchecked::<Velocity>() };
        let mut rigid_bodies = unsafe { world.borrow_mut_unchecked::<RigidBody>() };

        for state in &self.states {
            // Ham `id` yerine `get_mut_entity(state.entity)` kullan: bu, uygulamadan
            // ÖNCE `World::is_alive` ile generation'ı doğrular. Aksi halde bir entity
            // despawn edilip aynı id slot'u yeni bir entity'ye (gen++) yeniden verildiğinde,
            // despawn ÖNCESİ bir snapshot'ı geri yüklemek yeni entity'nin durumunu sessizce
            // eski verilerle bozardı. Generation uyuşmazsa bu state atlanır.
            if let Some(mut t) = transforms.get_mut_entity(state.entity) {
                t.position = state.position;
                t.rotation = state.rotation;
            }
            if let Some(mut v) = velocities.get_mut_entity(state.entity) {
                v.linear = state.linear_velocity;
                v.angular = state.angular_velocity;
            }
            if let Some(mut rb) = rigid_bodies.get_mut_entity(state.entity) {
                rb.is_sleeping = state.is_sleeping;
            }
        }
    }
}

/// Dairesel Tampon (Ring Buffer), geçmiş N kareyi tutar
#[derive(Debug, Clone)]
pub struct RollbackBuffer {
    buffer: Vec<Option<PhysicsStateSnapshot>>,
    capacity: usize,
}

impl RollbackBuffer {
    /// Creates a ring buffer holding the last `capacity` snapshots.
    pub fn new(capacity: usize) -> Self {
        // Modulo-by-zero koruması: indeksleme `% self.capacity` kullandığından
        // capacity=0 ilk save/get'te panik üretir. İmza değiştirmeden en az 1
        // kapasiteye normalize ediyoruz (başarı yolu etkilenmez).
        let capacity = capacity.max(1);
        Self {
            buffer: vec![None; capacity],
            capacity,
        }
    }

    /// Stores a snapshot in its tick slot (overwriting any older entry there).
    pub fn save(&mut self, snapshot: PhysicsStateSnapshot) {
        let index = (snapshot.tick as usize) % self.capacity;
        self.buffer[index] = Some(snapshot);
    }

    /// Returns the snapshot for `tick` if it is still present (not yet overwritten).
    pub fn get(&self, tick: u64) -> Option<&PhysicsStateSnapshot> {
        let index = (tick as usize) % self.capacity;
        if let Some(snap) = &self.buffer[index] {
            if snap.tick == tick {
                return Some(snap);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_physics_core::components::transform::Transform;
    use gizmo_physics_rigid::components::rigid_body::RigidBody;
    use gizmo_physics_rigid::components::velocity::Velocity;

    // REGRESYON (bulgu 20): despawn + id-yeniden-kullanım sonrası bir snapshot geri
    // yüklendiğinde, eski state YENİ entity'yi bozmamalı. restore() generation
    // doğrulaması yapmazsa (ham id ile get_mut), aynı id slot'unu paylaşan yeni entity
    // eski verilerle ezilir.
    #[test]
    fn restore_skips_stale_entity_after_id_reuse() {
        let mut world = World::new();

        // 1) Entity E'yi spawn et ve fizik durumu ver.
        let e = world.spawn();
        world.add_component(e, Transform::new(Vec3::new(1.0, 2.0, 3.0)));
        world.add_component(e, Velocity::new(Vec3::new(9.0, 9.0, 9.0)));
        world.add_component(e, RigidBody::new(1.0, true));

        // 2) Snapshot al (eski durumu yakalar).
        let snap = PhysicsStateSnapshot::capture(&world, 0);
        assert_eq!(snap.states.len(), 1);

        // 3) E'yi despawn et → id slot'u serbest kalır, generation artar.
        world.despawn(e);

        // 4) Aynı id slot'una YENİ bir entity düşür (gen farklı) ve TAZE durum ver.
        let e2 = world.spawn();
        assert_eq!(e2.id(), e.id(), "test id slot'unun yeniden kullanılmasına dayanır");
        assert_ne!(e2, e, "yeni entity'nin generation'ı farklı olmalı");
        world.add_component(e2, Transform::new(Vec3::new(-5.0, -5.0, -5.0)));
        world.add_component(e2, Velocity::new(Vec3::new(0.0, 0.0, 0.0)));

        // 5) Eski snapshot'ı geri yükle → yeni entity BOZULMAMALI.
        snap.restore(&mut world);

        let transforms = world.borrow::<Transform>();
        let t = transforms.get(e2.id()).expect("yeni entity'nin transform'u olmalı");
        assert_eq!(
            t.position,
            Vec3::new(-5.0, -5.0, -5.0),
            "eski snapshot yeni entity'yi bozdu (generation kontrolü yok)"
        );
    }

    // Kontrol: aynı (canlı) entity için restore gerçekten eski durumu geri yükler.
    #[test]
    fn restore_applies_to_live_entity() {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Transform::new(Vec3::new(1.0, 2.0, 3.0)));
        world.add_component(e, Velocity::new(Vec3::new(4.0, 5.0, 6.0)));

        let snap = PhysicsStateSnapshot::capture(&world, 0);

        // Durumu değiştir.
        {
            let mut transforms = world.borrow_mut::<Transform>();
            let mut t = transforms.get_mut(e.id()).unwrap();
            t.position = Vec3::new(100.0, 100.0, 100.0);
        }

        snap.restore(&mut world);

        let transforms = world.borrow::<Transform>();
        let t = transforms.get(e.id()).unwrap();
        assert_eq!(t.position, Vec3::new(1.0, 2.0, 3.0));
    }
}
