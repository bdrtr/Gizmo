use std::any::TypeId;
use std::{cell::RefCell, collections::HashMap};
use std::cell::{Ref, RefMut};

use crate::entity::Entity;
use crate::component::{Component, ComponentStorage, SparseSet};

pub struct World {
    next_entity_id: u32,
    generations: Vec<u32>,
    free_ids: Vec<u32>,
    // RefCell: Aynı anda farklı Component dizilerini eşzamanlı olarak ödünç alabilmek (Borrow) için çok kritiktir.
    storages: HashMap<TypeId, RefCell<Box<dyn ComponentStorage>>>,
}

impl World {
    pub fn new() -> Self {
        Self {
            next_entity_id: 0,
            generations: Vec::new(),
            free_ids: Vec::new(),
            storages: HashMap::new(),
        }
    }

    pub fn spawn(&mut self) -> Entity {
        if let Some(id) = self.free_ids.pop() {
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
            // Şimdilik sadece ID boşa çıkıyor, tam silme için storagelara da haber gitmeli.
        }
    }

    pub fn is_alive(&self, entity: Entity) -> bool {
        let id = entity.id() as usize;
        id < self.generations.len() && self.generations[id] == entity.generation()
    }

    // Sisteme garantili component ekleme
    pub fn add_component<T: Component>(&mut self, entity: Entity, component: T) {
        if !self.is_alive(entity) { return; }

        let type_id = TypeId::of::<T>();
        
        let storage = self.storages.entry(type_id).or_insert_with(|| {
            RefCell::new(Box::new(SparseSet::<T>::new()))
        });

        // RefCell üzerinden borrow_mut ile mutlak sahiplik alıyoruz
        let mut borrowed = storage.borrow_mut();
        if let Some(sparse_set) = borrowed.as_any_mut().downcast_mut::<SparseSet<T>>() {
            sparse_set.insert(entity.id(), component);
        }
    }

    /// Bir component dizisine okuma amaçlı (Read-Only) erişim verir.
    /// Başka sistemler de aynı anda okuyabilir (Ref).
    pub fn borrow<T: Component>(&self) -> Option<Ref<SparseSet<T>>> {
        let type_id = TypeId::of::<T>();
        let storage = self.storages.get(&type_id)?;
        
        // Ref::map ile Box<dyn ComponentStorage> tipini -> SparseSet<T> tipine çevirerek geri dönüyoruz
        Some(Ref::map(storage.borrow(), |s| {
            s.as_any().downcast_ref::<SparseSet<T>>().unwrap()
        }))
    }

    /// Bir component dizisine yazma amaçlı (Mutable) erişim verir.
    /// Çalışırken kimse bu veriyi okuyamaz veya yazamaz. Tip güvenliği!
    pub fn borrow_mut<T: Component>(&self) -> Option<RefMut<SparseSet<T>>> {
        let type_id = TypeId::of::<T>();
        let storage = self.storages.get(&type_id)?;
        
        Some(RefMut::map(storage.borrow_mut(), |s| {
            s.as_any_mut().downcast_mut::<SparseSet<T>>().unwrap()
        }))
    }
}
