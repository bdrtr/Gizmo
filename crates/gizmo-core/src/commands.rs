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
//! * Two systems that take `Commands` never share a batch: the parameter declares a write of the
//!   queue and of the id allocator, so commands are queued, and entity ids from
//!   [`Commands::spawn`] reserved, in the schedule's order — the same on every run. Until
//!   2026-11-14 it declared reads, and both came out in thread-scheduling order.

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

    /// Drops every pending command without running any of them, and returns how many went.
    ///
    /// This is the teardown counterpart to [`apply`](Self::apply), and it exists for
    /// [`World::clear_entities`](crate::world::World::clear_entities). A queued command names
    /// entities that no longer exist after a clear — and worse than that, `clear_entities` resets
    /// the generation counter, so the ids come back **bit for bit**. A command holding
    /// `Entity(0, gen 0)` does not miss harmlessly after a clear; it lands on whatever unrelated
    /// entity is spawned first.
    ///
    /// Discarding rather than applying is the deliberate choice: applying would run a queued
    /// `despawn` and with it the `on_remove` hooks that `clear_entities` documents itself as not
    /// running, so "finish the queue first" would quietly reintroduce the thing that contract
    /// rules out. A caller who does want the pending work to happen should call
    /// [`World::apply_commands`](crate::world::World::apply_commands) *before* the clear, while
    /// the entities those commands name are still the ones they meant.
    ///
    /// Every clone shares one `Arc`, so this empties the queue for all of them — which is the
    /// same queue by definition, not a surprise.
    ///
    /// Concurrent pushes are not fenced out: this pops until the queue reports empty, so a
    /// command pushed by another thread mid-drain may survive. `clear_entities` takes
    /// `&mut World`, so nothing else is running there.
    ///
    /// **It takes `&self`, so anything holding a `Commands` can reach it** — including a system
    /// running beside others that have already queued work, whose commands this would throw
    /// away. And a command that reaches `World::clear_entities` while [`apply`](Self::apply) is
    /// draining cancels the rest of its OWN flush, because `apply` pops from the queue this
    /// just emptied. Neither is a reason for `&mut self` (the queue is an `Arc` and every clone
    /// is the same queue, so exclusivity would be a lie), but both are reasons to treat this as
    /// a teardown operation rather than a general-purpose one.
    pub fn clear(&self) -> usize {
        let mut dropped = 0;
        while self.queue.pop().is_some() {
            dropped += 1;
        }
        dropped
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
    /// A *shared* borrow is enough because [`CommandQueue::push`] only needs `&self`; the
    /// parameter still declares a write, so two systems with `Commands` are not run side by
    /// side and their pushes keep the schedule's order.
    pub queue: Res<'w, CommandQueue>,
    /// Shared borrow of the entity-id allocator, used by [`Commands::spawn`] to reserve an
    /// id immediately (the allocator locks internally, so `&self` suffices).
    ///
    /// The allocator is global to the world; the write `Commands` declares keeps two systems from
    /// reserving at once, so ids come out in the schedule's order (see the [module docs](crate::commands)).
    pub entities: Res<'w, crate::entity::allocator::Entities>,
}

// SAFETY: it reads the `CommandQueue` and `Entities` resources through shared borrows, and declares
// a write of both (more than it touches, which is always sound): see `get_access_info`.
unsafe impl SystemParam for Commands<'static> {
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

    /// **A write of both**, though the borrows are shared: what a system pushes and which ids it
    /// reserves depend on which other system got there first, so two systems that take `Commands`
    /// must not run side by side. Declared as reads (until 2026-11-14), they shared a batch, and the
    /// commands and spawned ids of one run came out in another order on the next
    /// (`spawns_from_many_systems_get_the_same_ids_every_run`). A system with `Commands` still runs
    /// beside any system without one.
    fn get_access_info(info: &mut crate::system::AccessInfo) {
        <crate::system::ResMut<'static, CommandQueue> as SystemParam>::get_access_info(info);
        <crate::system::ResMut<'static, crate::entity::allocator::Entities> as SystemParam>::get_access_info(info);
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

    /// Creates a new entity carrying every component of `bundle` — the deferred twin of
    /// [`World::spawn_bundle`].
    ///
    /// The entity and all of its components arrive in **one** queued command, so a flush never
    /// shows it half-built, and it lands in its final archetype in one move rather than one
    /// archetype per component (`spawn().insert(a).insert(b)…` moves it once per `insert`). Until
    /// this existed a system that spawned a renderable thing wrote seven `insert` calls in a row
    /// where `setup` wrote one tuple — `kristal_avcisi`'s header records it.
    pub fn spawn_bundle<B: crate::component::Bundle + Send + Sync + 'static>(
        &mut self,
        bundle: B,
    ) -> EntityCommands<'_, 'w> {
        let entity = self.entities.reserve_entity();

        self.queue.push(move |world| {
            world.flush_spawn(entity);
            world.add_bundle(entity, bundle);
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

    /// Adds every component of `bundle` to the entity in one queued command — the deferred twin
    /// of [`World::add_bundle`]. A dead entity is a silent no-op, like [`insert`](Self::insert).
    pub fn insert_bundle<B: crate::component::Bundle + Send + Sync + 'static>(&mut self, bundle: B) -> &mut Self {
        let e = self.entity;
        self.commands.queue.push(move |world| {
            world.add_bundle(e, bundle);
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
        {
            let q = world.query::<&ComponentA>();
            for c in q.iter() {
                assert_eq!(c.0, 42);
                count += 1;
            }
        }
        assert_eq!(count, 1);
    }

    /// **Which entity a system's spawn gets does not depend on the threads.** Eight systems that
    /// touch nothing else each spawn one entity tagged with their number; across 200 runs the id
    /// each tag lands on is the same every time. While `Commands` declared only a *read* of the
    /// queue and the id allocator, the eight shared one batch and reserved ids in whatever order
    /// rayon ran them — so a game spawning from two systems got its entities in a different order
    /// on every run, and so its bodies in a different order in the physics world.
    #[test]
    // Not under Miri: what this checks is thread order — rayon's batch over eight systems, 200 times
    // — which the interpreter's one thread cannot vary, and the parallel scheduler is outside the Miri
    // job's scope on purpose (`ci.yml`). Interpreted, its 32 million spin iterations ran the job into
    // its 45-minute limit on every push from 2026-11-14 (`1ece8cc6`) until this.
    #[cfg_attr(miri, ignore = "a thread-order test: Miri runs one thread, and the spin takes hours interpreted")]
    fn spawns_from_many_systems_get_the_same_ids_every_run() {
        let one_run = || {
            let mut world = World::new();
            world.register_component_type::<ComponentA>();
            let mut schedule = Schedule::new();
            for tag in 0..8i32 {
                schedule.add_di_system::<(Commands<'static>,), _>(move |mut commands: Commands| {
                    // Enough work that rayon spreads the batch over its threads.
                    let mut x = 0u64;
                    for i in 0..20_000u64 {
                        x = std::hint::black_box(x.wrapping_mul(31).wrapping_add(i));
                    }
                    std::hint::black_box(x);
                    commands.spawn().insert(ComponentA(tag));
                });
            }
            schedule.run(&mut world, 0.1);
            let mut by_id: Vec<(u32, i32)> = world.query::<(crate::entity::Entity, &ComponentA)>().iter().map(|(e, a)| (e.id(), a.0)).collect();
            by_id.sort();
            by_id
        };
        let first = one_run();
        assert_eq!(first.len(), 8);
        for run in 1..200 {
            assert_eq!(one_run(), first, "run {run} gave the systems' entities other ids");
        }
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
        {
            let q = world.query::<(&ComponentA, &ComponentB)>();
            for (ca, cb) in q.iter() {
                assert_eq!(ca.0, 100);
                assert_eq!(cb.0, 2.5);
                count += 1;
            }
        }
        assert_eq!(count, 1);
    }

    /// A bundle spawned through `Commands` is the same entity `World::spawn_bundle` makes: both
    /// components, on the id `spawn_bundle` handed back, and nothing before the flush.
    #[test]
    fn spawn_bundle_defers_one_entity_with_every_component() {
        let mut world = World::new();
        let e = {
            let mut commands = Commands::fetch_stateless(&world, 0.0).unwrap();
            commands.spawn_bundle((ComponentA(7), ComponentB(1.5))).id()
        };
        // `entity_count` counts the reserved id already, so the components are what to look at.
        assert!(world.query::<&ComponentA>().get(e).is_none(), "nothing is observable before the flush");
        world.apply_commands();
        assert!(world.is_alive(e));
        assert_eq!(world.query::<&ComponentA>().get(e).cloned(), Some(ComponentA(7)));
        assert_eq!(world.query::<&ComponentB>().get(e).cloned(), Some(ComponentB(1.5)));
    }

    /// `insert_bundle` adds to what is there, and leaves a dead entity alone.
    #[test]
    fn insert_bundle_adds_every_component_and_skips_the_dead() {
        let mut world = World::new();
        let alive = world.spawn_bundle((ComponentA(1),));
        let dead = world.spawn();
        world.despawn(dead);
        {
            let mut commands = Commands::fetch_stateless(&world, 0.0).unwrap();
            commands.entity(alive).insert_bundle((ComponentB(2.0),));
            commands.entity(dead).insert_bundle((ComponentA(9), ComponentB(9.0)));
        }
        world.apply_commands();
        assert!(!world.is_alive(dead), "the dead entity came back");
        assert!(world.query::<&ComponentA>().get(dead).is_none());
        assert_eq!(world.query::<&ComponentA>().get(alive).cloned(), Some(ComponentA(1)));
        assert_eq!(world.query::<&ComponentB>().get(alive).cloned(), Some(ComponentB(2.0)));
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
            {
                let q = world.query::<(Entity, &ComponentA)>();
                for (id, c) in q.iter() {
                    if c.0 == 10 {
                        commands.entity(Entity::new(id.id(), 0)).despawn();
                    }
                }
            }
        });

        schedule.run(&mut world, 0.1);

        assert_eq!(world.entity_count(), 1);
        {
            let q = world.query::<&ComponentA>();
            for c in q.iter() {
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
            {
                let q = world.query::<(Entity, &ComponentA)>();
                for (id, _) in q.iter() {
                    commands.entity(Entity::new(id.id(), 0)).remove::<ComponentA>();
                }
            }
        });

        schedule.run(&mut world, 0.1);

        assert_eq!(world.entity_count(), 1);

        assert!(world.query::<&ComponentA>().is_empty(), "ComponentA still exists!");
        assert!(!world.query::<&ComponentB>().is_empty(), "ComponentB was unexpectedly removed!");
    }
}
