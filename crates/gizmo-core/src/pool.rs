use crate::entity::Entity;
use crate::world::World;
use std::collections::{HashMap, VecDeque};

/// Havuzda tutulacak nesnelerin durumunu belirten marker component.
/// Bu nesneler aktif değildir, yeniden kullanılmayı beklerler.
#[derive(Clone, Copy)]
pub struct Pooled;

crate::impl_component!(Pooled);

pub struct ObjectPool {
    /// Orijinal prefab nesnesi (bu nesne klonlanarak çoğaltılacak)
    pub prefab_id: u32,
    /// Kullanılmayan, havuzdaki boş nesnelerin listesi
    pub inactive: VecDeque<Entity>,
}

impl ObjectPool {
    pub fn new(prefab_id: u32) -> Self {
        Self {
            prefab_id,
            inactive: VecDeque::new(),
        }
    }
}

/// Nesne Havuzu Yönetim Sistemi
/// Mermiler, partiküller veya düşmanlar gibi sık yaratılıp yok edilen nesneleri
/// her seferinde tahsis etmek yerine tekrar kullanmanızı sağlar.
pub struct PoolManager {
    pools: HashMap<String, ObjectPool>,
}

impl Default for PoolManager {
    fn default() -> Self {
        Self::new()
    }
}

impl PoolManager {
    pub fn new() -> Self {
        Self {
            pools: HashMap::new(),
        }
    }

    /// Bir prefab nesnesini kaynak göstererek yeni bir havuz oluşturur.
    /// Prefab otomatik olarak `Pooled` ile işaretlenir, böylece render ve fizik sistemleri onu atlar.
    pub fn register_pool(&mut self, name: &str, prefab_entity: Entity) {
        self.pools.insert(name.to_string(), ObjectPool::new(prefab_entity.id()));
    }

    /// `register_pool` ile aynı, ama ek olarak prefab entity'yi `Pooled` ile işaretler.
    /// Bu sayede prefab asla render edilmez ve fizik sistemi tarafından simüle edilmez.
    pub fn register_pool_hidden(&mut self, world: &mut World, name: &str, prefab_entity: Entity) {
        world.add_component(prefab_entity, Pooled);
        self.pools.insert(name.to_string(), ObjectPool::new(prefab_entity.id()));
    }

    /// Bir bundle (MeshBundle vb.) ve zincirlenmiş bileşenleri doğrudan havuza kaydeder.
    /// Bundle anında spawn edilir ve çıkan Entity havuz referansı olarak kullanılır.
    pub fn register<B: crate::component::Bundle>(&mut self, world: &mut World, name: &str, bundle: B) {
        let prefab = world.spawn_bundle(bundle);
        self.register_pool(name, prefab);
    }


    /// Havuzdan bir nesne alır. Havuz boşsa prefab'ı klonlayarak yeni bir nesne üretir.
    pub fn instantiate(&mut self, world: &mut World, name: &str) -> Option<Entity> {
        let pool = self.pools.get_mut(name)?;
        
        if let Some(entity) = pool.inactive.pop_front() {
            // Nesne havuzdan çıkarıldı, `Pooled` tag'i siliniyor.
            world.remove_component::<Pooled>(entity);
            Some(entity)
        } else {
            // Havuz boş, prefab klonlanarak yeni obje yaratılacak!
            // `clone_entity` fonksiyonumuz O(1) prefab kopyalama desteği sunuyor
            let new_entities = world.clone_entity(pool.prefab_id, 1)?;
            let new_ent = new_entities[0];
            // Prefab Pooled olarak işaretlenmiş olabilir (register_pool_hidden),
            // klonlanan entity'den Pooled tag'ını kaldır ki aktif olarak doğsun.
            world.remove_component::<Pooled>(new_ent);
            Some(new_ent)
        }
    }

    /// Bir nesneyi tamamen yok etmek (despawn) yerine havuza geri gönderir.
    pub fn destroy(&mut self, world: &mut World, name: &str, entity: Entity) {
        if let Some(pool) = self.pools.get_mut(name) {
            // Nesne pasife alındığını bilmesi için Pooled bileşeni ekleniyor.
            world.add_component(entity, Pooled);
            pool.inactive.push_back(entity);
        } else {
            // Havuz bulunamadıysa standart despawn yap.
            world.despawn(entity);
        }
    }
}
