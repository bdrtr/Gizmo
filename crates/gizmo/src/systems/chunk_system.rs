use crate::core::World;
use gizmo_math::Vec3;
use std::collections::{HashMap, HashSet};

/// The side length of one streaming chunk, in metres.
pub const CHUNK_SIZE: f32 = 100.0;
/// How many chunks are kept loaded around the player, measured in chunks: 2 gives the 5×5 grid
/// centred on them.
pub const CHUNK_LOAD_RADIUS: i32 = 2; // Oyuncunun etrafındaki 5x5'lik grid yüklenir

/// A chunk's grid coordinate, `(x, z)`. Chunk `(0, 0)` is the one containing the world origin.
pub type ChunkCoord = (i32, i32);

/// Component that holds which Entity belongs to which Chunk
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChunkEntity {
    /// The chunk this entity belongs to; unloading that chunk despawns it.
    pub coord: ChunkCoord,
}

/// System Resource that manages all map loading/deletion operations
#[derive(Debug, Clone)]
pub struct ChunkManager {
    /// Coordinates of the Chunks loaded in RAM at that moment
    pub active_chunks: HashSet<ChunkCoord>,
    /// Which Entities are in which Chunk (we will use this for deleting)
    pub chunk_entities: HashMap<ChunkCoord, Vec<u64>>,
    /// The player's position in the previous frame (Only to do work when the Chunk changes)
    pub last_player_chunk: ChunkCoord,
}

impl Default for ChunkManager {
    fn default() -> Self {
        Self {
            active_chunks: HashSet::new(),
            chunk_entities: HashMap::new(),
            last_player_chunk: (i32::MAX, i32::MAX), // Başlangıçta tetiklenmesi için imkansız bir değer
        }
    }
}

impl ChunkManager {
    /// The chunk a world position falls in. Y is ignored: chunks are columns.
    pub fn world_pos_to_chunk(pos: Vec3) -> ChunkCoord {
        (
            (pos.x / CHUNK_SIZE).floor() as i32,
            (pos.z / CHUNK_SIZE).floor() as i32,
        )
    }

    /// The world position of a chunk's centre, at y = 0.
    pub fn chunk_to_world_pos(coord: ChunkCoord) -> Vec3 {
        Vec3::new(
            (coord.0 as f32) * CHUNK_SIZE + (CHUNK_SIZE / 2.0),
            0.0,
            (coord.1 as f32) * CHUNK_SIZE + (CHUNK_SIZE / 2.0),
        )
    }

    /// Registers an Entity into the Chunk system when it is created
    pub fn register_entity(&mut self, coord: ChunkCoord, entity_id: u64) {
        self.chunk_entities
            .entry(coord)
            .or_default()
            .push(entity_id);
    }
}

/// A dynamic body held still by the chunk system because the chunk under it is not loaded — see
/// [`open_world_chunk_system`]. Carries the body type it had, which it gets back when its chunk
/// loads again.
#[cfg(feature = "physics")]
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ChunkFrozen {
    /// What the body was before it was held.
    pub body_type: gizmo_physics_rigid::components::BodyType,
}
#[cfg(feature = "physics")]
gizmo_core::impl_component!(ChunkFrozen);

/// This system is called every frame. It checks the player's position.
/// If the player crossed a new Chunk boundary, it deletes the old Chunks and loads the new ones.
/// The `load_callback` function: lets the user spawn objects for every newly loaded Chunk.
///
/// **A dynamic body over a chunk that is not loaded is held still** (with the `physics` feature):
/// made kinematic at rest, marked [`ChunkFrozen`], and given back its body type when its chunk is
/// loaded again. The ground under it is a chunk's — unloading the chunk took it away — while the
/// body itself belongs to no chunk (a car the player left, a crate that rolled off the edge), so
/// it fell, and kept falling: measured in `dunya`, a car parked at the start 6.9 km below the
/// world two minutes after its owner walked off. Bodies that a chunk owns are despawned with it
/// and never get here. Checked every frame, not only when the player changes chunk: a body can
/// roll out of the loaded square on its own.
pub fn open_world_chunk_system<F, U>(
    world: &mut World,
    player_pos: Vec3,
    mut load_callback: F,
    mut unload_callback: U,
) where
    F: FnMut(&mut World, ChunkCoord),
    U: FnMut(&mut World, ChunkCoord, Vec<u64>),
{
    // ChunkManager resource'unu al (Yoksa oluştur)
    if world.get_resource::<ChunkManager>().is_none() {
        world.insert_resource(ChunkManager::default());
    }

    let current_chunk = ChunkManager::world_pos_to_chunk(player_pos);
    let mut chunks_to_load = Vec::new();
    let mut chunks_to_unload = Vec::new();

    // Sadece Chunk değiştiyse işlem yap (Performans optimizasyonu)
    {
        let mut manager = world.get_resource_mut::<ChunkManager>().unwrap();
        if manager.last_player_chunk == current_chunk {
            drop(manager);
            #[cfg(feature = "physics")]
            hold_bodies_over_unloaded_chunks(world);
            return; // Hala aynı bölgedeyiz, bir şey yapmaya gerek yok
        }
        manager.last_player_chunk = current_chunk;

        let mut expected_chunks = HashSet::new();

        // Oyuncunun etrafındaki 5x5 gridi hesapla
        for x in -CHUNK_LOAD_RADIUS..=CHUNK_LOAD_RADIUS {
            for z in -CHUNK_LOAD_RADIUS..=CHUNK_LOAD_RADIUS {
                expected_chunks.insert((current_chunk.0 + x, current_chunk.1 + z));
            }
        }

        // Hangi Chunk'lar silinecek? (Şu an aktif olan ama expected içinde olmayanlar)
        for &chunk in &manager.active_chunks {
            if !expected_chunks.contains(&chunk) {
                chunks_to_unload.push(chunk);
            }
        }

        // Hangi Chunk'lar yüklenecek? (Expected içinde olup aktif olmayanlar)
        for &chunk in &expected_chunks {
            if !manager.active_chunks.contains(&chunk) {
                chunks_to_load.push(chunk);
            }
        }
    } // manager borrow biter

    // --- SİLME İŞLEMİ (UNLOAD) ---
    for chunk in chunks_to_unload {
        let mut manager = world.get_resource_mut::<ChunkManager>().unwrap();
        manager.active_chunks.remove(&chunk);

        if let Some(entities) = manager.chunk_entities.remove(&chunk) {
            // Callback çağrılacak
            drop(manager);
            unload_callback(world, chunk, entities);
        }
        tracing::info!("🗑️ Chunk Yüklemesi Kaldırıldı (Havuzlandı): {:?}", chunk);
    }

    // --- YÜKLEME İŞLEMİ (LOAD) ---
    for chunk in chunks_to_load {
        {
            let mut manager = world.get_resource_mut::<ChunkManager>().unwrap();
            manager.active_chunks.insert(chunk);
        }
        tracing::info!("🌍 Yeni Chunk Yüklendi: {:?}", chunk);

        // Kullanıcının sağladığı (Örn: rpg_demo.rs içindeki) ağaç oluşturma fonksiyonunu çağırıyoruz
        load_callback(world, chunk);
    }
    #[cfg(feature = "physics")]
    hold_bodies_over_unloaded_chunks(world);
}

/// Holds every dynamic body whose chunk is not loaded, and lets go of every held one whose chunk
/// is — see [`open_world_chunk_system`].
#[cfg(feature = "physics")]
fn hold_bodies_over_unloaded_chunks(world: &mut World) {
    use gizmo_physics_core::components::Transform;
    use gizmo_physics_rigid::components::{BodyType, RigidBody, Velocity};
    let Some(active) = world.get_resource::<ChunkManager>().map(|m| m.active_chunks.clone()) else { return };
    if active.is_empty() {
        return;
    }
    let loaded = |p: Vec3| active.contains(&ChunkManager::world_pos_to_chunk(p));
    let (hold, free): (Vec<_>, Vec<_>) = {
        let frozen = world.query::<&ChunkFrozen>();
        let mut hold = Vec::new();
        let mut free = Vec::new();
        for (e, body, t) in world.query::<(gizmo_core::Entity, &RigidBody, &Transform)>().iter() {
            match frozen.get(e) {
                None if body.is_dynamic() && !loaded(t.position) => hold.push((e, Some(body.body_type))),
                // Still held: kept at rest, whatever wrote a velocity into it since (a kinematic
                // body moves by its velocity).
                Some(_) if !loaded(t.position) => hold.push((e, None)),
                Some(f) => free.push((e, f.body_type)),
                _ => {}
            }
        }
        (hold, free)
    };
    for (e, newly) in hold {
        if let Some(body_type) = newly {
            world.add_component(e, ChunkFrozen { body_type });
            if let Some(mut body) = world.query_mut::<&mut RigidBody>().get_mut(e) {
                body.body_type = BodyType::Kinematic;
            }
        }
        if let Some(mut v) = world.query_mut::<&mut Velocity>().get_mut(e) {
            v.linear = Vec3::ZERO;
            v.angular = Vec3::ZERO;
        }
    }
    for (e, body_type) in free {
        world.remove_component::<ChunkFrozen>(e);
        if let Some(mut body) = world.query_mut::<&mut RigidBody>().get_mut(e) {
            body.body_type = body_type;
            body.wake_up();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **A body over an unloaded chunk waits for it.** A car the player left at the start fell
    /// 6.9 km out of `dunya` once its chunk unloaded and took the ground with it. Held, it stays
    /// where it was, at rest; back in the loaded square it is dynamic again and awake.
    #[cfg(feature = "physics")]
    #[test]
    fn a_body_over_an_unloaded_chunk_is_held_until_the_chunk_is_back() {
        use gizmo_physics_core::components::Transform;
        use gizmo_physics_rigid::components::{BodyType, RigidBody, Velocity};
        let mut world = World::new();
        let parked = Vec3::new(5.0, 3.0, 5.0);
        let car = world.spawn_bundle((Transform::new(parked), RigidBody::new(900.0, true), Velocity::new(Vec3::new(0.0, -2.0, 0.0))));
        let stone = world.spawn_bundle((Transform::new(Vec3::new(5.0, 0.0, 6.0)), RigidBody::new_static()));
        let step = |world: &mut World, at: Vec3| open_world_chunk_system(world, at, |_, _| {}, |_, _, _| {});
        step(&mut world, Vec3::ZERO);
        assert!(world.get::<ChunkFrozen>(car).is_none(), "its chunk is loaded");
        // Walk 1 km away: chunk (0, 0) unloads.
        step(&mut world, Vec3::new(1000.0, 0.0, 0.0));
        assert_eq!(world.get::<ChunkFrozen>(car).map(|f| f.body_type), Some(BodyType::Dynamic));
        assert!(world.get::<RigidBody>(car).unwrap().is_kinematic());
        assert_eq!(world.get::<Velocity>(car).unwrap().linear, Vec3::ZERO);
        assert!(world.get::<ChunkFrozen>(stone).is_none(), "a static body is never held");
        // Something writes a velocity meanwhile: the next frame takes it away again.
        world.query_mut::<&mut Velocity>().get_mut(car).unwrap().linear = Vec3::X;
        step(&mut world, Vec3::new(1001.0, 0.0, 0.0));
        assert_eq!(world.get::<Velocity>(car).unwrap().linear, Vec3::ZERO);
        // Back: dynamic and awake.
        step(&mut world, Vec3::ZERO);
        assert!(world.get::<ChunkFrozen>(car).is_none());
        let body = world.get::<RigidBody>(car).unwrap();
        assert!(body.is_dynamic() && !body.is_sleeping);
        assert_eq!(world.get::<Transform>(car).unwrap().position, parked);
    }

    #[test]
    fn world_pos_to_chunk_origin_and_positive() {
        assert_eq!(ChunkManager::world_pos_to_chunk(Vec3::ZERO), (0, 0));
        // x=150, z=250 → (1, 2)
        assert_eq!(
            ChunkManager::world_pos_to_chunk(Vec3::new(150.0, 999.0, 250.0)),
            (1, 2)
        );
    }

    #[test]
    fn world_pos_to_chunk_is_floor_based_for_negatives() {
        // floor(-0.01/100)=floor(-0.0001)=-1 (truncation değil floor)
        assert_eq!(ChunkManager::world_pos_to_chunk(Vec3::new(-0.01, 0.0, -0.01)), (-1, -1));
        assert_eq!(ChunkManager::world_pos_to_chunk(Vec3::new(-100.0, 0.0, -250.0)), (-1, -3));
    }

    #[test]
    fn world_pos_to_chunk_boundary_is_half_open() {
        // tam CHUNK_SIZE bir üst chunk'a geçer; hemen altı önceki chunk'ta kalır
        assert_eq!(ChunkManager::world_pos_to_chunk(Vec3::new(100.0, 0.0, 100.0)), (1, 1));
        assert_eq!(ChunkManager::world_pos_to_chunk(Vec3::new(99.999, 0.0, 99.999)), (0, 0));
    }

    #[test]
    fn world_pos_to_chunk_ignores_y() {
        let a = ChunkManager::world_pos_to_chunk(Vec3::new(42.0, -5000.0, 42.0));
        let b = ChunkManager::world_pos_to_chunk(Vec3::new(42.0, 5000.0, 42.0));
        assert_eq!(a, b);
    }

    #[test]
    fn chunk_to_world_pos_returns_chunk_center_on_y0() {
        assert_eq!(ChunkManager::chunk_to_world_pos((0, 0)), Vec3::new(50.0, 0.0, 50.0));
        assert_eq!(ChunkManager::chunk_to_world_pos((1, 2)), Vec3::new(150.0, 0.0, 250.0));
        assert_eq!(ChunkManager::chunk_to_world_pos((-1, -1)), Vec3::new(-50.0, 0.0, -50.0));
    }

    #[test]
    fn chunk_center_round_trips_back_to_same_chunk() {
        for coord in [(0, 0), (3, -2), (-5, 7), (i32::from(-1i8), 4)] {
            let center = ChunkManager::chunk_to_world_pos(coord);
            assert_eq!(ChunkManager::world_pos_to_chunk(center), coord);
        }
    }

    #[test]
    fn register_entity_creates_then_appends_preserving_order() {
        let mut m = ChunkManager::default();
        m.register_entity((0, 0), 10);
        m.register_entity((0, 0), 20);
        m.register_entity((1, 1), 30);
        assert_eq!(m.chunk_entities[&(0, 0)], vec![10, 20]);
        assert_eq!(m.chunk_entities[&(1, 1)], vec![30]);
        assert_eq!(m.chunk_entities.len(), 2);
    }

    #[test]
    fn default_last_player_chunk_is_unreachable_sentinel() {
        // İlk frame'de kesin tetiklensin diye imkansız bir değer
        let m = ChunkManager::default();
        assert_eq!(m.last_player_chunk, (i32::MAX, i32::MAX));
        assert!(m.active_chunks.is_empty());
    }
}
