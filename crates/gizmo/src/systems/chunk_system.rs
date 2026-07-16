use crate::core::World;
use gizmo_math::Vec3;
use std::collections::{HashMap, HashSet};

pub const CHUNK_SIZE: f32 = 100.0;
pub const CHUNK_LOAD_RADIUS: i32 = 2; // Oyuncunun etrafındaki 5x5'lik grid yüklenir

pub type ChunkCoord = (i32, i32);

/// Hangi Entity'nin hangi Chunk'a ait olduğunu tutan Bileşen (Component)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChunkEntity {
    pub coord: ChunkCoord,
}

/// Tüm harita yükleme/silme işlemlerini yöneten Sistem Kaynağı (Resource)
#[derive(Debug, Clone)]
pub struct ChunkManager {
    /// O an RAM'de yüklü olan Chunk'ların koordinatları
    pub active_chunks: HashSet<ChunkCoord>,
    /// Hangi Chunk'ta hangi Entity'lerin olduğu (Silmek için kullanacağız)
    pub chunk_entities: HashMap<ChunkCoord, Vec<u64>>,
    /// Oyuncunun bir önceki frame'deki konumu (Sadece Chunk değiştiğinde işlem yapmak için)
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
    pub fn world_pos_to_chunk(pos: Vec3) -> ChunkCoord {
        (
            (pos.x / CHUNK_SIZE).floor() as i32,
            (pos.z / CHUNK_SIZE).floor() as i32,
        )
    }

    pub fn chunk_to_world_pos(coord: ChunkCoord) -> Vec3 {
        Vec3::new(
            (coord.0 as f32) * CHUNK_SIZE + (CHUNK_SIZE / 2.0),
            0.0,
            (coord.1 as f32) * CHUNK_SIZE + (CHUNK_SIZE / 2.0),
        )
    }

    /// Bir Entity oluşturulduğunda onu Chunk sistemine kaydeder
    pub fn register_entity(&mut self, coord: ChunkCoord, entity_id: u64) {
        self.chunk_entities
            .entry(coord)
            .or_default()
            .push(entity_id);
    }
}

/// Bu sistem her frame çağrılır. Oyuncunun pozisyonunu kontrol eder.
/// Eğer oyuncu yeni bir Chunk sınırından geçtiyse, eski Chunk'ları silip yenilerini yükler.
/// `load_callback` fonksiyonu: Yeni yüklenen her Chunk için kullanıcının obje spawn etmesini sağlar.
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
