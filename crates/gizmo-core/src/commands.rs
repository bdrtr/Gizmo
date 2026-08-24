//! Deferred structural mutation: a lock-free queue of `FnOnce(&mut World)` closures
//! ([`CommandQueue`]) and the [`Commands`] system parameter that pushes into it.
//!
//! A system is only ever handed `&World`, so it cannot spawn, despawn or add/remove
//! components while it runs. It enqueues the change instead, and the queue is drained later
//! under `&mut World`, at a point where nothing is iterating — see
//! [`CommandQueue::apply`] and [`World::apply_commands`].
//!
//! What follows from that, and is worth knowing before relying on any of it:
//!
//! * Nothing enqueued is observable until the flush — not even to the system that queued it.
//! * Within one flush, commands are applied in push order, which is why
//!   `spawn().insert(..)` works.
//! * Two systems co-scheduled in the same batch hold only a *shared* borrow of the queue —
//!   that is exactly what lets them share a batch (see [`Commands::queue`]) — so their
//!   pushes interleave in thread-scheduling order. Anything whose outcome depends on which
//!   of them landed first is not reproducible run to run and must be split across batches.
//! * Entity ids from [`Commands::spawn`] are reserved eagerly, from the same shared
//!   allocator, and carry the same caveat.

use crate::component::Component;
use crate::entity::Entity;
use crate::system::{Res, SystemParam};
use crate::world::World;
use std::sync::Arc;

use crossbeam_queue::SegQueue;

type BoxedCommand = Box<dyn FnOnce(&mut World) + Send + Sync>;

/// The lock-free command queue that makes it possible to intervene in the `World` safely
/// (spawn, despawn, adding/removing components) from within autonomous iterations and systems.
#[derive(Default, Clone)]
pub struct CommandQueue {
    queue: Arc<SegQueue<BoxedCommand>>,
}

impl CommandQueue {
    /// Creates a queue holding no commands and sharing storage with nothing else.
    ///
    /// Cloning a `CommandQueue` clones an `Arc`: every clone pushes into and drains the
    /// *same* queue. `new()` and the derived `Default` are the only constructors — the
    /// storage field is private, so every other way of obtaining a `CommandQueue` value is a
    /// clone, and therefore an alias of an existing queue. A world built by
    /// [`World::new`] already carries a `CommandQueue` resource, so inserting a fresh one
    /// replaces that resource and drops whatever commands it still held, unapplied.
    pub fn new() -> Self {
        Self::default()
    }

    /// Enqueues a closure to be run later against `&mut World`; nothing executes here.
    ///
    /// Takes `&self`, which is the whole point: systems holding only a shared borrow of the
    /// queue can request structural changes while other systems iterate the world. The
    /// closure is boxed, so pushing allocates. It is retained until an [`apply`](Self::apply)
    /// drains it — including across frames if nobody flushes — and it runs even if the
    /// entity it names has been despawned in the meantime.
    ///
    /// Ordering: commands pushed from one thread are applied in push order. For pushes from
    /// systems running concurrently, see the [module docs](crate::commands).
    pub fn push<F>(&self, command: F)
    where
        F: FnOnce(&mut World) + Send + Sync + 'static,
    {
        self.queue.push(Box::new(command));
    }

    /// Whether the queue holds no pending commands *at this instant*.
    ///
    /// With other threads pushing, the answer can already be stale by the time it is
    /// returned. It is meant as a cheap way to skip an `apply` — a command missed that way
    /// is only delayed to the next flush, never lost — not as a synchronisation point.
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Runs every queued command against `world` in push order, on the calling thread, and
    /// leaves the queue empty.
    ///
    /// The drain loop pops until the queue is exhausted, so a command that pushes onto this
    /// same queue is executed within the same call, after everything already queued — a
    /// command that unconditionally re-queues itself never terminates. Commands see the
    /// effects of all commands applied before them, so `spawn` followed by `insert` works.
    ///
    /// Each command is `FnOnce`, so it is consumed as it is popped: if one panics, the
    /// panic propagates out of `apply` and the commands after it stay queued and are applied
    /// by the next call.
    pub fn apply(&self, world: &mut World) {
        while let Some(command) = self.queue.pop() {
            command(world);
        }
    }
}

/// The `Commands` parameter that can be used in a system signature.
pub struct Commands<'w> {
    /// Shared borrow of the world's [`CommandQueue`] resource — the sink every method on
    /// `Commands`/[`EntityCommands`] pushes into.
    ///
    /// A *shared* borrow is enough because [`CommandQueue::push`] only needs `&self`; that
    /// is why several systems can each hold a `Commands` and why the scheduler is free to
    /// run them in one batch (they declare a read, not a write, of this resource).
    pub queue: Res<'w, CommandQueue>,
    /// Shared borrow of the entity-id allocator, used by [`Commands::spawn`] to reserve an
    /// id immediately (the allocator locks internally, so `&self` suffices).
    ///
    /// The allocator is global to the world, so concurrent reservations are safe — but they
    /// carry the reproducibility caveat in the [module docs](crate::commands).
    pub entities: Res<'w, crate::entity::allocator::Entities>,
}

impl crate::system::sealed::Sealed for Commands<'static> {}
impl SystemParam for Commands<'static> {
    type Item<'w> = Commands<'w>;

    type State = ();
    fn fetch<'w>(
        world: &'w World,
        dt: f32,
        _state: &'w mut (),
    ) -> Result<Self::Item<'w>, crate::system::SystemParamFetchError> {
        let queue = <Res<'static, CommandQueue> as SystemParam>::fetch_stateless(world, dt)?;
        let entities =
            <Res<'static, crate::entity::allocator::Entities> as SystemParam>::fetch_stateless(world, dt)?;
        Ok(Commands { queue, entities })
    }

    fn get_access_info(info: &mut crate::system::AccessInfo) {
        <Res<'static, CommandQueue> as SystemParam>::get_access_info(info);
        <Res<'static, crate::entity::allocator::Entities> as SystemParam>::get_access_info(info);
    }
}

impl<'w> Commands<'w> {
    /// Creates a new entity and returns an `EntityCommands` for making additions on top of it.
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

    /// Gets an `EntityCommands` for performing operations on an existing entity.
    pub fn entity(&mut self, entity: Entity) -> EntityCommands<'_, 'w> {
        EntityCommands {
            entity,
            commands: self,
        }
    }
}

/// Builder for the deferred commands aimed at one entity, produced by
/// [`Commands::spawn`] or [`Commands::entity`].
///
/// Every method only *enqueues*: nothing is observable in the world until the queue is
/// applied (see [`CommandQueue::apply`]). Calls are enqueued in the order they are made and
/// applied in that order, so chaining `spawn().insert(..).insert(..)` materialises the entity
/// before its components.
///
/// The target entity is captured by value and never validated. If it is dead by the time
/// the queue is applied — despawned meanwhile, or a handle from an older generation of a
/// recycled id — `insert`, `remove` and `despawn` are silent no-ops rather than errors.
///
/// It borrows the parent `Commands` mutably, so only one `EntityCommands` may be alive at a
/// time; finish with one entity before addressing the next.
pub struct EntityCommands<'a, 'w> {
    entity: Entity,
    commands: &'a mut Commands<'w>,
}

impl<'a, 'w> EntityCommands<'a, 'w> {
    /// Returns the native Entity ID this command buffer is aimed at.
    pub fn id(&self) -> Entity {
        self.entity
    }

    /// Adds a new component to the Entity (it makes no difference whether the Entity comes
    /// into being at that moment or later)
    pub fn insert<T: Component>(&mut self, component: T) -> &mut Self {
        let e = self.entity;
        self.commands.queue.push(move |world| {
            world.add_component(e, component);
        });
        self
    }

    /// Removes a component from the Entity
    pub fn remove<T: Component>(&mut self) -> &mut Self {
        let e = self.entity;
        self.commands.queue.push(move |world| {
            world.remove_component::<T>(e);
        });
        self
    }

    /// Destroys the Entity entirely
    pub fn despawn(&mut self) {
        let e = self.entity;
        self.commands.queue.push(move |world| {
            world.despawn(e);
        });
    }

    /// Destroys the Entity and all the children beneath it (recursive)
    pub fn despawn_recursive(&mut self) {
        use crate::hierarchy::HierarchyExt;
        let e = self.entity;
        self.commands.queue.push(move |world| {
            world.despawn_recursive(e);
        });
    }

    /// Adds a child to this entity
    pub fn add_child(&mut self, child: Entity) -> &mut Self {
        use crate::hierarchy::HierarchyExt;
        let p = self.entity;
        self.commands.queue.push(move |world| {
            world.add_child(p, child);
        });
        self
    }

    /// Detaches a child from this entity
    pub fn remove_child(&mut self, child: Entity) -> &mut Self {
        use crate::hierarchy::HierarchyExt;
        let p = self.entity;
        self.commands.queue.push(move |world| {
            world.remove_child(p, child);
        });
        self
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
                .insert(ComponentB(2.5));
        });

        schedule.run(&mut world, 0.1);

        let mut count = 0;
        if let Some(q) = world.query::<(&ComponentA, &ComponentB)>() {
            for (_, (ca, cb)) in q.iter() {
                assert_eq!(ca.0, 100);
                assert_eq!(cb.0, 2.5);
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
            let mut commands = Commands::fetch_stateless(world, dt).unwrap();
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
            let mut commands = Commands::fetch_stateless(world, dt).unwrap();
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
