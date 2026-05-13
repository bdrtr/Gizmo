use crate::core::World;
use gizmo_math::Vec3;
use std::collections::{HashMap, HashSet};

pub const CHUNK_SIZE: f32 = 100.0;
pub const CHUNK_LOAD_RADIUS: i32 = 2; // Oyuncunun etrafındaki 5x5'lik grid yüklenir

pub type ChunkCoord = (i32, i32);

/// Hangi Entity'nin hangi Chunk'a ait olduğunu tutan Bileşen (Component)
pub struct ChunkEntity {
    pub coord: ChunkCoord,
}

/// Tüm harita yükleme/silme işlemlerini yöneten Sistem Kaynağı (Resource)
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
