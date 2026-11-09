//! **Extraction:** keeping a second [`World`] a copy of the parts of another one that something —
//! a renderer — reads, so that something can work on the copy while the original moves on.
//!
//! An [`Extractor`] is told which component and resource types to copy, and each
//! [`run`](Extractor::run) brings the copy up to date with what changed since the last one:
//!
//! - **Entities keep their handles.** An entity is created in the copy with the original's id
//!   *and* generation ([`World::spawn_at`]), so an `Entity` read from either world names the same
//!   thing, and a cache keyed by `Entity` on the reading side keeps working across frames.
//! - **Only what changed is copied.** A component is cloned when the original was written since
//!   the last run (`Changed<T>`, with the run's own reference tick, not a system's), so a frame
//!   that moved ten of ten thousand entities copies ten values.
//! - **Removals and despawns follow.** A component taken off an entity is taken off its copy; an
//!   entity despawned is despawned in the copy, and an id recycled in the same frame arrives as
//!   the new generation with the old one gone. A [`World::clear_entities`] on the original clears
//!   the copy and copies everything again.
//! - **Resources are copied whole** on every run, and removed from the copy when the original has
//!   none.
//! - **Anything else** is a step of your own, `FnMut(&World, &mut World)`, run in order after the
//!   copies.
//!
//! The copy owns no entities of its own: an id belongs to the original, and one the copy spawned
//! for itself would be taken back by the original's next spawn of that id (see
//! [`World::spawn_at`]). State the reading side keeps belongs in its resources.
//!
//! A value written *in the copy* stays there until the original writes that component again —
//! a copy is refreshed, not rebuilt. Work that changes what the game sees (an animation clock, a
//! particle emitter's timer) belongs in the original, before the run.
//!
//! ```
//! use gizmo_core::prelude::*;
//! use gizmo_core::extract::Extractor;
//!
//! #[derive(Clone, PartialEq, Debug)]
//! struct Position(f32);
//! gizmo_core::impl_component!(Position);
//!
//! let mut game = World::new();
//! let mut render = World::new();
//! let mut extractor = Extractor::new();
//! extractor.component::<Position>();
//!
//! let ball = game.spawn_bundle((Position(1.0),));
//! extractor.run(&mut game, &mut render);
//! assert_eq!(render.get::<Position>(ball), Some(&Position(1.0)));
//!
//! game.despawn(ball);
//! extractor.run(&mut game, &mut render);
//! assert!(!render.is_alive(ball));
//! ```

use crate::component::Component;
use crate::entity::Entity;
use crate::query::{Changed, Query};
use crate::world::World;
use std::sync::{Arc, Mutex, Weak};

/// Entities a hook on the original world recorded, for the next run to act on. The hook holds it
/// weakly, so an extractor that is dropped leaves hooks that do nothing rather than lists that grow.
type Log = Arc<Mutex<Vec<Entity>>>;

fn push(log: &Weak<Mutex<Vec<Entity>>>, entity: Entity) {
    if let Some(log) = log.upgrade() {
        log.lock().unwrap_or_else(|e| e.into_inner()).push(entity);
    }
}

fn drain(log: &Log) -> Vec<Entity> {
    std::mem::take(&mut *log.lock().unwrap_or_else(|e| e.into_inner()))
}

/// One kind of thing an [`Extractor`] copies.
trait Step: Send {
    /// Registers what this step needs to hear about on the original world. Called once, on the
    /// first run.
    fn attach(&mut self, _main: &mut World) {}
    /// The original was cleared: forget what was copied, so the next `run` copies everything.
    fn reset(&mut self) {}
    /// Brings `render` up to date with `main`.
    fn run(&mut self, main: &World, render: &mut World);
}

struct ComponentStep<T: Component> {
    /// The original's tick at the last run: a value written after it is newer. 0 copies everything.
    last: u32,
    removed: Log,
    _marker: std::marker::PhantomData<fn() -> T>,
}

impl<T: Component> Step for ComponentStep<T> {
    fn attach(&mut self, main: &mut World) {
        let log = Arc::downgrade(&self.removed);
        main.register_on_remove::<T>(move |_, entity| push(&log, entity));
    }

    fn reset(&mut self) {
        self.last = 0;
        drain(&self.removed);
    }

    fn run(&mut self, main: &World, render: &mut World) {
        for entity in drain(&self.removed) {
            // A despawn is the extractor's own list; a component put back on the same frame is
            // newer than `last` and is copied below.
            if !main.is_alive(entity) || main.get::<T>(entity).is_some() {
                continue;
            }
            if render.is_alive(entity) {
                render.remove_component::<T>(entity);
            }
        }
        let changed: Vec<(Entity, T)> = Query::<(Entity, &T), Changed<T>>::new_since(main, self.last)
            .iter()
            .map(|(entity, value)| (entity, value.clone()))
            .collect();
        for (entity, value) in changed {
            render.spawn_at(entity);
            render.add_component(entity, value);
        }
        self.last = main.tick;
    }
}

struct ResourceStep<T>(std::marker::PhantomData<fn() -> T>);

impl<T: Clone + Send + Sync + 'static> Step for ResourceStep<T> {
    fn run(&mut self, main: &World, render: &mut World) {
        let value = main.get_resource::<T>().map(|r| (*r).clone());
        match value {
            Some(value) => render.insert_resource(value),
            None => {
                render.remove_resource::<T>();
            }
        }
    }
}

struct CustomStep<F>(F);

impl<F: FnMut(&World, &mut World) + Send> Step for CustomStep<F> {
    fn run(&mut self, main: &World, render: &mut World) {
        (self.0)(main, render)
    }
}

/// Keeps one world a copy of the chosen parts of another — see the [module](self) for what it
/// copies, when, and what it does not.
///
/// An extractor serves **one** original world: the first [`run`](Self::run) registers a despawn
/// hook and a removal hook per component type on it, and later runs read what those recorded.
#[derive(Default)]
pub struct Extractor {
    steps: Vec<Box<dyn Step>>,
    /// Steps that have registered their hooks; the rest attach on the next run.
    attached: usize,
    despawned: Log,
    hooked: bool,
    /// The original's [`World::reset_epoch`] at the last run; a different one means it was cleared.
    epoch: Option<u64>,
}

impl std::fmt::Debug for Extractor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Extractor").field("steps", &self.steps.len()).field("epoch", &self.epoch).finish()
    }
}

impl Extractor {
    /// An extractor that copies nothing yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Copies `T` on every entity that has it, as it changes, and takes it off the copy when it is
    /// removed.
    pub fn component<T: Component>(&mut self) -> &mut Self {
        self.steps.push(Box::new(ComponentStep::<T> {
            last: 0,
            removed: Log::default(),
            _marker: std::marker::PhantomData,
        }));
        self
    }

    /// Copies the resource `T` whole on every run, and removes it from the copy while the original
    /// has none.
    pub fn resource<T: Clone + Send + Sync + 'static>(&mut self) -> &mut Self {
        self.steps.push(Box::new(ResourceStep::<T>(std::marker::PhantomData)));
        self
    }

    /// A step of your own, run on every run after the steps added before it.
    pub fn step(&mut self, step: impl FnMut(&World, &mut World) + Send + 'static) -> &mut Self {
        self.steps.push(Box::new(CustomStep(step)));
        self
    }

    /// Brings `render` up to date with `main`: despawns first, then each step in the order it was
    /// added.
    ///
    /// Moves `main`'s change tick on by one afterwards, so that a write made after this run —
    /// even with no schedule run in between to advance the tick — is newer than what it copied.
    pub fn run(&mut self, main: &mut World, render: &mut World) {
        if !self.hooked {
            let log = Arc::downgrade(&self.despawned);
            main.register_despawn_hook(move |_, entity| push(&log, entity));
            self.hooked = true;
        }
        for step in &mut self.steps[self.attached..] {
            step.attach(main);
        }
        self.attached = self.steps.len();

        let epoch = main.reset_epoch();
        if self.epoch.is_some_and(|seen| seen != epoch) {
            render.clear_entities();
            drain(&self.despawned);
            for step in &mut self.steps {
                step.reset();
            }
        }
        self.epoch = Some(epoch);

        for entity in drain(&self.despawned) {
            if render.is_alive(entity) {
                render.despawn(entity);
            }
        }
        for step in &mut self.steps {
            step.run(main, render);
        }
        main.advance_change_tick();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq)]
    struct Pos(f32);
    crate::impl_component!(Pos);

    #[derive(Clone, Debug, PartialEq)]
    struct Tag;
    crate::impl_component!(Tag);

    #[derive(Clone, Debug, PartialEq)]
    struct Secret(u8);
    crate::impl_component!(Secret);

    #[derive(Clone, Debug, PartialEq)]
    struct Fog(f32);

    fn pair() -> (World, World, Extractor) {
        let mut extractor = Extractor::new();
        extractor.component::<Pos>().component::<Tag>().resource::<Fog>();
        (World::new(), World::new(), extractor)
    }

    fn pos(world: &World, entity: Entity) -> Option<f32> {
        world.get::<Pos>(entity).map(|p| p.0)
    }

    #[test]
    fn entities_arrive_with_the_same_handle_and_only_the_chosen_components() {
        let (mut game, mut render, mut ex) = pair();
        let _gap = game.spawn();
        let a = game.spawn_bundle((Pos(1.0), Secret(7)));
        let b = game.spawn_bundle((Tag,));
        ex.run(&mut game, &mut render);

        assert_eq!(pos(&render, a), Some(1.0));
        assert!(render.get::<Tag>(b).is_some());
        assert!(render.get::<Secret>(a).is_none(), "a type nobody asked for was copied");
        // The id the copy has not been told about stays free for the original's next spawn.
        let c = game.spawn_bundle((Pos(3.0),));
        ex.run(&mut game, &mut render);
        assert_eq!(pos(&render, c), Some(3.0));
        assert_eq!(render.query::<&Pos>().iter().count(), 2);
    }

    #[test]
    fn only_what_changed_is_copied() {
        let (mut game, mut render, mut ex) = pair();
        let a = game.spawn_bundle((Pos(1.0),));
        let b = game.spawn_bundle((Pos(2.0),));
        ex.run(&mut game, &mut render);

        // Marks on the copy that a full copy would wipe out.
        render.add_component(a, Pos(-1.0));
        render.add_component(b, Pos(-2.0));
        if let Some(mut p) = game.query_mut::<&mut Pos>().get_mut(b) {
            p.0 = 20.0;
        }
        ex.run(&mut game, &mut render);
        assert_eq!(pos(&render, a), Some(-1.0), "an unchanged component was copied again");
        assert_eq!(pos(&render, b), Some(20.0));
    }

    #[test]
    fn a_write_after_a_run_on_the_same_tick_is_seen_by_the_next_run() {
        let (mut game, mut render, mut ex) = pair();
        let a = game.spawn_bundle((Pos(1.0),));
        ex.run(&mut game, &mut render);
        // No schedule runs in between, so nothing else moves the tick.
        if let Some(mut p) = game.query_mut::<&mut Pos>().get_mut(a) {
            p.0 = 5.0;
        }
        ex.run(&mut game, &mut render);
        assert_eq!(pos(&render, a), Some(5.0));
    }

    #[test]
    fn removals_follow_and_a_component_put_back_survives() {
        let (mut game, mut render, mut ex) = pair();
        let a = game.spawn_bundle((Pos(1.0), Tag));
        let b = game.spawn_bundle((Pos(2.0),));
        ex.run(&mut game, &mut render);

        game.remove_component::<Tag>(a);
        game.remove_component::<Pos>(b);
        game.add_component(b, Pos(9.0));
        ex.run(&mut game, &mut render);
        assert!(render.get::<Tag>(a).is_none());
        assert_eq!(pos(&render, a), Some(1.0));
        assert_eq!(pos(&render, b), Some(9.0));
    }

    #[test]
    fn a_despawn_follows_and_a_recycled_id_arrives_as_its_new_generation() {
        let (mut game, mut render, mut ex) = pair();
        let a = game.spawn_bundle((Pos(1.0),));
        ex.run(&mut game, &mut render);

        game.despawn(a);
        let again = game.spawn_bundle((Pos(4.0),));
        assert_eq!(again.id(), a.id(), "the test needs the id recycled");
        ex.run(&mut game, &mut render);
        assert!(!render.is_alive(a));
        assert!(render.is_alive(again));
        assert_eq!(pos(&render, again), Some(4.0));
        assert_eq!(render.query::<&Pos>().iter().count(), 1);
    }

    #[test]
    fn a_cleared_original_is_copied_again_from_nothing() {
        let (mut game, mut render, mut ex) = pair();
        let a = game.spawn_bundle((Pos(1.0),));
        ex.run(&mut game, &mut render);

        game.clear_entities();
        let b = game.spawn_bundle((Pos(2.0),));
        ex.run(&mut game, &mut render);
        // `b` is `a`'s exact handle after a clear (id 0, generation 0); there must be one of it.
        assert_eq!(a, b);
        assert_eq!(render.query::<&Pos>().iter().count(), 1);
        assert_eq!(pos(&render, b), Some(2.0));
    }

    #[test]
    fn resources_are_copied_and_removed() {
        let (mut game, mut render, mut ex) = pair();
        game.insert_resource(Fog(0.5));
        ex.run(&mut game, &mut render);
        assert_eq!(render.get_resource::<Fog>().map(|f| f.0), Some(0.5));
        game.remove_resource::<Fog>();
        ex.run(&mut game, &mut render);
        assert!(render.get_resource::<Fog>().is_none());
    }

    #[test]
    fn a_step_of_your_own_runs_after_the_copies() {
        let (mut game, mut render, mut ex) = pair();
        ex.step(|main, render| {
            let n = main.query::<&Pos>().iter().count();
            let copied = render.query::<&Pos>().iter().count();
            render.insert_resource((n, copied));
        });
        game.spawn_bundle((Pos(1.0),));
        game.spawn_bundle((Pos(2.0),));
        ex.run(&mut game, &mut render);
        assert_eq!(render.get_resource::<(usize, usize)>().map(|r| *r), Some((2, 2)));
    }

    #[test]
    fn spawn_at_takes_an_id_from_whatever_holds_it() {
        let mut world = World::new();
        let own = world.spawn_bundle((Pos(1.0),));
        let wanted = Entity::new(own.id(), 3);
        world.spawn_at(wanted);
        assert!(world.is_alive(wanted));
        assert!(!world.is_alive(own));
        world.add_component(wanted, Pos(2.0));
        assert_eq!(pos(&world, wanted), Some(2.0));
        // Past the high-water mark: the ids skipped are free for the world's own spawns.
        let far = Entity::new(10, 0);
        world.spawn_at(far);
        let next = world.spawn();
        assert!(next.id() < 10, "a skipped id was not put on the free list");
        assert!(world.is_alive(far));
    }
}
