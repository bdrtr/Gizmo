use super::{AddHook, DespawnHook, RemoveHook, SetHook, World};
use crate::archetype::ComponentInfo;
use crate::component::Component;
use crate::entity::Entity;

use std::any::TypeId;
use std::collections::HashMap;

impl World {
    /// Belirli bir component turunun runtime metadata'sini kaydeder.
    /// Archetype storage migration asamalarinda column olusturma icin kullanilir.
    #[inline]
    pub fn register_component_type<T: Component>(&mut self) {
        let type_id = TypeId::of::<T>();
        self.component_infos
            .entry(type_id)
            .or_insert_with(ComponentInfo::of::<T>);
    }

    /// Registers a Component hook (Observer) for `OnInsert`.
    pub fn add_observer<T: Component, F>(&mut self, mut system: F) -> &mut Self
    where
        F: FnMut(crate::observer::On<crate::observer::Insert, T>) + Send + Sync + 'static,
    {
        let type_id = TypeId::of::<T>();
        let mut hooks = self.component_hooks.remove(&type_id).unwrap_or_default();

        hooks.on_add.push(Box::new(move |_world, entity| {
            let event = crate::observer::On {
                event: crate::observer::Insert,
                entity,
                _marker: std::marker::PhantomData,
            };
            system(event);
        }));

        self.component_hooks.insert(type_id, hooks);
        self
    }

    /// Özel EntityEvent'ler için Entity bazlı Observer kaydı
    pub fn observe<E: crate::observer::EntityEvent, F>(&mut self, entity: Entity, listener: F) -> &mut Self
    where
        F: FnMut(crate::observer::On<E>) + Send + Sync + 'static,
    {
        let type_id = TypeId::of::<E>();
        let map_any = self.entity_observers.entry(type_id).or_insert_with(|| {
            Box::new(HashMap::<Entity, Vec<Box<dyn FnMut(crate::observer::On<E>) + Send + Sync + 'static>>>::new())
        });

        let map = map_any.downcast_mut::<HashMap<Entity, Vec<Box<dyn FnMut(crate::observer::On<E>) + Send + Sync + 'static>>>>().unwrap();
        map.entry(entity).or_default().push(Box::new(listener));
        self
    }

    /// Bir Event'i tetikler ve hiyerarşide yukarı doğru yayar (bubble-up)
    pub fn trigger<E: crate::observer::EntityEvent>(&mut self, event: E) {
        use crate::component::Parent;
        let mut current_entity = event.target();

        loop {
            // Observer'ları bu entity için bul ve çalıştır
            let mut hooks_to_run = Vec::new();

            if let Some(map_any) = self.entity_observers.get_mut(&TypeId::of::<E>()) {
                if let Some(map) = map_any.downcast_mut::<HashMap<Entity, Vec<Box<dyn FnMut(crate::observer::On<E>) + Send + Sync + 'static>>>>() {
                    if let Some(listeners) = map.remove(&current_entity) {
                        hooks_to_run = listeners;
                    }
                }
            }

            for mut listener in hooks_to_run.drain(..) {
                let e = crate::observer::On {
                    event: event.clone(),
                    entity: current_entity,
                    _marker: std::marker::PhantomData,
                };
                listener(e);

                // Geri koy
                if let Some(map_any) = self.entity_observers.get_mut(&TypeId::of::<E>()) {
                    if let Some(map) = map_any.downcast_mut::<HashMap<Entity, Vec<Box<dyn FnMut(crate::observer::On<E>) + Send + Sync + 'static>>>>() {
                        map.entry(current_entity).or_default().push(listener);
                    }
                }
            }

            if !event.can_propagate() {
                break;
            }

            // Propagate to parent
            if let Some(parent_ptr) = self.get_component_ptr(current_entity, TypeId::of::<Parent>()) {
                current_entity = self.entity(unsafe { (*(parent_ptr as *const Parent)).0 }).unwrap();
            } else {
                break;
            }
        }
    }

    /// Belirli bir component turu kayitli mi?
    #[inline]
    pub fn is_component_registered<T: Component>(&self) -> bool {
        self.component_infos.contains_key(&TypeId::of::<T>())
    }

    /// Kayitli component metadata sayisi.
    #[inline]
    pub fn registered_component_count(&self) -> usize {
        self.component_infos.len()
    }

    pub fn register_on_add<T: Component>(&mut self, hook: AddHook) {
        self.component_hooks
            .entry(TypeId::of::<T>())
            .or_default()
            .on_add
            .push(hook);
    }

    pub fn register_on_remove<T: Component>(&mut self, hook: RemoveHook) {
        self.component_hooks
            .entry(TypeId::of::<T>())
            .or_default()
            .on_remove
            .push(hook);
    }

    pub fn register_on_set<T: Component>(&mut self, hook: SetHook) {
        self.component_hooks
            .entry(TypeId::of::<T>())
            .or_default()
            .on_set
            .push(hook);
    }

    pub fn register_despawn_hook(&mut self, hook: DespawnHook) {
        self.despawn_hooks.push(hook);
    }
}
