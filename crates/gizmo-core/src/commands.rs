use crate::component::Component;
use crate::entity::Entity;
use crate::system::{Res, SystemParam};
use crate::world::World;
use std::sync::{Arc, RwLock};

type BoxedCommand = Box<dyn FnOnce(&mut World) + Send + Sync>;

/// Otonom iterasyonlar ve sistemler içerisinden güvenli bir şekilde `World`e
/// müdahale etmeyi (spawn, despawn, bileşen ekleme/çıkarma) sağlayan komut kuyruğu.
#[derive(Default, Clone)]
pub struct CommandQueue {
    queue: Arc<RwLock<Vec<BoxedCommand>>>,
}

impl CommandQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push<F>(&self, command: F)
    where
        F: FnOnce(&mut World) + Send + Sync + 'static,
    {
        self.queue.write().unwrap().push(Box::new(command));
    }

    pub(crate) fn take_all(&self) -> Vec<BoxedCommand> {
        std::mem::take(&mut *self.queue.write().unwrap())
    }

    pub fn apply(&self, world: &mut World) {
        let commands = self.take_all();
        for command in commands {
            command(world);
        }
    }
}

/// Sistem imzasında kullanılabilecek `Commands` parametresi.
pub struct Commands<'w> {
    pub queue: Res<'w, CommandQueue>,
}

impl SystemParam for Commands<'static> {
    type Item<'w> = Commands<'w>;

    fn fetch<'w>(world: &'w World, dt: f32) -> Option<Self::Item<'w>> {
        let queue = <Res<'static, CommandQueue> as SystemParam>::fetch(world, dt)?;
        Some(Commands { queue })
    }
}

impl<'w> Commands<'w> {
    /// Yeni bir entity oluşturur ve onun üzerine eklentiler yapmak için `EntityCommands` döndürür.
    pub fn spawn(&mut self) -> EntityCommands<'_, 'w> {
        let entity_box = Arc::new(RwLock::new(None));
        let cloned_box = entity_box.clone();

        self.queue.push(move |world| {
            let e = world.spawn();
            *cloned_box.write().unwrap() = Some(e);
        });

        EntityCommands {
            entity_opt: entity_box,
            commands: self,
        }
    }

    /// Var olan bir entity üzerinde işlemler yapmak için `EntityCommands` alır.
    pub fn entity(&mut self, entity: Entity) -> EntityCommands<'_, 'w> {
        EntityCommands {
            entity_opt: Arc::new(RwLock::new(Some(entity))),
            commands: self,
        }
    }
}

pub struct EntityCommands<'a, 'w> {
    entity_opt: Arc<RwLock<Option<Entity>>>,
    commands: &'a mut Commands<'w>,
}

impl<'a, 'w> EntityCommands<'a, 'w> {
    /// Entity'ye yeni bir bileşen ekler (Entity o an veya sonradan oluşur olsun fark etmez)
    pub fn insert<T: Component>(&mut self, component: T) -> &mut Self {
        let entity_box = self.entity_opt.clone();
        self.commands.queue.push(move |world| {
            if let Some(e) = *entity_box.read().unwrap() {
                world.add_component(e, component);
            }
        });
        self
    }

    /// Entity'den bir bileşen çıkarır
    pub fn remove<T: Component>(&mut self) -> &mut Self {
        let entity_box = self.entity_opt.clone();
        self.commands.queue.push(move |world| {
            if let Some(e) = *entity_box.read().unwrap() {
                world.remove_component::<T>(e);
            }
        });
        self
    }

    /// Entity'yi tamamen yok eder
    pub fn despawn(&mut self) {
        let entity_box = self.entity_opt.clone();
        self.commands.queue.push(move |world| {
            if let Some(e) = *entity_box.read().unwrap() {
                world.despawn(e);
            }
        });
    }
}
