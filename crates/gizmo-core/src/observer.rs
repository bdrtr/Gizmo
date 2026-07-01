use crate::entity::Entity;
use std::marker::PhantomData;

#[derive(Clone, Copy)]
pub struct Insert;
#[derive(Clone, Copy)]
pub struct Remove;
#[derive(Clone, Copy)]
pub struct Replace;

pub trait EntityEvent: Send + Sync + 'static + Clone {
    fn target(&self) -> Entity;
    fn can_propagate(&self) -> bool { false }
}

pub struct On<E, T = ()> {
    pub event: E,
    pub entity: Entity,
    pub _marker: PhantomData<T>,
}

impl<E: Clone, T> Clone for On<E, T> {
    fn clone(&self) -> Self {
        Self {
            event: self.event.clone(),
            entity: self.entity,
            _marker: PhantomData,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::World;
    use crate::component::Component;
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    #[allow(dead_code)]
    struct Health(f32);

    #[derive(Clone)]
    struct Poison;

    impl Component for Health {}
    impl Component for Poison {}

    #[test]
    fn test_observer_on_insert() {
        let mut world = World::new();
        let counter = Arc::new(Mutex::new(0));
        let counter_clone = counter.clone();

        // Register observer that increments the counter whenever a Health component is inserted.
        world.add_observer(move |_event: On<Insert, Health>| {
            *counter_clone.lock().unwrap() += 1;
        });

        // Spawning an entity with Health should trigger the observer
        world.spawn_bundle((Health(100.0),));
        assert_eq!(*counter.lock().unwrap(), 1);

        // Spawning another entity without Health should NOT trigger
        let e = world.spawn_bundle((Poison,));
        assert_eq!(*counter.lock().unwrap(), 1);

        // Adding Health later should trigger the observer
        world.add_component(e, Health(50.0));
        assert_eq!(*counter.lock().unwrap(), 2);
    }

    #[test]
    fn test_component_hooks_directly() {
        let mut world = World::new();
        let removed_counter = Arc::new(Mutex::new(0));
        let rc_clone = removed_counter.clone();

        world.component_hooks.entry(std::any::TypeId::of::<Poison>()).or_default().on_remove.push(Box::new(move |_w, _e| {
            *rc_clone.lock().unwrap() += 1;
        }));

        let e = world.spawn_bundle((Poison,));
        assert_eq!(*removed_counter.lock().unwrap(), 0);

        // Removing the component triggers on_remove hook
        world.remove_component::<Poison>(e);
        assert_eq!(*removed_counter.lock().unwrap(), 1);

        // Despawning an entity with the component triggers on_remove hook
        let e2 = world.spawn_bundle((Poison,));
        world.despawn(e2);
        assert_eq!(*removed_counter.lock().unwrap(), 2);
    }
}
