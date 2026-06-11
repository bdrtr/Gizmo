use gizmo_core::{Entity, World};
use gizmo_math::{Quat, Vec3};
use serde::{Deserialize, Serialize};

/// Tek bir objenin fiziki durumu (hızlı kopyalanabilir ve ağdan gönderilebilir)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EntityState {
    pub entity: Entity,
    pub position: Vec3,
    pub rotation: Quat,
    pub linear_velocity: Vec3,
    pub angular_velocity: Vec3,
    pub is_sleeping: bool,
}

/// Tüm dünyadaki fizik objelerinin anlık yedeği
#[derive(Clone, Default, Debug, Serialize, Deserialize)]
pub struct PhysicsStateSnapshot {
    pub tick: u64,
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

        let transforms = world.borrow_mut::<Transform>();
        let velocities = world.borrow_mut::<Velocity>();
        let rigid_bodies = world.borrow_mut::<RigidBody>();

        for state in &self.states {
            let id = state.entity.id();

            if let Some(mut t) = transforms.get_mut(id) {
                t.position = state.position;
                t.rotation = state.rotation;
            }
            if let Some(mut v) = velocities.get_mut(id) {
                v.linear = state.linear_velocity;
                v.angular = state.angular_velocity;
            }
            if let Some(mut rb) = rigid_bodies.get_mut(id) {
                rb.is_sleeping = state.is_sleeping;
            }
        }
    }
}

/// Dairesel Tampon (Ring Buffer), geçmiş N kareyi tutar
pub struct RollbackBuffer {
    buffer: Vec<Option<PhysicsStateSnapshot>>,
    capacity: usize,
}

impl RollbackBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: vec![None; capacity],
            capacity,
        }
    }

    pub fn save(&mut self, snapshot: PhysicsStateSnapshot) {
        let index = (snapshot.tick as usize) % self.capacity;
        self.buffer[index] = Some(snapshot);
    }

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
