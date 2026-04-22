use std::any::TypeId;
use std::marker::PhantomData;
use std::sync::{RwLockReadGuard, RwLockWriteGuard};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceFetchError {
    NotFound(TypeId),
    BorrowConflict(TypeId),
}

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

/// Sıfır allocation ile yaşayan entity'ler üzerinde iterasyon yapan iterator.


#[cfg(test)]
mod tests {
    use crate::impl_component;
    use crate::World;
    use crate::Entity;

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

    /// Aynı component türü iki kez eklenince archetype migration'da veri güncellenmeli.
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
        
        world.register_on_add::<HookTracker>(|_, _| {
            ADD_COUNT.fetch_add(1, Ordering::SeqCst);
        });
        world.register_on_set::<HookTracker>(|_, _| {
            SET_COUNT.fetch_add(1, Ordering::SeqCst);
        });
        world.register_on_remove::<HookTracker>(|_, _| {
            REMOVE_COUNT.fetch_add(1, Ordering::SeqCst);
        });

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
