use std::any::TypeId;
use std::collections::{HashMap, HashSet};
use std::{cell::RefCell, cell::{Ref, RefMut}};

use crate::entity::Entity;
use crate::component::{Component, ComponentStorage, SparseSet};

pub struct World {
    next_entity_id: u32,
    generations: Vec<u32>,
    free_ids: Vec<u32>,
    free_set: HashSet<u32>,  // O(1) contains() kontrolü için
    // RefCell: Aynı anda farklı Component dizilerini eşzamanlı olarak ödünç alabilmek (Borrow) için çok kritiktir.
    storages: HashMap<TypeId, RefCell<Box<dyn ComponentStorage>>>,
    // Entity'den bağımsız global veriler (Time, WindowSize, Input vs.)
    resources: HashMap<TypeId, RefCell<Box<dyn std::any::Any>>>,
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

    pub fn despawn(&mut self, entity: Entity) {
        if self.is_alive(entity) {
            let id = entity.id();
            self.generations[id as usize] += 1;
            self.free_ids.push(id);
            self.free_set.insert(id);
            
            for storage in self.storages.values_mut() {
                storage.get_mut().remove_entity(id);
            }
        }
    }

    pub fn despawn_by_id(&mut self, id: u32) {
        if (id as usize) < self.generations.len() {
            let gen = self.generations[id as usize];
            self.despawn(Entity::new(id, gen));
        }
    }

    /// Yaşayan (despawn olmamış) tüm Entity'leri döndürür — O(n) artık, eskisi O(n²) idi
    pub fn iter_alive_entities(&self) -> Vec<Entity> {
        let mut alive = Vec::with_capacity((self.next_entity_id as usize).saturating_sub(self.free_set.len()));
        for id in 0..self.next_entity_id {
            if !self.free_set.contains(&id) {
                let gen = self.generations[id as usize];
                alive.push(Entity::new(id, gen));
            }
        }
        alive
    }

    #[inline]
    pub fn is_alive(&self, entity: Entity) -> bool {
        let id = entity.id() as usize;
        id < self.generations.len() && self.generations[id] == entity.generation()
    }

    /// Sisteme component ekleme — sıfır bellek israfı
    pub fn add_component<T: Component>(&mut self, entity: Entity, component: T) {
        if !self.is_alive(entity) { return; }

        let type_id = TypeId::of::<T>();
        
        let storage = self.storages.entry(type_id).or_insert_with(|| {
            RefCell::new(Box::new(SparseSet::<T>::new()))
        });

        let mut borrowed = storage.borrow_mut();
        if let Some(sparse_set) = borrowed.as_any_mut().downcast_mut::<SparseSet<T>>() {
            sparse_set.insert(entity.id(), component);
        }
    }

    /// Component dizisine okuma erişimi (Read-Only, Ref ile paylaşılabilir).
    pub fn borrow<T: Component>(&self) -> Option<Ref<'_, SparseSet<T>>> {
        let type_id = TypeId::of::<T>();
        let storage = self.storages.get(&type_id)?;
        
        Some(Ref::map(storage.borrow(), |s| {
            s.as_any().downcast_ref::<SparseSet<T>>().unwrap()
        }))
    }

    /// Component dizisine yazma erişimi (Mutable, tekil sahiplik).
    pub fn borrow_mut<T: Component>(&self) -> Option<RefMut<'_, SparseSet<T>>> {
        let type_id = TypeId::of::<T>();
        let storage = self.storages.get(&type_id)?;
        
        Some(RefMut::map(storage.borrow_mut(), |s| {
            s.as_any_mut().downcast_mut::<SparseSet<T>>().unwrap()
        }))
    }

    // ==========================================================
    // ERGONOMİK SORGULAR (QUERY API)
    // ==========================================================

    pub fn query_mut<T1: Component>(&self) -> Option<crate::query::QueryMut<'_, T1>> {
        crate::query::QueryMut::new(self)
    }

    pub fn query_ref<T1: Component>(&self) -> Option<crate::query::QueryRef<'_, T1>> {
        crate::query::QueryRef::new(self)
    }

    pub fn query_mut_ref<T1: Component, T2: Component>(&self) -> Option<crate::query::QueryMutRef<'_, T1, T2>> {
        crate::query::QueryMutRef::new(self)
    }

    pub fn query_mut_mut<T1: Component, T2: Component>(&self) -> Option<crate::query::QueryMutMut<'_, T1, T2>> {
        crate::query::QueryMutMut::new(self)
    }

    pub fn query_ref_ref<T1: Component, T2: Component>(&self) -> Option<crate::query::QueryRefRef<'_, T1, T2>> {
        crate::query::QueryRefRef::new(self)
    }

    pub fn query_mut_ref_ref<T1: Component, T2: Component, T3: Component>(&self) -> Option<crate::query::QueryMutRefRef<'_, T1, T2, T3>> {
        crate::query::QueryMutRefRef::new(self)
    }

    pub fn query_ref_ref_ref<T1: Component, T2: Component, T3: Component>(&self) -> Option<crate::query::QueryRefRefRef<'_, T1, T2, T3>> {
        crate::query::QueryRefRefRef::new(self)
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
        self.resources.insert(type_id, RefCell::new(Box::new(resource)));
    }

    /// Global bir Resource'u okumak için çağrılır (Immutable Borrow)
    pub fn get_resource<T: 'static>(&self) -> Option<Ref<'_, T>> {
        let type_id = TypeId::of::<T>();
        let storage = self.resources.get(&type_id)?;
        
        Some(Ref::map(storage.borrow(), |s| {
            s.downcast_ref::<T>().unwrap()
        }))
    }

    /// Global bir Resource'u değiştirmek için çağrılır (Mutable Borrow)
    pub fn get_resource_mut<T: 'static>(&self) -> Option<RefMut<'_, T>> {
        let type_id = TypeId::of::<T>();
        let storage = self.resources.get(&type_id)?;
        
        Some(RefMut::map(storage.borrow_mut(), |s| {
            s.downcast_mut::<T>().unwrap()
        }))
    }
}
