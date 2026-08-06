//! Object pooling: reuse despawned entities instead of allocating fresh ones.
//!
//! A [`PoolManager`] holds one [`ObjectPool`] per registered prefab and hands out entities
//! that were returned rather than destroyed. Intended for churn-heavy spawns — bullets,
//! particles, debris — where the allocation and the component inserts dominate.
//!
//! Pooled entities keep whatever components they were left with, so a pool is only as clean
//! as the code that returns entities to it.
use crate::entity::Entity;
use crate::world::World;
use std::collections::{HashMap, VecDeque};

/// Havuzda tutulacak nesnelerin durumunu belirten marker component.
/// Bu nesneler aktif değildir, yeniden kullanılmayı beklerler.
#[derive(Clone, Copy)]
pub struct Pooled;

crate::impl_component!(Pooled);

/// One pool's bookkeeping: the entity to clone from, plus the instances currently parked.
///
/// Plain data with no invariants of its own — [`PoolManager`] is what keeps it consistent.
/// Nothing here checks that a parked entity is still alive, that it appears only once, or
/// that it originally came from this pool.
///
/// Note `prefab_id` is a raw id, not an [`Entity`]: it carries no generation, so it neither
/// keeps the prefab alive nor notices when the prefab is despawned and its id slot recycled.
/// In that case cloning silently copies whatever entity now occupies the slot.
pub struct ObjectPool {
    /// Orijinal prefab nesnesi (bu nesne klonlanarak çoğaltılacak)
    pub prefab_id: u32,
    /// Kullanılmayan, havuzdaki boş nesnelerin listesi
    pub inactive: VecDeque<Entity>,
}

impl ObjectPool {
    /// An empty pool sourcing from `prefab_id`, so the first instantiate clones the prefab
    /// instead of reusing anything.
    ///
    /// Touches no world state: `prefab_id` is not checked for liveness and the prefab is not
    /// marked [`Pooled`] — see [`PoolManager::register_pool_hidden`] for that.
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
    /// A manager with no pools registered.
    ///
    /// Until a name is registered it is simply unknown, and the two accessors disagree about
    /// what that means: [`instantiate`](Self::instantiate) returns `None`, while
    /// [`destroy`](Self::destroy) falls back to despawning the entity outright.
    ///
    /// Equivalent to `PoolManager::default()`.
    pub fn new() -> Self {
        Self {
            pools: HashMap::new(),
        }
    }

    /// Bir prefab nesnesini kaynak göstererek yeni bir havuz oluşturur.
    /// Prefab otomatik olarak `Pooled` ile işaretlenir, böylece render ve fizik sistemleri onu atlar.
    pub fn register_pool(&mut self, name: &str, prefab_entity: Entity) {
        self.pools
            .insert(name.to_string(), ObjectPool::new(prefab_entity.id()));
    }

    /// `register_pool` ile aynı, ama ek olarak prefab entity'yi `Pooled` ile işaretler.
    /// Bu sayede prefab asla render edilmez ve fizik sistemi tarafından simüle edilmez.
    pub fn register_pool_hidden(&mut self, world: &mut World, name: &str, prefab_entity: Entity) {
        world.add_component(prefab_entity, Pooled);
        self.pools
            .insert(name.to_string(), ObjectPool::new(prefab_entity.id()));
    }

    /// Bir bundle (MeshBundle vb.) ve zincirlenmiş bileşenleri doğrudan havuza kaydeder.
    /// Bundle anında spawn edilir ve çıkan Entity havuz referansı olarak kullanılır.
    pub fn register<B: crate::component::Bundle>(
        &mut self,
        world: &mut World,
        name: &str,
        bundle: B,
    ) {
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
    ///
    /// Parking is idempotent: an entity already sitting in the pool, and an entity that is
    /// no longer alive, are both ignored (a `tracing::debug!` records it). Without that,
    /// two retire paths for the same bullet queued its id twice and two later
    /// [`instantiate`](Self::instantiate) calls handed the SAME entity to two different
    /// callers.
    ///
    /// If `name` is not a registered pool the entity is despawned outright instead.
    pub fn destroy(&mut self, world: &mut World, name: &str, entity: Entity) {
        if let Some(pool) = self.pools.get_mut(name) {
            // Membership + liveness guard. `inactive` is a bare queue with no index, so
            // nothing used to stop the same entity being parked twice; `instantiate` then
            // popped it once per copy and two callers fought over one entity's components.
            // A dead handle is worse than a duplicate: `add_component` no-ops on it, so it
            // was parked WITHOUT the `Pooled` tag and later handed out as a live object on
            // which every component insert silently does nothing.
            //
            // The `contains` scan is O(pool size). It is measured against nothing — but it
            // sits on the same call as `add_component`'s archetype migration, which is the
            // dominant cost here, and a side index would mean adding a field to
            // `ObjectPool`, whose fields are all `pub` (a struct-literal break). Revisit
            // only with a profile that shows this scan.
            if !world.is_alive(entity) {
                tracing::debug!(
                    entity = entity.id(),
                    pool = name,
                    "PoolManager::destroy: entity is already dead; not parked"
                );
                return;
            }
            if pool.inactive.contains(&entity) {
                tracing::debug!(
                    entity = entity.id(),
                    pool = name,
                    "PoolManager::destroy: entity is already parked; ignoring double retire"
                );
                return;
            }
            // Nesne pasife alındığını bilmesi için Pooled bileşeni ekleniyor.
            world.add_component(entity, Pooled);
            pool.inactive.push_back(entity);
        } else {
            // Havuz bulunamadıysa standart despawn yap.
            world.despawn(entity);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy)]
    struct Bullet;
    crate::impl_component!(Bullet);

    /// Registers a pool named "bullets" whose prefab carries one component, and returns the
    /// manager plus the world.
    fn setup() -> (World, PoolManager) {
        let mut world = World::new();
        let prefab = world.spawn();
        world.add_component(prefab, Bullet);
        let mut pools = PoolManager::new();
        pools.register_pool("bullets", prefab);
        (world, pools)
    }

    /// Retiring the same entity twice must park it ONCE.
    ///
    /// `destroy` had no membership check, so the id landed in `inactive` twice and the next
    /// two `instantiate` calls returned the same entity to two callers — both then wrote to
    /// one entity's components, and one of the two "objects" silently did not exist.
    #[test]
    fn destroy_parks_an_entity_at_most_once() {
        let (mut world, mut pools) = setup();

        let e = pools.instantiate(&mut world, "bullets").expect("pool exists");
        pools.destroy(&mut world, "bullets", e);
        pools.destroy(&mut world, "bullets", e); // ikinci kez emekliye ayırma

        let a = pools.instantiate(&mut world, "bullets").expect("pool exists");
        let b = pools.instantiate(&mut world, "bullets").expect("pool exists");
        assert_eq!(a, e, "the parked entity is reused first");
        assert_ne!(
            a, b,
            "the same entity was handed to two callers (double-parked)"
        );
    }

    /// A despawned entity must not be parked: `add_component` no-ops on a dead handle, so
    /// the pool used to hold an entity with no `Pooled` tag and a stale generation, and
    /// `instantiate` handed that dead handle out as a fresh object.
    #[test]
    fn destroy_does_not_park_a_dead_entity() {
        let (mut world, mut pools) = setup();

        let e = pools.instantiate(&mut world, "bullets").expect("pool exists");
        world.despawn(e);
        pools.destroy(&mut world, "bullets", e);

        let next = pools.instantiate(&mut world, "bullets").expect("pool exists");
        assert!(
            world.is_alive(next),
            "instantiate handed out a despawned entity"
        );
        assert_ne!(next, e, "the dead handle must not come back out of the pool");
    }

    /// NOT a regression test — it passes with or without the guards above. It fences the
    /// `else` branch the guards now sit beside: an unregistered pool name must still fall
    /// through to a plain despawn rather than being swallowed by an early return.
    #[test]
    fn destroy_on_unknown_pool_despawns() {
        let (mut world, mut pools) = setup();
        let e = pools.instantiate(&mut world, "bullets").expect("pool exists");

        pools.destroy(&mut world, "no_such_pool", e);

        assert!(!world.is_alive(e), "unknown pool falls back to despawn");
    }
}
