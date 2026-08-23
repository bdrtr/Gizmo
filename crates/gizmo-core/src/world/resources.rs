//! World resources: singletons keyed by type instead of being attached to an entity.
//!
//! Each resource sits behind its own `RwLock`, which is why `&World` suffices to hand out a
//! *mutable* guard — exclusivity is per resource and enforced at runtime, not by the borrow
//! checker. All the accessors here are built on the non-blocking `try_*` primitives, so a
//! request that conflicts with a guard you are still holding returns `None` /
//! [`ResourceFetchError::BorrowConflict`] rather than deadlocking. The single exception is
//! [`World::get_resource_mut_or_default`], which takes `&mut World` and does block.

use super::World;
use std::any::TypeId;
use std::marker::PhantomData;
use std::sync::RwLock;
use std::sync::{RwLockReadGuard, RwLockWriteGuard};

/// Why a typed resource lookup failed.
///
/// Both variants carry the *requested* type's `TypeId`. It is included because these errors
/// come out of generic functions whose `T` is not otherwise recoverable from the value; it
/// has no printable name, so log `std::any::type_name::<T>()` at the call site instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ResourceFetchError {
    /// No resource of this type is in the world — never inserted, or removed since.
    NotFound(TypeId),
    /// The resource exists but its lock could not be taken without waiting: a conflicting
    /// guard for it is still alive, or an earlier panic while a guard was held poisoned the
    /// lock. Either way the fetch gives up immediately rather than blocking, so this is
    /// never a deadlock — but it is also indistinguishable from ordinary contention.
    BorrowConflict(TypeId),
}

impl std::fmt::Display for ResourceFetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResourceFetchError::NotFound(tid) => {
                write!(f, "resource not found in world (TypeId: {tid:?})")
            }
            ResourceFetchError::BorrowConflict(tid) => {
                write!(f, "resource borrow conflict (TypeId: {tid:?})")
            }
        }
    }
}

impl std::error::Error for ResourceFetchError {}

/// Shared access to one world resource, valid for as long as the guard lives.
///
/// `Deref` only — obtain a [`ResourceWriteGuard`] to mutate. While this guard exists the
/// resource's read lock is held, so [`World::get_resource_mut`] for the *same* type returns
/// `None`; other resources are unaffected, each having its own lock. The guard also borrows
/// the `World` immutably, so no `&mut World` method can run until it is dropped.
pub struct ResourceReadGuard<'a, T> {
    pub(crate) guard: RwLockReadGuard<'a, Box<dyn std::any::Any + Send + Sync>>,
    pub(crate) _marker: PhantomData<T>,
}

impl<'a, T: 'static> std::ops::Deref for ResourceReadGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.guard.downcast_ref::<T>().unwrap()
    }
}

/// Exclusive access to one world resource, valid for as long as the guard lives.
///
/// While it exists the resource's write lock is held, so *every* further fetch of the same
/// type — read or write — fails with [`ResourceFetchError::BorrowConflict`]; other
/// resources are unaffected. Note that it is handed out from `&World`, so holding two of
/// these for two different resources at once is legal and expected.
///
/// Dropping it is the only thing that releases the lock: leak one (`std::mem::forget`) and
/// every later fetch of that type keeps failing with [`ResourceFetchError::BorrowConflict`].
pub struct ResourceWriteGuard<'a, T> {
    pub(crate) guard: RwLockWriteGuard<'a, Box<dyn std::any::Any + Send + Sync>>,
    pub(crate) _marker: PhantomData<T>,
}

impl<'a, T: 'static> std::ops::Deref for ResourceWriteGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.guard.downcast_ref::<T>().unwrap()
    }
}

impl<'a, T: 'static> std::ops::DerefMut for ResourceWriteGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.guard.downcast_mut::<T>().unwrap()
    }
}

impl World {
    // ==========================================================
    // RESOURCE SİSTEMİ (GLOBAL VERİLER)
    // ==========================================================

    /// Adds a global Resource to the system, or overwrites it.
    pub fn insert_resource<T: Send + Sync + 'static>(&mut self, resource: T) {
        let type_id = TypeId::of::<T>();
        let replaced = self
            .resources
            .insert(type_id, RwLock::new(Box::new(resource)))
            .is_some();
        tracing::debug!(
            resource = std::any::type_name::<T>(),
            replaced,
            "insert_resource"
        );
    }

    /// Called in order to read a global Resource (Immutable Borrow)
    pub fn get_resource<T: 'static>(&self) -> Option<ResourceReadGuard<'_, T>> {
        self.try_get_resource::<T>().ok()
    }

    /// Called in order to modify a global Resource (Mutable Borrow)
    pub fn get_resource_mut<T: 'static>(&self) -> Option<ResourceWriteGuard<'_, T>> {
        self.try_get_resource_mut::<T>().ok()
    }

    /// The same function as `get_resource`, but it carries the failure reason via `Result`.
    pub fn try_get_resource<T: 'static>(
        &self,
    ) -> Result<ResourceReadGuard<'_, T>, ResourceFetchError> {
        let type_id = TypeId::of::<T>();
        let storage = self
            .resources
            .get(&type_id)
            .ok_or(ResourceFetchError::NotFound(type_id))?;
        let guard = storage
            .try_read()
            .map_err(|_| ResourceFetchError::BorrowConflict(type_id))?;
        Ok(ResourceReadGuard {
            guard,
            _marker: PhantomData,
        })
    }

    /// The same function as `get_resource_mut`, but it carries the failure reason via `Result`.
    pub fn try_get_resource_mut<T: 'static>(
        &self,
    ) -> Result<ResourceWriteGuard<'_, T>, ResourceFetchError> {
        let type_id = TypeId::of::<T>();
        let storage = self
            .resources
            .get(&type_id)
            .ok_or(ResourceFetchError::NotFound(type_id))?;
        let guard = storage
            .try_write()
            .map_err(|_| ResourceFetchError::BorrowConflict(type_id))?;
        Ok(ResourceWriteGuard {
            guard,
            _marker: PhantomData,
        })
    }

    /// If a global Resource does not exist it creates it as Default, then returns a Mutable Borrow.
    /// It requires a World mutable borrow, so that the registration into the hashmap is safe.
    pub fn get_resource_mut_or_default<T: Default + Send + Sync + 'static>(
        &mut self,
    ) -> ResourceWriteGuard<'_, T> {
        let type_id = TypeId::of::<T>();
        self.resources
            .entry(type_id)
            .or_insert_with(|| RwLock::new(Box::new(T::default())));

        let storage = self
            .resources
            .get(&type_id)
            .expect("resource just inserted");
        // Poison kurtarma: bir resource kullanıcısı panikleyip kilidi
        // zehirlese bile motor çalışmaya devam etsin (imza değişmez). Sessiz
        // kurtarma bir paniği gizlerdi; artık önce uyarı basılır (yalnız poison
        // yolunda çalışır, mutlu yolda maliyeti sıfırdır).
        let guard = storage.write().unwrap_or_else(|e| {
            tracing::warn!(
                resource = std::any::type_name::<T>(),
                "resource lock poisoned by an earlier panic; recovering"
            );
            e.into_inner()
        });
        ResourceWriteGuard {
            guard,
            _marker: PhantomData,
        }
    }

    /// Removes a global Resource from the ECS entirely and returns its ownership
    pub fn remove_resource<T: 'static>(&mut self) -> Option<T> {
        let type_id = TypeId::of::<T>();
        let Some(cell) = self.resources.remove(&type_id) else {
            tracing::trace!(resource = std::any::type_name::<T>(), "remove_resource: not present");
            return None;
        };
        // Poisoned RwLock: a previous holder panicked. The original code swallowed this
        // via `.ok()?`, silently returning None (and dropping the resource) with no trace
        // of the earlier panic. Keep that behavior but surface it as a warning first.
        let boxed_any = match cell.into_inner() {
            Ok(b) => b,
            Err(_) => {
                tracing::warn!(
                    resource = std::any::type_name::<T>(),
                    "remove_resource: lock poisoned by an earlier panic; resource dropped"
                );
                return None;
            }
        };
        match boxed_any.downcast::<T>() {
            Ok(boxed_t) => {
                tracing::debug!(resource = std::any::type_name::<T>(), "remove_resource");
                Some(*boxed_t)
            }
            // Keyed by TypeId, so the stored box is always `T`; a mismatch means a
            // registration invariant broke. Previously returned None silently.
            Err(_) => {
                tracing::warn!(
                    resource = std::any::type_name::<T>(),
                    "remove_resource: TypeId matched but downcast failed (registry invariant broken)"
                );
                None
            }
        }
    }

    /// Temporarily takes a resource out of the world, passes it to a closure, and then puts it
    /// back. This is the cleanest way to keep the borrow checker happy when you need to use
    /// `&mut World` while inside the resource.
    ///
    /// # Example
    /// ```
    /// # use gizmo_core::prelude::*;
    /// # #[derive(Clone)] struct Enemy;
    /// # gizmo_core::impl_component!(Enemy);
    /// # let mut world = World::new();
    /// # let prefab = world.spawn();
    /// # world.add_component(prefab, Enemy);
    /// # let mut pools = PoolManager::new();
    /// # pools.register_pool("enemy", prefab);
    /// # world.insert_resource(pools);
    /// // The third parameter is the closure's own type, so it can only be `_`.
    /// world.resource_scope::<PoolManager, (), _>(|world, pool| {
    ///     pool.instantiate(world, "enemy");
    /// });
    ///
    /// // The resource went back into the world, and the pool cloned the prefab -> prefab + clone = 2 Enemy.
    /// assert!(world.get_resource::<PoolManager>().is_some());
    /// assert_eq!(world.query::<&Enemy>().unwrap().iter().count(), 2);
    /// ```
    pub fn resource_scope<T: Send + Sync + 'static, U, F>(&mut self, f: F) -> Option<U>
    where
        F: FnOnce(&mut World, &mut T) -> U,
    {
        let resource = self.remove_resource::<T>()?;

        // Panic güvenliği: Closure panic yaparsa bile resource geri yerleştirilir.
        // Drop guard, stack unwind sırasında resource'u tekrar world'e koyar.
        struct Guard<'a, T: Send + Sync + 'static> {
            world: *mut World,
            resource: Option<T>,
            _marker: std::marker::PhantomData<&'a mut World>,
        }
        impl<'a, T: Send + Sync + 'static> Drop for Guard<'a, T> {
            fn drop(&mut self) {
                if let Some(resource) = self.resource.take() {
                    // SAFETY: self.world is valid for the lifetime of the Guard.
                    unsafe { &mut *self.world }.insert_resource(resource);
                }
            }
        }

        let mut guard = Guard::<T> {
            world: self as *mut World,
            resource: Some(resource),
            _marker: std::marker::PhantomData,
        };

        let result = f(self, guard.resource.as_mut().unwrap());

        // Normal dönüş: guard.drop() resource'u geri koyacak.
        Some(result)
    }
}

// Sıfır allocation ile yaşayan entity'ler üzerinde iterasyon yapan iterator.
#[cfg(test)]
mod tests {
    use crate::impl_component;
    use crate::Entity;
    use crate::World;

    // Test component types
    #[derive(Debug, Clone, PartialEq)]
    struct Position {
        x: f32,
        y: f32,
    }

    #[derive(Debug, Clone, PartialEq)]
    struct Health(u32);

    impl_component!(Position, Health);

    #[test]
    fn test_spawn_and_alive_count() {
        let mut world = World::new();
        let e1 = world.spawn();
        let e2 = world.spawn();
        let e3 = world.spawn();
        assert_eq!(world.entity_count(), 3);
        assert!(world.is_alive(e1));
        assert!(world.is_alive(e2));
        assert!(world.is_alive(e3));
    }

    #[test]
    fn test_despawn_removes_components() {
        let mut world = World::new();
        let e1 = world.spawn();
        world.add_component(e1, Position { x: 1.0, y: 2.0 });
        world.add_component(e1, Health(100));

        assert!(world.borrow::<Position>().get(e1.id()).is_some());
        assert!(world.borrow::<Health>().get(e1.id()).is_some());

        world.despawn(e1);

        assert!(!world.is_alive(e1));
        assert!(world.borrow::<Position>().get(e1.id()).is_none());
        assert!(world.borrow::<Health>().get(e1.id()).is_none());
    }

    #[test]
    fn test_despawn_only_touches_relevant_storages() {
        let mut world = World::new();
        let e1 = world.spawn();
        let e2 = world.spawn();

        // e1 has Position only, e2 has both
        world.add_component(e1, Position { x: 0.0, y: 0.0 });
        world.add_component(e2, Position { x: 1.0, y: 1.0 });
        world.add_component(e2, Health(50));

        // Despawn e1 — should not affect e2
        world.despawn(e1);

        assert!(world.borrow::<Position>().get(e2.id()).is_some());
        assert!(world.borrow::<Health>().get(e2.id()).is_some());
        assert_eq!(world.entity_count(), 1);
    }

    #[test]
    fn test_iter_alive_entities_zero_allocation() {
        let mut world = World::new();
        let _e1 = world.spawn();
        let e2 = world.spawn();
        let _e3 = world.spawn();

        world.despawn(e2);

        // Iterator should return 2 entities (e1 and e3), skipping e2
        let alive: Vec<Entity> = world.iter_alive_entities();
        assert_eq!(alive.len(), 2);
        assert!(alive.iter().all(|e: &Entity| e.id() != e2.id()));
    }

    #[test]
    fn test_entity_id_reuse_after_despawn() {
        let mut world = World::new();
        let e1 = world.spawn();
        let old_id = e1.id();
        let old_gen = e1.generation();

        world.despawn(e1);

        let e_new = world.spawn();
        // Should reuse the same ID with bumped generation
        assert_eq!(e_new.id(), old_id);
        assert_eq!(e_new.generation(), old_gen + 1);

        // Old entity should not be alive
        assert!(!world.is_alive(e1));
        assert!(world.is_alive(e_new));
    }

    #[test]
    fn test_add_component_overwrites() {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Health(100));
        world.add_component(e, Health(50)); // Overwrite

        let hp = world.borrow::<Health>();
        assert_eq!(hp.get(e.id()).unwrap().0, 50);
    }

    /// When the same component type is added twice the data must be updated in archetype migration.
    #[test]
    fn test_double_add_component_despawn_safe() {
        let mut world = World::new();
        let e = world.spawn();
        let id = e.id();
        world.add_component(e, Health(100));
        world.add_component(e, Health(50));

        world.despawn(e);

        assert!(!world.is_alive(e));
        assert!(world.borrow::<Health>().get(id).is_none());

        // ID yeniden kullanıldığında eski Health taşınmamalı
        let e2 = world.spawn();
        assert_eq!(e2.id(), id);
        assert!(world.borrow::<Health>().get(e2.id()).is_none());
    }

    #[test]
    fn test_component_registration_metadata() {
        let mut world = World::new();
        assert_eq!(world.registered_component_count(), 0);
        assert!(!world.is_component_registered::<Position>());
        assert!(!world.is_component_registered::<Health>());

        let e = world.spawn();
        world.add_component(e, Position { x: 1.0, y: 2.0 });
        assert!(world.is_component_registered::<Position>());
        assert_eq!(world.registered_component_count(), 1);

        world.add_component(e, Health(100));
        assert!(world.is_component_registered::<Health>());
        assert_eq!(world.registered_component_count(), 2);

        // remove metadata'yi silmez
        world.remove_component::<Health>(e);
        assert!(world.is_component_registered::<Health>());
        assert_eq!(world.registered_component_count(), 2);
    }

    #[derive(Debug, Clone, PartialEq)]
    struct HookTracker(u32);
    impl_component!(HookTracker);

    #[test]
    fn test_component_hooks() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static ADD_COUNT: AtomicUsize = AtomicUsize::new(0);
        static SET_COUNT: AtomicUsize = AtomicUsize::new(0);
        static REMOVE_COUNT: AtomicUsize = AtomicUsize::new(0);

        ADD_COUNT.store(0, Ordering::SeqCst);
        SET_COUNT.store(0, Ordering::SeqCst);
        REMOVE_COUNT.store(0, Ordering::SeqCst);

        let mut world = World::new();

        world.register_on_add::<HookTracker>(Box::new(|_, _| {
            ADD_COUNT.fetch_add(1, Ordering::SeqCst);
        }));
        world.register_on_set::<HookTracker>(Box::new(|_, _| {
            SET_COUNT.fetch_add(1, Ordering::SeqCst);
        }));
        world.register_on_remove::<HookTracker>(Box::new(|_, _| {
            REMOVE_COUNT.fetch_add(1, Ordering::SeqCst);
        }));

        let e1 = world.spawn();

        // Adding should trigger OnAdd and OnSet
        world.add_component(e1, HookTracker(1));
        assert_eq!(ADD_COUNT.load(Ordering::SeqCst), 1);
        assert_eq!(SET_COUNT.load(Ordering::SeqCst), 1);
        assert_eq!(REMOVE_COUNT.load(Ordering::SeqCst), 0);

        // Overwriting should trigger ONLY OnSet
        world.add_component(e1, HookTracker(2));
        assert_eq!(ADD_COUNT.load(Ordering::SeqCst), 1);
        assert_eq!(SET_COUNT.load(Ordering::SeqCst), 2);
        assert_eq!(REMOVE_COUNT.load(Ordering::SeqCst), 0);

        // Removing should trigger OnRemove
        world.remove_component::<HookTracker>(e1);
        assert_eq!(ADD_COUNT.load(Ordering::SeqCst), 1);
        assert_eq!(SET_COUNT.load(Ordering::SeqCst), 2);
        assert_eq!(REMOVE_COUNT.load(Ordering::SeqCst), 1);

        // Despawn should trigger OnRemove for remaining components
        world.add_component(e1, HookTracker(3));
        assert_eq!(ADD_COUNT.load(Ordering::SeqCst), 2); // added again
        assert_eq!(SET_COUNT.load(Ordering::SeqCst), 3);

        world.despawn(e1);
        assert_eq!(REMOVE_COUNT.load(Ordering::SeqCst), 2); // removed again via despawn
    }

    #[test]
    fn test_world_compaction() {
        let mut world = World::new();

        // Spawn 100 entities with two components
        for _ in 0..100 {
            let e = world.spawn();
            world.add_component(e, Position { x: 0.0, y: 0.0 });
            world.add_component(e, Health(10));
        }

        assert_eq!(world.archetype_index.archetype_count(), 3); // 0 (empty), 1 (Pos), 2 (Pos, Health)

        let all_entities = world.iter_alive_entities();

        // Remove 'Health' from the first 50 entities.
        // This moves them back to Archetype 1 (Pos).
        for e in all_entities.iter().take(50) {
            world.remove_component::<Health>(*e);
        }

        // Despawn the remaining 50 entities. (Archetype 2 is now completely EMPTY)
        for e in all_entities.iter().skip(50) {
            world.despawn(*e);
        }

        // Wait, removing components moved the 50 entities to archetype index 1.
        // Despawning the remaining 50 means archetype index 2 has 0 entities.
        assert_eq!(world.archetype_index.archetypes[2].len(), 0);

        // Call compaction
        world.compact();

        // The empty archetype 2 should be gone.
        assert_eq!(world.archetype_index.archetype_count(), 2);

        // The remaining 50 entities should still be fully accessible
        let pos_view = world.borrow::<Position>();
        let mut count = 0;
        for e in world.iter_alive_entities() {
            let eid: u32 = e.id();
            assert!(pos_view.get(eid).is_some());
            count += 1;
        }
        assert_eq!(count, 50);
    }
}
