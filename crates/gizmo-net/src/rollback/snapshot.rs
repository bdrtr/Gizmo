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
    #[tracing::instrument(skip_all, name = "snapshot_capture")]
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

        // Per-frame sıcak yol → trace seviyesi; alan hesabı yalnızca Vec::len() (bedava).
        tracing::trace!(tick, entity_count = states.len(), "Fizik snapshot'ı yakalandı");
        Self { tick, states }
    }

    /// Snapshot'u mevcut dünyaya anında geri yükler (Restore / Rollback)
    #[tracing::instrument(skip_all, name = "snapshot_restore")]
    pub fn restore(&self, world: &mut World) {
        use gizmo_physics_core::components::transform::Transform;
        use gizmo_physics_rigid::components::velocity::Velocity;
        use gizmo_physics_rigid::components::rigid_body::RigidBody;

        // SAFETY: exclusive `&mut World`; Transform/Velocity/RigidBody are distinct component
        // types, so these three mutable queries never alias the same storage.
        let mut transforms = unsafe { world.borrow_mut_unchecked::<Transform>() };
        let mut velocities = unsafe { world.borrow_mut_unchecked::<Velocity>() };
        let mut rigid_bodies = unsafe { world.borrow_mut_unchecked::<RigidBody>() };

        // Bir state'in en az bir bileşeni canlı entity'ye uygulandı mı — aksi halde
        // (despawn/id-yeniden-kullanım, generation uyuşmazlığı) atlanmış demektir.
        let mut skipped = 0usize;

        for state in &self.states {
            // Ham `id` yerine `get_mut_entity(state.entity)` kullan: bu, uygulamadan
            // ÖNCE `World::is_alive` ile generation'ı doğrular. Aksi halde bir entity
            // despawn edilip aynı id slot'u yeni bir entity'ye (gen++) yeniden verildiğinde,
            // despawn ÖNCESİ bir snapshot'ı geri yüklemek yeni entity'nin durumunu sessizce
            // eski verilerle bozardı. Generation uyuşmazsa bu state atlanır.
            let mut applied_any = false;
            if let Some(mut t) = transforms.get_mut_entity(state.entity) {
                t.position = state.position;
                t.rotation = state.rotation;
                applied_any = true;
            }
            if let Some(mut v) = velocities.get_mut_entity(state.entity) {
                v.linear = state.linear_velocity;
                v.angular = state.angular_velocity;
                applied_any = true;
            }
            if let Some(mut rb) = rigid_bodies.get_mut_entity(state.entity) {
                rb.is_sleeping = state.is_sleeping;
                applied_any = true;
            }
            if !applied_any {
                skipped += 1;
            }
        }

        // Atlama beklenen (kasıtlı) bir durum, o yüzden yalnızca gerçekleştiğinde ve
        // debug seviyesinde toplu (aggregate) raporla — per-entity gürültü YOK.
        if skipped > 0 {
            tracing::debug!(
                tick = self.tick,
                total = self.states.len(),
                skipped,
                "Snapshot restore: bazı entity'ler atlandı (despawn/id-yeniden-kullanım, generation uyuşmazlığı)"
            );
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
        if capacity == 0 {
            tracing::warn!("RollbackBuffer kapasitesi 0 verildi, 1'e normalize edildi (çağıran hatası)");
        }
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

    // --- RollbackBuffer (saf halka-tampon mantığı) ---

    fn snap(tick: u64) -> PhysicsStateSnapshot {
        PhysicsStateSnapshot { tick, states: Vec::new() }
    }

    #[test]
    fn rollback_buffer_save_then_get_roundtrips() {
        let mut buf = RollbackBuffer::new(8);
        buf.save(snap(5));
        assert_eq!(buf.get(5).map(|s| s.tick), Some(5));
    }

    #[test]
    fn rollback_buffer_get_absent_tick_is_none() {
        let buf = RollbackBuffer::new(8);
        assert!(buf.get(3).is_none(), "hiç kaydedilmeyen tick None olmalı");
    }

    // capacity=4 → tick 0 ve tick 4 AYNI slot'u paylaşır. Yeni tick eskiyi ezer ve
    // eski tick artık slot etiketiyle uyuşmadığı için get() None döner (bayat okuma yok).
    #[test]
    fn rollback_buffer_slot_collision_evicts_old_and_reports_none() {
        let mut buf = RollbackBuffer::new(4);
        buf.save(snap(0));
        buf.save(snap(4));
        assert!(buf.get(0).is_none(), "ezilen eski tick None olmalı");
        assert_eq!(buf.get(4).map(|s| s.tick), Some(4));
    }

    #[test]
    fn rollback_buffer_capacity_zero_is_normalized_and_never_panics() {
        let mut buf = RollbackBuffer::new(0); // en az 1'e normalize edilir
        buf.save(snap(0));
        assert_eq!(buf.get(0).map(|s| s.tick), Some(0));
        let _ = buf.get(123_456); // modulo-by-zero koruması → panik yok
    }

    // FullState paketi bu snapshot'ı ağdan yollar → serde tur-gidişi state'leri korumalı.
    #[test]
    fn physics_snapshot_serde_roundtrip_preserves_states() {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Transform::new(Vec3::new(1.5, -2.0, 3.25)));
        world.add_component(e, Velocity::new(Vec3::new(7.0, 8.0, 9.0)));
        let original = PhysicsStateSnapshot::capture(&world, 99);

        let bytes = bincode::serialize(&original).unwrap();
        let back: PhysicsStateSnapshot = bincode::deserialize(&bytes).unwrap();

        assert_eq!(back.tick, 99);
        assert_eq!(back.states.len(), 1);
        assert_eq!(back.states[0].position, Vec3::new(1.5, -2.0, 3.25));
        assert_eq!(back.states[0].linear_velocity, Vec3::new(7.0, 8.0, 9.0));
        assert_eq!(back.states[0].entity, e);
    }

    // capture yalnız Transform VE Velocity'si olan (hareketli) objeleri yakalar.
    #[test]
    fn capture_excludes_entities_missing_velocity() {
        let mut world = World::new();
        let only_t = world.spawn();
        world.add_component(only_t, Transform::new(Vec3::ZERO)); // Velocity yok → hariç
        let moving = world.spawn();
        world.add_component(moving, Transform::new(Vec3::new(1.0, 0.0, 0.0)));
        world.add_component(moving, Velocity::new(Vec3::new(2.0, 0.0, 0.0)));

        let snap = PhysicsStateSnapshot::capture(&world, 0);
        assert_eq!(snap.states.len(), 1, "yalnız Transform+Velocity yakalanmalı");
        assert_eq!(snap.states[0].entity, moving);
    }

    // restore yalnız pozisyonu değil hızı ve uyku bayrağını da geri yüklemeli.
    #[test]
    fn restore_recovers_velocity_and_sleep_flag_not_just_position() {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Transform::new(Vec3::new(1.0, 2.0, 3.0)));
        world.add_component(e, Velocity::new(Vec3::new(4.0, 5.0, 6.0)));
        world.add_component(e, RigidBody::new(1.0, true)); // is_sleeping = false

        // Uyku bayrağını kur, sonra snapshot al.
        {
            let mut rbs = world.borrow_mut::<RigidBody>();
            let mut rb = rbs.get_mut(e.id()).unwrap();
            rb.is_sleeping = true;
        }
        let snap = PhysicsStateSnapshot::capture(&world, 0);

        // Her şeyi boz.
        {
            let mut ts = world.borrow_mut::<Transform>();
            let mut t = ts.get_mut(e.id()).unwrap();
            t.position = Vec3::new(-9.0, -9.0, -9.0);
        }
        {
            let mut vs = world.borrow_mut::<Velocity>();
            let mut v = vs.get_mut(e.id()).unwrap();
            v.linear = Vec3::ZERO;
        }
        {
            let mut rbs = world.borrow_mut::<RigidBody>();
            let mut rb = rbs.get_mut(e.id()).unwrap();
            rb.is_sleeping = false;
        }

        snap.restore(&mut world);

        {
            let ts = world.borrow::<Transform>();
            assert_eq!(
                ts.get(e.id()).unwrap().position,
                Vec3::new(1.0, 2.0, 3.0),
                "pozisyon geri gelmeli"
            );
        }
        {
            let vs = world.borrow::<Velocity>();
            assert_eq!(
                vs.get(e.id()).unwrap().linear,
                Vec3::new(4.0, 5.0, 6.0),
                "hız geri gelmeli"
            );
        }
        {
            let rbs = world.borrow::<RigidBody>();
            assert!(rbs.get(e.id()).unwrap().is_sleeping, "uyku bayrağı geri gelmeli");
        }
    }
}
