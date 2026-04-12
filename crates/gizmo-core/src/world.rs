use std::any::TypeId;
use std::collections::{HashMap, HashSet};
use std::{
    cell::RefCell,
    cell::{Ref, RefMut},
};

use crate::component::{Component, ComponentStorage, SparseSet};
use crate::entity::Entity;

pub struct World {
    next_entity_id: u32,
    generations: Vec<u32>,
    free_ids: Vec<u32>,
    free_set: HashSet<u32>, // O(1) contains() kontrolü için
    // RefCell: Aynı anda farklı Component dizilerini eşzamanlı olarak ödünç alabilmek (Borrow) için çok kritiktir.
    storages: HashMap<TypeId, RefCell<Box<dyn ComponentStorage>>>,
    // Entity'den bağımsız global veriler (Time, WindowSize, Input vs.)
    resources: HashMap<TypeId, RefCell<Box<dyn std::any::Any>>>,
    // Entity başına hangi TypeId'lerde component var — despawn'da O(S) yerine O(C) tarama.
    // HashSet: aynı tür iki kez `add_component` ile eklenemez; despawn'ta `remove_entity` yalnızca bir kez çağrılır.
    entity_components: HashMap<u32, HashSet<TypeId>>,
}

impl World {
    pub fn new() -> Self {
        Self {
            next_entity_id: 0,
            generations: Vec::new(),
            free_ids: Vec::new(),
            free_set: HashSet::new(),
            storages: HashMap::new(),
            resources: HashMap::new(),
            entity_components: HashMap::new(),
        }
    }
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}

impl World {
    pub fn spawn(&mut self) -> Entity {
        if let Some(id) = self.free_ids.pop() {
            self.free_set.remove(&id);
            let gen = self.generations[id as usize];
            Entity::new(id, gen)
        } else {
            let id = self.next_entity_id;
            self.next_entity_id += 1;
            self.generations.push(0);
            Entity::new(id, 0)
        }
    }

    pub fn get_entity(&self, id: u32) -> Option<Entity> {
        if (id as usize) < self.generations.len() && !self.free_set.contains(&id) {
            return Some(Entity::new(id, self.generations[id as usize]));
        }
        None
    }

    pub fn despawn(&mut self, entity: Entity) {
        if self.is_alive(entity) {
            let id = entity.id();
            self.generations[id as usize] += 1;
            self.free_ids.push(id);
            self.free_set.insert(id);

            // Sadece bu entity'nin component'ı olan storage'lara dokunur — O(C) (C = entity'nin component sayısı)
            if let Some(type_ids) = self.entity_components.remove(&id) {
                for type_id in type_ids {
                    if let Some(storage) = self.storages.get_mut(&type_id) {
                        storage.get_mut().remove_entity(id);
                    }
                }
            }
        }
    }

    pub fn despawn_by_id(&mut self, id: u32) {
        if (id as usize) < self.generations.len() {
            let gen = self.generations[id as usize];
            self.despawn(Entity::new(id, gen));
        }
    }

    /// Yaşayan (despawn olmamış) tüm Entity'leri döndüren iterator — sıfır allocation
    pub fn iter_alive_entities(&self) -> AliveEntityIter<'_> {
        AliveEntityIter {
            current_id: 0,
            max_id: self.next_entity_id,
            free_set: &self.free_set,
            generations: &self.generations,
        }
    }

    /// Tüm yaşayan entity'leri Vec olarak döndürür (kolaylık metodu)
    pub fn alive_entities(&self) -> Vec<Entity> {
        self.iter_alive_entities().collect()
    }

    #[inline]
    pub fn is_alive(&self, entity: Entity) -> bool {
        let id = entity.id() as usize;
        id < self.generations.len() && self.generations[id] == entity.generation()
    }

    /// Sisteme component ekleme — sıfır bellek israfı
    pub fn add_component<T: Component>(&mut self, entity: Entity, component: T) {
        if !self.is_alive(entity) {
            return;
        }

        let type_id = TypeId::of::<T>();

        let storage = self
            .storages
            .entry(type_id)
            .or_insert_with(|| RefCell::new(Box::new(SparseSet::<T>::new())));

        let mut borrowed = storage.borrow_mut();
        if let Some(sparse_set) = borrowed.as_any_mut().downcast_mut::<SparseSet<T>>() {
            sparse_set.insert(entity.id(), component);
        }

        // Entity → TypeId takibini güncelle (despawn optimizasyonu için).
        // `insert`: ikinci kez aynı tür eklenirse yine tek kayıt (SparseSet zaten üzerine yazar).
        self.entity_components
            .entry(entity.id())
            .or_default()
            .insert(type_id);
    }

    /// Sistemden component silme
    pub fn remove_component<T: Component>(&mut self, entity: Entity) {
        if !self.is_alive(entity) {
            return;
        }

        let type_id = TypeId::of::<T>();

        if let Some(storage) = self.storages.get_mut(&type_id) {
            let mut borrowed = storage.borrow_mut();
            if let Some(sparse_set) = borrowed.as_any_mut().downcast_mut::<SparseSet<T>>() {
                sparse_set.remove(entity.id());
            }
        }

        if let Some(types) = self.entity_components.get_mut(&entity.id()) {
            types.remove(&type_id);
        }
    }

    /// Component dizisine okuma erişimi (Read-Only, Ref ile paylaşılabilir).
    pub fn borrow<T: Component>(&self) -> Option<Ref<'_, SparseSet<T>>> {
        let type_id = TypeId::of::<T>();
        let storage = self.storages.get(&type_id)?;

        match storage.try_borrow() {
            Ok(borrowed) => Some(Ref::map(borrowed, |s| {
                s.as_any().downcast_ref::<SparseSet<T>>().unwrap()
            })),
            Err(_) => {
                panic!(
                    "[ECS] PANIC: borrow<{}> failed — a mutable borrow is already active! \
                    This usually means another query or system is currently holding a mutable \
                    reference to this component type. Fix the aliasing conflict.",
                    std::any::type_name::<T>()
                );
            }
        }
    }

    /// Component dizisine yazma erişimi (Mutable, tekil sahiplik).
    pub fn borrow_mut<T: Component>(&self) -> Option<RefMut<'_, SparseSet<T>>> {
        let type_id = TypeId::of::<T>();
        let storage = self.storages.get(&type_id)?;

        match storage.try_borrow_mut() {
            Ok(borrowed) => Some(RefMut::map(borrowed, |s| {
                s.as_any_mut().downcast_mut::<SparseSet<T>>().unwrap()
            })),
            Err(_) => {
                panic!(
                    "[ECS] PANIC: borrow_mut<{}> failed — another borrow is already active! \
                    This happens when multiple queries attempt to access the same component simultaneously \
                    where at least one is a mutable access. Fix the aliasing conflict.",
                    std::any::type_name::<T>()
                );
            }
        }
    }

    // ==========================================================
    // ERGONOMİK SORGULAR (QUERY API)
    // ==========================================================

    pub fn query<'w, Q: crate::query::WorldQuery<'w>>(&'w self) -> Option<crate::query::Query<'w, Q>> {
        crate::query::Query::new(self)
    }

    /// Toplam yaşayan entity sayısı
    #[inline]
    pub fn entity_count(&self) -> u32 {
        self.next_entity_id - self.free_ids.len() as u32
    }

    // ==========================================================
    // RESOURCE SİSTEMİ (GLOBAL VERİLER)
    // ==========================================================

    /// Sisteme global bir Resource ekler veya üzerine yazar.
    pub fn insert_resource<T: 'static>(&mut self, resource: T) {
        let type_id = TypeId::of::<T>();
        self.resources
            .insert(type_id, RefCell::new(Box::new(resource)));
    }

    /// Global bir Resource'u okumak için çağrılır (Immutable Borrow)
    pub fn get_resource<T: 'static>(&self) -> Option<Ref<'_, T>> {
        let type_id = TypeId::of::<T>();
        let storage = self.resources.get(&type_id)?;

        match storage.try_borrow() {
            Ok(borrowed) => Some(Ref::map(borrowed, |s| {
                s.downcast_ref::<T>().unwrap()
            })),
            Err(_) => {
                crate::gizmo_log!(
                    Warning,
                    "[ECS] get_resource<{}> başarısız — mutable borrow aktif!",
                    std::any::type_name::<T>()
                );
                None
            }
        }
    }

    /// Global bir Resource'u değiştirmek için çağrılır (Mutable Borrow)
    pub fn get_resource_mut<T: 'static>(&self) -> Option<RefMut<'_, T>> {
        let type_id = TypeId::of::<T>();
        let storage = self.resources.get(&type_id)?;

        match storage.try_borrow_mut() {
            Ok(borrowed) => Some(RefMut::map(borrowed, |s| {
                s.downcast_mut::<T>().unwrap()
            })),
            Err(_) => {
                crate::gizmo_log!(
                    Warning,
                    "[ECS] get_resource_mut<{}> başarısız — başka bir borrow aktif!",
                    std::any::type_name::<T>()
                );
                None
            }
        }
    }

    /// Global bir Resource yoksa Default olarak oluşturur, ardından Mutable Borrow döndürür.
    /// World mutable borrow gerektirir, böylece hashmap'e güvenle kayıt yapılabilir.
    pub fn get_resource_mut_or_default<T: Default + 'static>(&mut self) -> RefMut<'_, T> {
        let type_id = TypeId::of::<T>();
        if !self.resources.contains_key(&type_id) {
            self.resources
                .insert(type_id, RefCell::new(Box::new(T::default())));
        }

        let storage = self.resources.get(&type_id).unwrap();
        RefMut::map(storage.borrow_mut(), |s| s.downcast_mut::<T>().unwrap())
    }

    /// Global bir Resource'u ECS'ten tamamen çıkartır ve sahipliğini döndürür
    pub fn remove_resource<T: 'static>(&mut self) -> Option<T> {
        let type_id = TypeId::of::<T>();
        let cell = self.resources.remove(&type_id)?;
        let boxed_any = cell.into_inner();
        match boxed_any.downcast::<T>() {
            Ok(boxed_t) => Some(*boxed_t),
            Err(_) => None,
        }
    }
}

/// Sıfır allocation ile yaşayan entity'ler üzerinde iterasyon yapan iterator.
pub struct AliveEntityIter<'a> {
    current_id: u32,
    max_id: u32,
    free_set: &'a HashSet<u32>,
    generations: &'a Vec<u32>,
}

impl<'a> Iterator for AliveEntityIter<'a> {
    type Item = Entity;

    #[inline]
    fn next(&mut self) -> Option<Entity> {
        while self.current_id < self.max_id {
            let id = self.current_id;
            self.current_id += 1;
            if !self.free_set.contains(&id) {
                let gen = self.generations[id as usize];
                return Some(Entity::new(id, gen));
            }
        }
        None
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = (self.max_id - self.current_id) as usize;
        (0, Some(remaining))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Test component types
    #[derive(Debug, Clone, PartialEq)]
    struct Position {
        x: f32,
        y: f32,
    }

    #[derive(Debug, Clone, PartialEq)]
    struct Health(u32);

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

        assert!(world.borrow::<Position>().unwrap().get(e1.id()).is_some());
        assert!(world.borrow::<Health>().unwrap().get(e1.id()).is_some());

        world.despawn(e1);

        assert!(!world.is_alive(e1));
        assert!(world.borrow::<Position>().unwrap().get(e1.id()).is_none());
        assert!(world.borrow::<Health>().unwrap().get(e1.id()).is_none());
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

        assert!(world.borrow::<Position>().unwrap().get(e2.id()).is_some());
        assert!(world.borrow::<Health>().unwrap().get(e2.id()).is_some());
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
        let alive: Vec<Entity> = world.iter_alive_entities().collect();
        assert_eq!(alive.len(), 2);
        assert!(alive.iter().all(|e| e.id() != e2.id()));
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

        let hp = world.borrow::<Health>().unwrap();
        assert_eq!(hp.get(e.id()).unwrap().0, 50);
    }

    /// Aynı component türü iki kez eklenince entity_components'ta tek TypeId kalmalı;
    /// despawn SparseSet'te remove_entity'yi yalnızca bir kez tetiklemeli (çift silme / panic yok).
    #[test]
    fn test_double_add_component_despawn_safe() {
        let mut world = World::new();
        let e = world.spawn();
        let id = e.id();
        world.add_component(e, Health(100));
        world.add_component(e, Health(50));

        world.despawn(e);

        assert!(!world.is_alive(e));
        assert!(world.borrow::<Health>().unwrap().get(id).is_none());

        // ID yeniden kullanıldığında eski Health taşınmamalı
        let e2 = world.spawn();
        assert_eq!(e2.id(), id);
        assert!(world.borrow::<Health>().unwrap().get(e2.id()).is_none());
    }
}
