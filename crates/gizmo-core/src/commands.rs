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
