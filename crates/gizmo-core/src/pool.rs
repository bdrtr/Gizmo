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

/// The marker component that indicates the state of objects to be kept in the pool.
/// These objects are not active, they are waiting to be reused.
#[derive(Clone, Copy)]
pub struct Pooled;

crate::impl_component!(Pooled);

/// One pool's bookkeeping: the entity to clone from, plus the instances currently parked.
///
/// Plain data with no invariants of its own — [`PoolManager`] is what keeps it consistent.
/// Nothing here checks that a parked entity is still alive, that it appears only once, or
/// that it originally came from this pool.
///
pub struct ObjectPool {
    /// The prefab every new instance is cloned from.
    ///
    /// **An `Entity`, not a raw id, since 2026-08-31.** It used to be a bare `u32`, and this
    /// doc used to describe the consequence rather than fix it: an id carries no generation, so
    /// nothing noticed when the prefab was despawned and its slot recycled — `instantiate` then
    /// cloned whatever entity happened to occupy that slot, silently, and handed the result out
    /// as a bullet. Keeping the generation is what lets `instantiate` tell the two apart.
    ///
    /// It still does not keep the prefab alive. A despawned prefab makes the pool unable to
    /// produce new instances, which `instantiate` reports by returning `None` rather than by
    /// inventing one.
    pub prefab: Entity,
    /// The list of unused, idle objects in the pool
    pub inactive: VecDeque<Entity>,
}

impl ObjectPool {
    /// An empty pool sourcing from `prefab`, so the first instantiate clones it instead of
    /// reusing anything.
    ///
    /// Touches no world state: the prefab is not checked for liveness here and not marked
    /// [`Pooled`] — see [`PoolManager::register_pool_hidden`] for the second, and
    /// [`PoolManager::instantiate`] for the first, which is where liveness has to be rechecked
    /// anyway because the prefab can die at any point after this call.
    pub fn new(prefab: Entity) -> Self {
        Self {
            prefab,
            inactive: VecDeque::new(),
        }
    }
}

/// The Object Pool Management System
/// It lets you reuse objects that are frequently created and destroyed — such as bullets,
/// particles or enemies — instead of allocating them every single time.
pub struct PoolManager {
    pools: HashMap<String, ObjectPool>,
    /// The [`World::reset_epoch`] every pool here was registered against.
    ///
    /// A pool holds `Entity` handles — the prefab and everything parked — and
    /// [`World::clear_entities`] resets the generation counter, so after one those handles come
    /// back bit-identical attached to entities of the NEW scene. `is_alive` passes for every one
    /// of them, which is why the two liveness checks in [`PoolManager::instantiate`] do not
    /// reach this case and nothing per-handle ever could. Comparing the epoch is the only thing
    /// that can, and every method taking a `&World` does it first.
    epoch: u64,
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
            epoch: 0,
        }
    }

    /// Drops every pool if the world has been cleared since they were registered.
    ///
    /// Called first by everything here that takes a `&World`. It is a whole-manager reset rather
    /// than a per-pool one because a clear destroys every entity there is: no pool can survive
    /// it, and a pool whose prefab and parked instances are all strangers is worse than no pool.
    /// `instantiate` then reports the name as unknown, which is true.
    fn discard_if_world_was_cleared(&mut self, world: &World) {
        let now = world.reset_epoch();
        if self.epoch != now {
            if !self.pools.is_empty() {
                tracing::debug!(
                    pools = self.pools.len(),
                    from = self.epoch,
                    to = now,
                    "PoolManager: the world was cleared; every pool's handles name other \
                     entities now, so all of them are dropped"
                );
            }
            self.pools.clear();
            self.epoch = now;
        }
    }

    /// Creates a new pool using a prefab object as its source.
    ///
    /// Takes the world since 2026-08-31, and only to read [`World::reset_epoch`]: a pool
    /// registered before a [`World::clear_entities`] must not survive it, and one registered
    /// *after* must not be swept away with the ones that came before. Distinguishing those two
    /// needs the epoch at registration time, which is the whole reason for the parameter.
    pub fn register_pool(&mut self, world: &World, name: &str, prefab_entity: Entity) {
        self.discard_if_world_was_cleared(world);
        self.pools
            .insert(name.to_string(), ObjectPool::new(prefab_entity));
    }

    /// The same as `register_pool`, but it additionally marks the prefab entity with `Pooled`.
    /// This way the prefab is never rendered and is not simulated by the physics system.
    pub fn register_pool_hidden(&mut self, world: &mut World, name: &str, prefab_entity: Entity) {
        self.discard_if_world_was_cleared(world);
        world.add_component(prefab_entity, Pooled);
        self.pools
            .insert(name.to_string(), ObjectPool::new(prefab_entity));
    }

    /// Registers a bundle (MeshBundle etc.) and its chained components directly into the pool.
    /// The bundle is spawned immediately and the resulting Entity is used as the pool reference.
    pub fn register<B: crate::component::Bundle>(
        &mut self,
        world: &mut World,
        name: &str,
        bundle: B,
    ) {
        let prefab = world.spawn_bundle(bundle);
        self.register_pool(world, name, prefab);
    }

    /// Takes an object from the pool. If the pool is empty it produces a new object by cloning
    /// the prefab.
    ///
    /// **Both halves check liveness, and neither used to.** `destroy` refuses to park a dead
    /// entity, but nothing kept a parked one alive afterwards — anything holding the handle can
    /// despawn it, and `clear_entities` destroys the lot — so the queue could hand out a corpse
    /// on which every later component insert silently does nothing. Dead entries are discarded
    /// here and the search continues, falling through to a clone if the whole queue is stale.
    ///
    /// A despawned prefab makes the pool unable to produce anything new, and that is reported as
    /// `None` rather than by cloning whatever now occupies its id slot.
    pub fn instantiate(&mut self, world: &mut World, name: &str) -> Option<Entity> {
        self.discard_if_world_was_cleared(world);
        let pool = self.pools.get_mut(name)?;

        while let Some(entity) = pool.inactive.pop_front() {
            if !world.is_alive(entity) {
                tracing::debug!(
                    entity = entity.id(),
                    pool = name,
                    "PoolManager::instantiate: parked entity died while pooled; discarded"
                );
                continue;
            }
            // Nesne havuzdan çıkarıldı, `Pooled` tag'i siliniyor.
            world.remove_component::<Pooled>(entity);
            return Some(entity);
        }

        // Havuz boş (ya da tamamı bayattı) — prefab klonlanarak yeni obje yaratılacak.
        if !world.is_alive(pool.prefab) {
            tracing::debug!(
                pool = name,
                prefab = pool.prefab.id(),
                "PoolManager::instantiate: the prefab is gone; the pool cannot produce more"
            );
            return None;
        }
        // `clone_entity` fonksiyonumuz O(1) prefab kopyalama desteği sunuyor
        let new_entities = world.clone_entity(pool.prefab.id(), 1)?;
        let new_ent = new_entities[0];
        // Prefab Pooled olarak işaretlenmiş olabilir (register_pool_hidden),
        // klonlanan entity'den Pooled tag'ını kaldır ki aktif olarak doğsun.
        world.remove_component::<Pooled>(new_ent);
        Some(new_ent)
    }

    /// Instead of destroying an object outright (despawn), sends it back to the pool.
    ///
    /// Parking is idempotent: an entity already sitting in the pool, and an entity that is
    /// no longer alive, are both ignored (a `tracing::debug!` records it). Without that,
    /// two retire paths for the same bullet queued its id twice and two later
    /// [`instantiate`](Self::instantiate) calls handed the SAME entity to two different
    /// callers.
    ///
    /// If `name` is not a registered pool the entity is despawned outright instead.
    pub fn destroy(&mut self, world: &mut World, name: &str, entity: Entity) {
        self.discard_if_world_was_cleared(world);
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
        pools.register_pool(&world, "bullets", prefab);
        (world, pools)
    }

    /// A despawned prefab must stop the pool, not make it clone a stranger.
    ///
    /// `ObjectPool` stored the prefab as a raw `u32`, and its own doc described what that cost
    /// without fixing it: an id carries no generation, so once the prefab was despawned and its
    /// slot recycled, `instantiate` cloned whatever entity had taken the slot and handed the
    /// copy out as a pooled object. Ordinary use gets there — `Entities::reserve_entity` drains
    /// the free list first, so the very next spawn after the despawn lands on that slot.
    #[test]
    fn a_despawned_prefab_stops_the_pool_instead_of_cloning_its_replacement() {
        #[derive(Clone, Copy)]
        struct Stranger;
        crate::impl_component!(Stranger);

        let (mut world, mut pools) = setup();
        let prefab = pools.pools["bullets"].prefab;

        world.despawn(prefab);
        // The next spawn takes the freed slot, so the raw id now names somebody else entirely.
        let replacement = world.spawn();
        world.add_component(replacement, Stranger);
        assert_eq!(replacement.id(), prefab.id(), "the id really was recycled");

        assert!(
            pools.instantiate(&mut world, "bullets").is_none(),
            "the pool has no prefab any more and must say so"
        );
        assert_eq!(
            world.query::<&Stranger>().map(|q| q.iter().count()),
            Some(1),
            "and nothing cloned the entity that took the prefab's slot"
        );
    }

    /// A parked entity that dies while pooled must not be handed out.
    ///
    /// `destroy` refuses to park a dead entity, but nothing kept a parked one alive afterwards:
    /// anything holding the handle can despawn it. The queue then handed out a corpse, on which
    /// every later component insert silently does nothing — the same failure the parking guard
    /// was written to prevent, one step later.
    #[test]
    fn a_parked_entity_that_dies_while_pooled_is_discarded_not_handed_out() {
        let (mut world, mut pools) = setup();

        let a = pools.instantiate(&mut world, "bullets").expect("clone from prefab");
        let b = pools.instantiate(&mut world, "bullets").expect("clone from prefab");
        pools.destroy(&mut world, "bullets", a);
        pools.destroy(&mut world, "bullets", b);

        // Something else destroys the first parked one behind the pool's back.
        world.despawn(a);

        let handed_out = pools.instantiate(&mut world, "bullets").expect("one live entry left");
        assert_ne!(handed_out, a, "the dead entry must not be handed out");
        assert_eq!(handed_out, b);
        assert!(world.is_alive(handed_out));
    }

    /// After a `clear_entities` the pool must not hand out entities of the new scene.
    ///
    /// This is the case neither liveness check can reach, and the reason is structural rather
    /// than an oversight: a clear resets the generation counter as well as the id counter, so
    /// every parked handle and the prefab come back BIT-IDENTICAL attached to whatever the new
    /// scene spawns. `is_alive` returns true for all of them. Only `World::reset_epoch` can tell
    /// the difference, because it is about the world rather than about any one handle.
    #[test]
    fn a_world_clear_empties_the_pools_instead_of_recycling_the_new_scene() {
        #[derive(Clone, Copy)]
        struct Newcomer;
        crate::impl_component!(Newcomer);

        let (mut world, mut pools) = setup();
        let parked = pools.instantiate(&mut world, "bullets").expect("clone from prefab");
        pools.destroy(&mut world, "bullets", parked);

        world.clear_entities();

        // The new scene takes the ids the pool is still holding, generation and all. Spawn far
        // enough to reach the parked id — ids restart at 0, so one spawn is not necessarily it.
        let mut newcomer = None;
        for _ in 0..=parked.id() {
            let e = world.spawn();
            if e == parked {
                newcomer = Some(e);
            }
        }
        let newcomer = newcomer.expect("the freed ids come back in order, so one of them is it");
        world.add_component(newcomer, Newcomer);
        assert!(
            world.is_alive(parked),
            "the parked handle passes the liveness check — that is the whole problem, and it \
             is why no per-handle test can close this case"
        );

        assert!(
            pools.instantiate(&mut world, "bullets").is_none(),
            "the pool's handles all name entities of the new scene, so the pool is gone"
        );
        assert_eq!(
            world.query::<&Newcomer>().map(|q| q.iter().count()),
            Some(1),
            "and the new scene's entity was neither handed out nor cloned"
        );
    }

    /// A pool registered AFTER a clear must survive — the sweep is not indiscriminate.
    ///
    /// Without an epoch recorded at registration this is the case that breaks: a manager that
    /// simply compared "my epoch" against the world's would drop the pool it had just been given
    /// on the next call. That is why `register_pool` takes the world.
    #[test]
    fn a_pool_registered_after_a_clear_is_kept() {
        let (mut world, mut pools) = setup();
        world.clear_entities();

        let prefab = world.spawn();
        world.add_component(prefab, Bullet);
        pools.register_pool(&world, "bullets", prefab);

        let made = pools.instantiate(&mut world, "bullets");
        assert!(made.is_some(), "a pool registered after the clear is current, not stale");
        assert!(world.is_alive(made.unwrap()));
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
