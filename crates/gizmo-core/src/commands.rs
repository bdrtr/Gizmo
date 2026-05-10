use crate::component::Component;
use crate::entity::Entity;
use crate::system::{Res, SystemParam};
use crate::world::World;
use std::sync::Arc;

use crossbeam_queue::SegQueue;

type BoxedCommand = Box<dyn FnOnce(&mut World) + Send + Sync>;

/// Otonom iterasyonlar ve sistemler içerisinden güvenli bir şekilde `World`e
/// müdahale etmeyi (spawn, despawn, bileşen ekleme/çıkarma) sağlayan kilitsiz komut kuyruğu.
#[derive(Default, Clone)]
pub struct CommandQueue {
    queue: Arc<SegQueue<BoxedCommand>>,
}

impl CommandQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push<F>(&self, command: F)
    where
        F: FnOnce(&mut World) + Send + Sync + 'static,
    {
        self.queue.push(Box::new(command));
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    pub fn apply(&self, world: &mut World) {
        while let Some(command) = self.queue.pop() {
            command(world);
        }
    }
}

/// Sistem imzasında kullanılabilecek `Commands` parametresi.
pub struct Commands<'w> {
    pub queue: Res<'w, CommandQueue>,
    pub entities: Res<'w, crate::entity::allocator::Entities>,
}

impl SystemParam for Commands<'static> {
    type Item<'w> = Commands<'w>;

    fn fetch<'w>(
        world: &'w World,
        dt: f32,
    ) -> Result<Self::Item<'w>, crate::system::SystemParamFetchError> {
        let queue = <Res<'static, CommandQueue> as SystemParam>::fetch(world, dt)?;
        let entities =
            <Res<'static, crate::entity::allocator::Entities> as SystemParam>::fetch(world, dt)?;
        Ok(Commands { queue, entities })
    }

    fn get_access_info(info: &mut crate::system::AccessInfo) {
        <Res<'static, CommandQueue> as SystemParam>::get_access_info(info);
        <Res<'static, crate::entity::allocator::Entities> as SystemParam>::get_access_info(info);
    }
}

impl<'w> Commands<'w> {
    /// Yeni bir entity oluşturur ve onun üzerine eklentiler yapmak için `EntityCommands` döndürür.
    pub fn spawn(&mut self) -> EntityCommands<'_, 'w> {
        let entity = self.entities.reserve_entity();

        self.queue.push(move |world| {
            world.flush_spawn(entity);
        });

        EntityCommands {
            entity,
            commands: self,
        }
    }

    /// Var olan bir entity üzerinde işlemler yapmak için `EntityCommands` alır.
    pub fn entity(&mut self, entity: Entity) -> EntityCommands<'_, 'w> {
        EntityCommands {
            entity,
            commands: self,
        }
    }
}

pub struct EntityCommands<'a, 'w> {
    entity: Entity,
    commands: &'a mut Commands<'w>,
}

impl<'a, 'w> EntityCommands<'a, 'w> {
    /// Bu komut tamponunun hedeflendiği native Entity ID'sini döndürür.
    pub fn id(&self) -> Entity {
        self.entity
    }

    /// Entity'ye yeni bir bileşen ekler (Entity o an veya sonradan oluşur olsun fark etmez)
    pub fn insert<T: Component>(&mut self, component: T) -> &mut Self {
        let e = self.entity;
        self.commands.queue.push(move |world| {
            world.add_component(e, component);
        });
        self
    }

    /// Entity'den bir bileşen çıkarır
    pub fn remove<T: Component>(&mut self) -> &mut Self {
        let e = self.entity;
        self.commands.queue.push(move |world| {
            world.remove_component::<T>(e);
        });
        self
    }

    /// Entity'yi tamamen yok eder
    pub fn despawn(&mut self) {
        let e = self.entity;
        self.commands.queue.push(move |world| {
            world.despawn(e);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::Schedule;

    use crate::world::World;

    #[derive(Clone, PartialEq, Debug)]
    struct ComponentA(i32);
    impl Component for ComponentA {}

    #[derive(Clone, PartialEq, Debug)]
    struct ComponentB(f32);
    impl Component for ComponentB {}

    #[test]
    fn test_command_queue_push_and_apply() {
        let mut world = World::new();
        let queue = CommandQueue::new();

        queue.push(|w| {
            let e = w.spawn();
            w.add_component(e, ComponentA(42));
        });

        // Apply öncesi entity yok
        assert_eq!(world.entity_count(), 0);

        queue.apply(&mut world);

        // Apply sonrası 1 entity var ve componenti eklenmiş
        assert_eq!(world.entity_count(), 1);

        let mut count = 0;
        if let Some(q) = world.query::<&ComponentA>() {
            for (_, c) in q.iter() {
                assert_eq!(c.0, 42);
                count += 1;
            }
        }
        assert_eq!(count, 1);
    }

    #[test]
    fn test_commands_system_spawn_and_insert() {
        let mut world = World::new();
        let mut schedule = Schedule::new();

        schedule.add_di_system::<(Commands<'static>,), _>(|mut commands: Commands| {
            commands
                .spawn()
                .insert(ComponentA(100))
                .insert(ComponentB(3.14));
        });

        schedule.run(&mut world, 0.1);

        let mut count = 0;
        if let Some(q) = world.query::<(&ComponentA, &ComponentB)>() {
            for (_, (ca, cb)) in q.iter() {
                assert_eq!(ca.0, 100);
                assert_eq!(cb.0, 3.14);
                count += 1;
            }
        }
        assert_eq!(count, 1);
    }

    #[test]
    fn test_commands_system_despawn() {
        let mut world = World::new();

        let e1 = world.spawn();
        world.add_component(e1, ComponentA(10));

        let e2 = world.spawn();
        world.add_component(e2, ComponentA(20));

        let mut schedule = Schedule::new();

        // Use a standard (&World, f32) system to access query and manually fetch Commands
        schedule.add_system(|world: &World, dt: f32| {
            let mut commands = Commands::fetch(world, dt).unwrap();
            if let Some(q) = world.query::<&ComponentA>() {
                for (id, c) in q.iter() {
                    if c.0 == 10 {
                        commands.entity(Entity::new(id, 0)).despawn();
                    }
                }
            }
        });

        schedule.run(&mut world, 0.1);

        assert_eq!(world.entity_count(), 1);
        if let Some(q) = world.query::<&ComponentA>() {
            for (_, c) in q.iter() {
                assert_eq!(c.0, 20);
            }
        }
    }

    #[test]
    fn test_commands_system_remove_component() {
        let mut world = World::new();

        let e = world.spawn();
        world.add_component(e, ComponentA(1));
        world.add_component(e, ComponentB(2.0));

        let mut schedule = Schedule::new();

        schedule.add_system(|world: &World, dt: f32| {
            let mut commands = Commands::fetch(world, dt).unwrap();
            if let Some(q) = world.query::<&ComponentA>() {
                for (id, _) in q.iter() {
                    commands.entity(Entity::new(id, 0)).remove::<ComponentA>();
                }
            }
        });

        schedule.run(&mut world, 0.1);

        assert_eq!(world.entity_count(), 1);

        let mut has_a = false;
        if let Some(q) = world.query::<&ComponentA>() {
            has_a = q.iter().count() > 0;
        }
        assert!(!has_a, "ComponentA still exists!");

        let mut has_b = false;
        if let Some(q) = world.query::<&ComponentB>() {
            has_b = q.iter().count() > 0;
        }
        assert!(has_b, "ComponentB was unexpectedly removed!");
    }
}
