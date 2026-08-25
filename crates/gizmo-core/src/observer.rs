//! Push-side notification — callbacks that run at the instant something happens, as opposed
//! to the [`event`](crate::event) queues, which systems poll a frame later.
//!
//! Three mechanisms are spelled with the types in this module, and what separates them is what
//! each is **keyed to**: a component type, a target entity, or nothing at all.
//!
//! * **Component lifecycle.** `World::add_observer::<E, T, _>` appends a hook for `T` on the
//!   phase `E` names — [`Insert`], [`Replace`] or [`Remove`] — and the marker comes from the
//!   closure's own `On<E, T>`, so the ordinary call needs no turbofish. The three partition a
//!   component's life: a write fires `Insert` **xor** `Replace`, never both and never neither,
//!   and a detachment fires `Remove`. Being hooks, all three inherit the hook system's gaps —
//!   the bulk paths that write archetype columns directly notify nothing (see
//!   [`ComponentHooks`](crate::world::ComponentHooks)), so this is push-side *notification*,
//!   not an audit trail.
//! * **Entity-targeted events.** `World::observe` attaches a listener for one
//!   [`EntityEvent`] type to one entity; `World::trigger` dispatches a value to that
//!   entity's listeners and, if the event opts in, on up the `Parent` chain — until a listener
//!   calls [`World::stop_propagation`](crate::world::World::stop_propagation).
//! * **Untargeted events.** `World::observe_global` and `World::trigger_global`, keyed only by
//!   the event's own type: "the level loaded", "the round ended". No hierarchy, so no `Clone`
//!   bound — the listener is handed the event by reference. Added 2026-08-24; before it, such a
//!   thing had to be an [`Events`](crate::event::Events) queue read a frame later.
//!
//! All three run synchronously on the calling thread, inside the `&mut World` call that caused
//! them. The two **event** paths hand their listener a `&mut World` directly; a **lifecycle**
//! observer does not — its `On` is built by a hook that has one, so the world is one layer down
//! at [`register_on_add`](crate::world::World::register_on_add) and its siblings, and a
//! lifecycle callback that needs the world registers there instead. Until 2026-08-24 none of
//! them reached a world, which is why a chain reaction had to be written as a captured queue
//! drained by a later system, and therefore advanced one link per frame.
//!
//! **Re-entrancy is bounded by detachment, not by prohibition.** For the whole of a dispatch the
//! list it draws from is out of the world — a component type's hook lists during a hook, one
//! entity's listeners during a trigger, one event type's listeners during a global publish. So a
//! callback that provokes the same notification at the same target terminates instead of
//! recursing, while any *other* target's callbacks are live and run nested.
//!
//! *For the whole of a dispatch* is the load-bearing half. `trigger` used to return each listener
//! to the map as soon as it finished, so by the second listener the first was live again and a
//! nested trigger at the same entity re-ran it. Corrected 2026-08-24, with the test that a single
//! listener could never have caught.
//!
//! Listeners registered against the same entity and event type run in registration order.
//! Neither mechanism has an unregister, and registering the same closure twice makes it fire
//! twice. Registrations then accumulate for the life of the world — with one exception, and it
//! is an exception rather than a hatch: [`World::clear_entities`](crate::world::World::clear_entities)
//! destroys every `observe` listener, because it destroys every entity they were keyed to and
//! hands the same `Entity` values back to strangers. Type-keyed registrations — the lifecycle
//! hooks and `observe_global` — survive it, since they were never about a particular entity.

use crate::entity::Entity;
use std::marker::PhantomData;

/// Lifecycle marker meaning "component `T` became present on this entity".
///
/// Only ever used as the `E` of [`On`]: an observer registered with `World::add_observer`
/// is called with `On<Insert, T>`. Despite the name it does not fire on every write —
/// writing over a value the entity already has counts as a *set*, not an insert, and
/// notifies nothing here.
///
/// Zero-sized: it carries no payload, so it tells the observer *that* the component appeared
/// and — through [`On::entity`] — on which entity, never what the value is.
#[derive(Clone, Copy)]
pub struct Insert;
/// Lifecycle marker meaning "component `T` was detached from this entity".
///
/// Delivered to `World::add_observer(|e: On<Remove, T>| …)`, from the same
/// [`RemoveHook`](crate::world::RemoveHook) list a raw hook registration fills — so it
/// inherits that list's rules exactly, and they are worth reading: whether the component is
/// still readable when the observer runs depends on the removal path and on `T`'s storage
/// type. `World::remove_bundle` used to detach Table components without firing anything;
/// giving `Remove` a dispatch path is what turned that from an internal quirk into a broken
/// promise, so it was fixed in the same change.
///
/// A whole-entity `World::despawn` fires it once per component type the entity still held,
/// in an order across types that comes from a `HashMap` and is arbitrary.
#[derive(Clone, Copy)]
pub struct Remove;
/// Lifecycle marker meaning "a write replaced a value the entity already had".
///
/// The strict complement of [`Insert`] within a write: a write that notifies at all fires the
/// component's set hooks and exactly one of `On<Insert, T>` or `On<Replace, T>` alongside them —
/// never both, never neither. *That notifies at all* is the caveat: the bulk paths listed on
/// [`ComponentHooks`](crate::world::ComponentHooks) write columns directly and fire nothing, so
/// the partition holds over the writes that are announced, not over every write that happens. That is the distinction an observer cannot draw for itself, because it is
/// handed the entity and nothing else; the dispatcher knows which branch it took, so it says.
///
/// Zero-sized like the others: it witnesses *that* a value was replaced, never what it was.
/// Reading the component back out gives the **new** value — the old one is already dropped,
/// because the write is an assignment precisely so a `T: Drop` does not leak.
#[derive(Clone, Copy)]
pub struct Replace;

mod sealed {
    /// Keeps [`Lifecycle`](super::Lifecycle) closed. The trait selects a hook list on
    /// `ComponentHooks`, so a downstream impl could only either duplicate one of the three or
    /// name a list that does not exist.
    pub trait SealedLifecycle {}
}

/// Which of a component's hook lists an observer is registered on — the type-level half of
/// `World::add_observer`.
///
/// Implemented for [`Insert`], [`Remove`] and [`Replace`] and sealed against anything else.
/// It exists so the marker in the *closure's own signature* picks the list:
///
/// ```
/// # use gizmo_core::world::World;
/// # use gizmo_core::observer::{On, Insert, Remove, Replace};
/// # #[derive(Clone, Copy)] pub struct Hp(u32);
/// # gizmo_core::impl_component!(Hp);
/// # let mut world = World::new();
/// world.add_observer(|e: On<Insert, Hp>| { let _ = e.entity; });
/// world.add_observer(|e: On<Replace, Hp>| { let _ = e.entity; });
/// world.add_observer(|e: On<Remove, Hp>| { let _ = e.entity; });
/// ```
///
/// There is no fourth marker for "despawned": a despawn already reaches `On<Remove, T>` once
/// per component the entity held, and the whole-entity notification is
/// [`World::register_despawn_hook`](crate::world::World::register_despawn_hook), which is not
/// per-component and so cannot be an `On<_, T>`.
pub trait Lifecycle: sealed::SealedLifecycle + Copy + Send + Sync + 'static {
    /// The zero-sized witness handed to the observer as `On::event`.
    fn witness() -> Self;

    /// The list on `ComponentHooks` this marker registers against.
    ///
    /// All four hook aliases are the same boxed closure type, which is what lets one function
    /// return any of the lists. Hidden because naming that type is an implementation detail —
    /// the trait is sealed, so nobody outside can call it usefully anyway.
    #[doc(hidden)]
    fn list(hooks: &mut crate::world::ComponentHooks) -> &mut Vec<crate::world::AddHook>;
}

impl sealed::SealedLifecycle for Insert {}
impl Lifecycle for Insert {
    fn witness() -> Self {
        Insert
    }
    fn list(hooks: &mut crate::world::ComponentHooks) -> &mut Vec<crate::world::AddHook> {
        &mut hooks.on_add
    }
}

impl sealed::SealedLifecycle for Remove {}
impl Lifecycle for Remove {
    fn witness() -> Self {
        Remove
    }
    fn list(hooks: &mut crate::world::ComponentHooks) -> &mut Vec<crate::world::AddHook> {
        &mut hooks.on_remove
    }
}

impl sealed::SealedLifecycle for Replace {}
impl Lifecycle for Replace {
    fn witness() -> Self {
        Replace
    }
    fn list(hooks: &mut crate::world::ComponentHooks) -> &mut Vec<crate::world::AddHook> {
        &mut hooks.on_replace
    }
}

/// One registered [`World::observe`](crate::world::World::observe) listener.
///
/// Named because three places have to spell it — the registration, the map it lives in, and
/// the dispatch that takes it out — and a signature typo used to mean a `downcast_mut` that
/// silently returned `None`, i.e. a listener that was never called and no error anywhere.
pub type EntityListener<E> =
    Box<dyn FnMut(&mut crate::world::World, On<E>) + Send + Sync + 'static>;

/// One registered [`World::observe_global`](crate::world::World::observe_global) listener.
///
/// It takes the event by **reference**, which is the whole difference from
/// [`EntityListener`]: a global event is delivered to a flat list rather than walked up a
/// hierarchy, so there is no second recipient to hand a second copy to and therefore no
/// `Clone` bound on the event type. An event holding a `Vec` costs nothing to publish.
pub type GlobalListener<E> =
    Box<dyn FnMut(&mut crate::world::World, &E) + Send + Sync + 'static>;

/// A user-defined event delivered to listeners attached to individual entities
/// (`World::observe`) and dispatched by `World::trigger`, as opposed to an
/// [`Events`](crate::event::Events) queue, which is read by systems.
///
/// `Clone` is required because one dispatch may hand the same value to several listeners and
/// to several entities along a propagation chain; `Send + Sync + 'static` because the
/// listener table lives in the world and is keyed by the event's `TypeId`.
pub trait EntityEvent: Send + Sync + 'static + Clone {
    /// The entity dispatch starts at — the bottom of the chain when propagation is enabled.
    ///
    /// Read once per dispatch and expected to be stable for a given event value. The entity
    /// need not be alive or carry any listener; a target with no listener for this event
    /// type is simply skipped. That ends the dispatch when
    /// [`can_propagate`](Self::can_propagate) is `false`, but when it is `true` the walk
    /// continues to the ancestors regardless of whether the target itself was observed.
    fn target(&self) -> Entity;

    /// Whether dispatch continues to the target's ancestors once the target's own listeners
    /// have run (bubbling).
    ///
    /// `false` by default: only [`target`](Self::target) is notified. When `true`, dispatch
    /// walks up the `Parent` chain, running each ancestor's listeners for this event type, and
    /// stops at the first entity that
    ///
    /// * has no `Parent` component,
    /// * whose recorded parent id is no longer alive,
    /// * whose listener called [`World::stop_propagation`](crate::world::World::stop_propagation), or
    /// * **this walk has already visited** — the cycle guard, added 2026-08-24. `Parent` is a
    ///   bare id that can be written directly, so a chain can loop; without this the walk did
    ///   not terminate.
    ///
    /// Listeners along the chain all receive equal clones of the same event.
    ///
    /// This is read once per step of the walk, so an implementation that returns different
    /// answers at different times gets an unspecified walk rather than an interesting one —
    /// return a constant, or a value fixed when the event is constructed.
    fn can_propagate(&self) -> bool { false }
}

/// What a listener receives: the event value plus the entity this particular delivery is
/// about.
///
/// Two unrelated dispatch paths share the type:
/// * component lifecycle — `E` is a marker such as [`Insert`] and `T` is the component type
///   the observer was registered for;
/// * [`EntityEvent`] listeners — `E` is the event value and `T` stays at its default `()`.
///
/// It is handed to the listener *by value* and the listener returns `()`. There is still no
/// return channel through the *type* — a listener cannot report that it handled the delivery —
/// but there is one through the world: an entity-event listener receives `&mut World`, and
/// [`World::stop_propagation`](crate::world::World::stop_propagation) is read by the dispatch
/// loop once the listener returns. The signature never had to change.
///
/// `Clone` requires only `E: Clone`, never `T: Clone`, since `T` is a phantom tag.
pub struct On<E, T = ()> {
    /// The event value. For the lifecycle markers this is a zero-sized witness carrying no
    /// data — neither the component that was inserted nor the value it replaced.
    pub event: E,
    /// The entity this delivery concerns.
    ///
    /// For a lifecycle observer, the entity that gained the component. For a propagating
    /// [`EntityEvent`], the entity whose listener is running *right now*: it equals
    /// `event.target()` on the first hop only, and names each ancestor in turn afterwards
    /// while `target()` keeps naming the origin.
    pub entity: Entity,
    /// Binds the value to the component type `T` a lifecycle observer was registered for.
    ///
    /// Zero-sized and carries nothing readable; it is public only so the struct can be
    /// built with a literal `PhantomData`.
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

    /// Sparse storage, so the partition can be asserted on the branch that computes
    /// `existed` for itself rather than reading it off the archetype migration.
    #[derive(Clone)]
    #[allow(dead_code)]
    struct Sparse(u32);

    impl Component for Health {}
    impl Component for Poison {}
    impl Component for Sparse {
        fn storage_type() -> crate::component::StorageType {
            crate::component::StorageType::SparseSet
        }
    }

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

    /// A ledger of which phase fired, per entity, so a test can assert the *partition*
    /// rather than one counter at a time.
    #[derive(Default, Debug, PartialEq, Eq)]
    struct Phases {
        inserted: Vec<u32>,
        replaced: Vec<u32>,
        removed: Vec<u32>,
    }

    /// Registers all three observers for `T` against one shared ledger.
    fn watch<T: Component>(world: &mut World) -> Arc<Mutex<Phases>> {
        let log = Arc::new(Mutex::new(Phases::default()));
        let (a, b, c) = (log.clone(), log.clone(), log.clone());
        world.add_observer(move |e: On<Insert, T>| a.lock().unwrap().inserted.push(e.entity.id()));
        world.add_observer(move |e: On<Replace, T>| b.lock().unwrap().replaced.push(e.entity.id()));
        world.add_observer(move |e: On<Remove, T>| c.lock().unwrap().removed.push(e.entity.id()));
        log
    }

    /// **A write fires `Insert` xor `Replace`** — the claim the whole `on_replace` list exists
    /// to make true, asserted on Table storage.
    ///
    /// The negative control is the second `add_component`: before `on_replace` existed the
    /// overwrite fired `on_set` and nothing else, so `replaced` stayed empty while `inserted`
    /// stayed at one. Both halves are checked, because a dispatcher that fired `Insert` again
    /// on the overwrite would also make `replaced` empty — and that is the opposite bug.
    #[test]
    fn a_write_fires_insert_xor_replace() {
        let mut world = World::new();
        let log = watch::<Health>(&mut world);

        let e = world.spawn_bundle((Health(100.0),));
        assert_eq!(log.lock().unwrap().inserted, vec![e.id()], "the first write is an insert");
        assert!(log.lock().unwrap().replaced.is_empty(), "…and not a replace");

        world.add_component(e, Health(50.0));
        assert_eq!(log.lock().unwrap().replaced, vec![e.id()], "the second write is a replace");
        assert_eq!(log.lock().unwrap().inserted, vec![e.id()], "…and not a second insert");

        world.add_component(e, Health(25.0));
        assert_eq!(log.lock().unwrap().replaced, vec![e.id(); 2], "every later write replaces");
        assert!(log.lock().unwrap().removed.is_empty(), "nothing was detached");
    }

    /// The same partition on **SparseSet** storage, which computes `existed` itself instead of
    /// reading it off which branch it is in.
    ///
    /// Worth its own test: the two storages reach the decision by different routes, and this
    /// crate has already shipped one storage-dependent lifecycle bug — the sparse path used to
    /// fire `on_add` unconditionally, so re-adding a sparse component double-fired `Insert`.
    #[test]
    fn the_partition_holds_on_sparse_storage_too() {
        let mut world = World::new();
        let log = watch::<Sparse>(&mut world);

        let e = world.spawn_bundle((Sparse(1),));
        world.add_component(e, Sparse(2));
        world.add_component(e, Sparse(3));

        let seen = log.lock().unwrap();
        assert_eq!(seen.inserted, vec![e.id()], "one insert, not three");
        assert_eq!(seen.replaced, vec![e.id(); 2], "two replaces");
    }

    /// `On<Remove, T>` reaches a closure — the marker that had no dispatch path at all.
    ///
    /// Both routes are asserted: an explicit `remove_component`, and a whole-entity `despawn`,
    /// which fires it once per component type the entity still held.
    ///
    /// Each half checks the ledger is **empty before** the detachment and populated after, and
    /// that is not ceremony. Asserting only the end state passes for the wrong reason if
    /// `Remove` is mis-wired to the `on_add` list: the spawn fills `removed` at insertion time
    /// and the final `assert_eq!` cannot tell that apart from a working remove path. Found by
    /// running exactly that mutation.
    #[test]
    fn remove_reaches_an_observer_by_both_routes() {
        let mut world = World::new();
        let log = watch::<Health>(&mut world);

        let a = world.spawn_bundle((Health(1.0),));
        assert!(log.lock().unwrap().removed.is_empty(), "spawning fired the remove observer");
        world.remove_component::<Health>(a);
        assert_eq!(log.lock().unwrap().removed, vec![a.id()], "remove_component");

        let b = world.spawn_bundle((Health(2.0),));
        assert_eq!(log.lock().unwrap().removed, vec![a.id()], "spawning fired it again");
        world.despawn(b);
        assert_eq!(log.lock().unwrap().removed, vec![a.id(), b.id()], "despawn");

        // And a removal is not a write: neither of the other two phases moved.
        let seen = log.lock().unwrap();
        assert_eq!(seen.inserted, vec![a.id(), b.id()]);
        assert!(seen.replaced.is_empty());
    }

    /// `remove_bundle` notifies for its **Table** components too — it did not until
    /// `On<Remove, T>` existed.
    ///
    /// The bundle carries one of each storage class on purpose. The sparse half was already
    /// notified by an explicit loop; the Table half went out through `move_entity_to` in
    /// silence, so the same component removed by `remove_component` and by `remove_bundle`
    /// answered differently. A silent notification gap is invisible to every test that only
    /// checks the world's *contents* afterwards, which is why this asserts the observer.
    #[test]
    fn remove_bundle_notifies_for_both_storage_classes() {
        let mut world = World::new();
        let table = watch::<Health>(&mut world);
        let sparse = watch::<Sparse>(&mut world);

        let e = world.spawn_bundle((Health(1.0), Sparse(2)));
        assert!(table.lock().unwrap().removed.is_empty());
        world.remove_bundle::<(Health, Sparse)>(e);

        assert_eq!(table.lock().unwrap().removed, vec![e.id()], "the Table component");
        assert_eq!(sparse.lock().unwrap().removed, vec![e.id()], "the sparse component");
        // And the data really is gone — the notification is not standing in for the removal.
        assert!(world.query_entity::<&Health>(e.id()).is_none());
    }

    /// A bundle naming a component the entity does not have notifies nothing for it.
    ///
    /// The negative control for the collection above: firing per *bundle member* rather than
    /// per member the entity actually held would make `remove_bundle` announce removals that
    /// never happened, and the test before this one would not notice.
    #[test]
    fn remove_bundle_does_not_announce_what_was_never_there() {
        let mut world = World::new();
        let log = watch::<Health>(&mut world);

        let e = world.spawn_bundle((Poison,));
        world.remove_bundle::<(Health, Poison)>(e);
        assert!(log.lock().unwrap().removed.is_empty(), "announced a removal that did not happen");
    }

    /// `insert_batch` obeys the same partition — the third dispatch site, and the one whose
    /// overwrite group is a separate code path from `add_component`'s.
    #[test]
    fn insert_batch_splits_its_group_the_same_way() {
        let mut world = World::new();
        let log = watch::<Health>(&mut world);

        let a = world.spawn_bundle((Health(1.0),));
        let b = world.spawn();
        // `a` already has it and `b` does not: one batch, both cases.
        world.insert_batch(&[a, b], Health(9.0));

        let seen = log.lock().unwrap();
        assert_eq!(seen.replaced, vec![a.id()], "the entity that had one was replaced");
        assert_eq!(seen.inserted, vec![a.id(), b.id()], "the entity that did not was inserted");
    }

    /// Each marker registers on its **own** list, so an observer never hears another phase.
    ///
    /// The negative control for `Lifecycle::list`: wiring two markers to the same list would
    /// leave every other test here passing — they assert what fired, and a duplicate list
    /// still fires. This asserts what did *not*.
    #[test]
    fn the_three_markers_do_not_share_a_list() {
        let mut world = World::new();
        let only_removes = Arc::new(Mutex::new(0));
        let counter = only_removes.clone();
        world.add_observer(move |_: On<Remove, Poison>| *counter.lock().unwrap() += 1);

        let e = world.spawn_bundle((Poison,));
        world.add_component(e, Poison);
        assert_eq!(*only_removes.lock().unwrap(), 0, "an insert or a replace woke the remove observer");

        world.remove_component::<Poison>(e);
        assert_eq!(*only_removes.lock().unwrap(), 1);
    }

    /// Two observers on the same phase both fire, in registration order — the property
    /// `add_observer` inherits from the hook list it pushes onto.
    #[test]
    fn observers_on_one_phase_accumulate() {
        let mut world = World::new();
        let order = Arc::new(Mutex::new(Vec::<u8>::new()));
        let (first, second) = (order.clone(), order.clone());
        world.add_observer(move |_: On<Replace, Health>| first.lock().unwrap().push(1));
        world.add_observer(move |_: On<Replace, Health>| second.lock().unwrap().push(2));

        let e = world.spawn_bundle((Health(1.0),));
        world.add_component(e, Health(2.0));
        assert_eq!(*order.lock().unwrap(), vec![1, 2]);
    }

    /// `register_on_replace` is the hatch, and it sees the same events as the observer.
    #[test]
    fn the_raw_hook_is_the_hatch_for_replace() {
        let mut world = World::new();
        let hits = Arc::new(Mutex::new(0));
        let counter = hits.clone();
        world.register_on_replace::<Health>(Box::new(move |_w, _e| *counter.lock().unwrap() += 1));

        let e = world.spawn_bundle((Health(1.0),));
        assert_eq!(*hits.lock().unwrap(), 0, "the insert is not a replace");
        world.add_component(e, Health(2.0));
        assert_eq!(*hits.lock().unwrap(), 1);
    }

    /// An event whose target is chosen at construction; `can_propagate` is off unless asked.
    #[derive(Clone)]
    struct Ping {
        target: Entity,
        bubble: bool,
    }
    impl EntityEvent for Ping {
        fn target(&self) -> Entity {
            self.target
        }
        fn can_propagate(&self) -> bool {
            self.bubble
        }
    }

    /// **The entity-event listener gets a usable `&mut World`** — the capability the whole
    /// change is about, asserted by doing something only a world can do.
    ///
    /// Before 2026-08-24 the listener was handed the `On` and nothing else, so acting on a
    /// notification meant writing to captured state and waiting for a later system to read it.
    #[test]
    fn an_entity_listener_can_act_on_the_world() {
        let mut world = World::new();
        let e = world.spawn();
        world.observe::<Ping, _>(e, |world: &mut World, _on| {
            world.spawn();
        });

        let before = world.entity_count();
        world.trigger(Ping { target: e, bubble: false });
        assert_eq!(world.entity_count(), before + 1, "the listener could not reach the world");
    }

    /// A listener that triggers the **same** event back at its **own** entity terminates.
    ///
    /// Not a special case in the dispatcher: the entity's listeners are **owned by the dispatch**
    /// for the length of the call — moved out of the map, not borrowed from it — so the nested
    /// trigger finds nothing to run at that entity. Termination is therefore structural rather
    /// than checked, and this test records the contract rather than guarding a branch: a
    /// mutation swapping the `remove` for a `mem::take` left it green, because either way the
    /// list is empty while the listener runs. What it would catch is a redesign — one that
    /// notified from a live list, or deferred nested triggers to a queue.
    #[test]
    fn a_listener_retriggering_its_own_entity_terminates() {
        let mut world = World::new();
        let e = world.spawn();
        let hits = Arc::new(Mutex::new(0));
        let counter = hits.clone();
        world.observe::<Ping, _>(e, move |world: &mut World, on| {
            *counter.lock().unwrap() += 1;
            world.trigger(Ping { target: on.entity, bubble: false });
        });

        world.trigger(Ping { target: e, bubble: false });
        assert_eq!(*hits.lock().unwrap(), 1, "the nested trigger re-entered the same listener");
    }

    /// **The whole entity's list is detached, not just the listener currently running.**
    ///
    /// Found by review 2026-08-24, and it was a real defect rather than a doc quibble: `trigger`
    /// used to put each listener back *inside* its loop, so by the time the second listener ran,
    /// the first was live in the map again. A nested trigger at the same entity — the case the
    /// termination guarantee is about — re-ran every listener that had already finished. One
    /// listener could not see it, which is exactly why
    /// `a_listener_retriggering_its_own_entity_terminates` stayed green.
    #[test]
    fn a_nested_trigger_does_not_rerun_listeners_that_already_finished() {
        let mut world = World::new();
        let e = world.spawn();
        let runs = Arc::new(Mutex::new(Vec::<u8>::new()));
        let (first, second) = (runs.clone(), runs.clone());

        world.observe::<Ping, _>(e, move |_w: &mut World, _on| {
            first.lock().unwrap().push(1);
        });
        world.observe::<Ping, _>(e, move |world: &mut World, on| {
            second.lock().unwrap().push(2);
            // The nested dispatch must find nothing: listener 1 has run, but the entity's list
            // belongs to the outer dispatch until it finishes.
            world.trigger(Ping { target: on.entity, bubble: false });
        });

        world.trigger(Ping { target: e, bubble: false });
        assert_eq!(*runs.lock().unwrap(), vec![1, 2], "a finished listener was re-run by a nested trigger");
    }

    /// A listener registered from **inside** a dispatch lands after the ones already there, and
    /// runs from the next trigger — the same merge-back order the global path uses.
    #[test]
    fn an_entity_listener_registered_during_a_dispatch_keeps_its_place() {
        let mut world = World::new();
        let e = world.spawn();
        let runs = Arc::new(Mutex::new(Vec::<u8>::new()));
        let (outer, inner) = (runs.clone(), runs.clone());

        world.observe::<Ping, _>(e, move |world: &mut World, _on| {
            outer.lock().unwrap().push(1);
            let inner = inner.clone();
            world.observe::<Ping, _>(e, move |_w: &mut World, _on| {
                inner.lock().unwrap().push(2);
            });
        });

        world.trigger(Ping { target: e, bubble: false });
        assert_eq!(*runs.lock().unwrap(), vec![1], "the new listener ran during its own registration");
        world.trigger(Ping { target: e, bubble: false });
        assert_eq!(*runs.lock().unwrap(), vec![1, 1, 2], "registration order was not preserved");
    }

    /// …and the listener is **put back**, so the next trigger reaches it again. The detachment
    /// is for the duration of one dispatch, not a de-registration.
    #[test]
    fn a_listener_survives_the_dispatch_that_detached_it() {
        let mut world = World::new();
        let e = world.spawn();
        let hits = Arc::new(Mutex::new(0));
        let counter = hits.clone();
        world.observe::<Ping, _>(e, move |world: &mut World, on| {
            *counter.lock().unwrap() += 1;
            world.trigger(Ping { target: on.entity, bubble: false });
        });

        world.trigger(Ping { target: e, bubble: false });
        world.trigger(Ping { target: e, bubble: false });
        assert_eq!(*hits.lock().unwrap(), 2, "the listener was lost by the first dispatch");
    }

    /// A nested trigger at a **different** entity does run — the detachment is per entity, and
    /// this is the half that makes a chain reaction resolvable inside one dispatch.
    #[test]
    fn a_nested_trigger_at_another_entity_runs() {
        let mut world = World::new();
        let first = world.spawn();
        let second = world.spawn();
        let order = Arc::new(Mutex::new(Vec::<u8>::new()));

        let (a, b) = (order.clone(), order.clone());
        world.observe::<Ping, _>(second, move |_world: &mut World, _on| {
            b.lock().unwrap().push(2);
        });
        world.observe::<Ping, _>(first, move |world: &mut World, _on| {
            a.lock().unwrap().push(1);
            world.trigger(Ping { target: second, bubble: false });
            a.lock().unwrap().push(3);
        });

        world.trigger(Ping { target: first, bubble: false });
        assert_eq!(
            *order.lock().unwrap(),
            vec![1, 2, 3],
            "the nested dispatch did not run inside the outer listener"
        );
    }

    /// Bubbling still reaches the ancestors, and each hop's listener gets the world too.
    #[test]
    fn a_bubbling_event_hands_every_hop_the_world() {
        use crate::hierarchy::HierarchyExt;
        let mut world = World::new();
        let parent = world.spawn();
        let child = world.spawn();
        world.add_child(parent, child);

        let seen = Arc::new(Mutex::new(Vec::<u32>::new()));
        let (c, p) = (seen.clone(), seen.clone());
        world.observe::<Ping, _>(child, move |world: &mut World, on| {
            c.lock().unwrap().push(on.entity.id());
            world.spawn();
        });
        world.observe::<Ping, _>(parent, move |world: &mut World, on| {
            p.lock().unwrap().push(on.entity.id());
            world.spawn();
        });

        let before = world.entity_count();
        world.trigger(Ping { target: child, bubble: true });
        assert_eq!(*seen.lock().unwrap(), vec![child.id(), parent.id()], "the walk");
        assert_eq!(world.entity_count(), before + 2, "both hops reached the world");
    }

    /// Builds `root -> mid -> leaf` and returns the three, root first.
    fn three_deep(world: &mut World) -> [Entity; 3] {
        use crate::hierarchy::HierarchyExt;
        let root = world.spawn();
        let mid = world.spawn();
        let leaf = world.spawn();
        world.add_child(root, mid);
        world.add_child(mid, leaf);
        [root, mid, leaf]
    }

    /// Registers a listener on each of `entities` that records its index, and optionally stops
    /// the walk at one of them.
    fn record_walk(world: &mut World, entities: &[Entity], stop_at: Option<usize>) -> Arc<Mutex<Vec<usize>>> {
        let trail = Arc::new(Mutex::new(Vec::new()));
        for (index, entity) in entities.iter().enumerate() {
            let t = trail.clone();
            world.observe::<Ping, _>(*entity, move |world: &mut World, _on| {
                t.lock().unwrap().push(index);
                if stop_at == Some(index) {
                    world.stop_propagation();
                }
            });
        }
        trail
    }

    /// **A listener ends the walk** — the gap `CAPABILITY_GAPS.md` measured as "2 more links
    /// visited after one tried".
    ///
    /// The control is the first assertion: the identical chain with nobody calling
    /// `stop_propagation` visits all three, so the second assertion is measuring the call and
    /// not the hierarchy.
    #[test]
    fn a_listener_can_end_the_walk() {
        let mut world = World::new();
        let [root, mid, leaf] = three_deep(&mut world);
        let free = record_walk(&mut world, &[leaf, mid, root], None);
        world.trigger(Ping { target: leaf, bubble: true });
        assert_eq!(*free.lock().unwrap(), vec![0, 1, 2], "the uncancelled walk");

        let mut world = World::new();
        let [root, mid, leaf] = three_deep(&mut world);
        // Index 1 is `mid`: stop there, and `root` must never hear it.
        let stopped = record_walk(&mut world, &[leaf, mid, root], Some(1));
        world.trigger(Ping { target: leaf, bubble: true });
        assert_eq!(*stopped.lock().unwrap(), vec![0, 1], "the walk continued past the veto");
    }

    /// Stopping at the **target** means the ancestors hear nothing at all.
    #[test]
    fn stopping_at_the_target_reaches_no_ancestor() {
        let mut world = World::new();
        let [root, mid, leaf] = three_deep(&mut world);
        let trail = record_walk(&mut world, &[leaf, mid, root], Some(0));
        world.trigger(Ping { target: leaf, bubble: true });
        assert_eq!(*trail.lock().unwrap(), vec![0]);
    }

    /// The other listeners **on the same entity** still run: the flag is read once that entity
    /// is finished, not between its listeners.
    ///
    /// Worth pinning because either answer is defensible and the docs promise this one.
    #[test]
    fn stopping_does_not_cut_the_entitys_own_listeners_short() {
        let mut world = World::new();
        let [root, mid, leaf] = three_deep(&mut world);
        let _ = (root, mid);
        let seen = Arc::new(Mutex::new(Vec::<u8>::new()));
        let (first, second) = (seen.clone(), seen.clone());
        world.observe::<Ping, _>(leaf, move |world: &mut World, _on| {
            first.lock().unwrap().push(1);
            world.stop_propagation();
        });
        world.observe::<Ping, _>(leaf, move |_world: &mut World, _on| {
            second.lock().unwrap().push(2);
        });

        world.trigger(Ping { target: leaf, bubble: true });
        assert_eq!(*seen.lock().unwrap(), vec![1, 2], "the second listener on the target was skipped");
    }

    /// **A nested walk answers for itself.** An inner dispatch that stops propagation must not
    /// end the outer one — the flag is saved and restored per `trigger`.
    ///
    /// This is the whole reason the flag is not simply a bool anyone can set: without the
    /// save/restore, one unrelated event cancelling itself would silently truncate the walk
    /// that happened to be running.
    #[test]
    fn a_nested_walk_does_not_cancel_the_outer_one() {
        let mut world = World::new();
        let [root, mid, leaf] = three_deep(&mut world);
        let other = world.spawn();

        let trail = Arc::new(Mutex::new(Vec::<usize>::new()));
        for (index, entity) in [leaf, mid, root].iter().enumerate() {
            let t = trail.clone();
            world.observe::<Ping, _>(*entity, move |world: &mut World, _on| {
                t.lock().unwrap().push(index);
                if index == 0 {
                    // A different, unrelated dispatch that cancels itself.
                    world.trigger(Ping { target: other, bubble: false });
                }
            });
        }
        world.observe::<Ping, _>(other, |world: &mut World, _on| world.stop_propagation());

        world.trigger(Ping { target: leaf, bubble: true });
        assert_eq!(
            *trail.lock().unwrap(),
            vec![0, 1, 2],
            "a nested dispatch's cancellation truncated the outer walk"
        );
    }

    /// Calling it with no dispatch running is inert rather than an error, and does not poison
    /// the next `trigger`.
    #[test]
    fn stopping_outside_a_dispatch_is_inert() {
        let mut world = World::new();
        let [root, mid, leaf] = three_deep(&mut world);
        let trail = record_walk(&mut world, &[leaf, mid, root], None);
        world.stop_propagation();
        world.trigger(Ping { target: leaf, bubble: true });
        assert_eq!(*trail.lock().unwrap(), vec![0, 1, 2], "a stale flag truncated the next walk");
    }

    /// An untargeted event carrying a non-`Clone` payload — the bound the global path drops.
    struct LevelLoaded {
        name: String,
    }

    /// **An event that belongs to nothing reaches a listener** — the third door.
    ///
    /// The payload is a `String` on purpose: the entity path requires `Clone` because a
    /// bubbling walk hands the same event to several entities, and this path does not.
    #[test]
    fn a_global_event_reaches_its_listeners_in_order() {
        let mut world = World::new();
        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let (first, second) = (seen.clone(), seen.clone());
        world.observe_global::<LevelLoaded, _>(move |_w: &mut World, e: &LevelLoaded| {
            first.lock().unwrap().push(format!("1:{}", e.name));
        });
        world.observe_global::<LevelLoaded, _>(move |_w: &mut World, e: &LevelLoaded| {
            second.lock().unwrap().push(format!("2:{}", e.name));
        });

        world.trigger_global(LevelLoaded { name: "hangar".into() });
        assert_eq!(*seen.lock().unwrap(), vec!["1:hangar", "2:hangar"]);
    }

    /// The listener gets the world, like the other two doors.
    #[test]
    fn a_global_listener_can_act_on_the_world() {
        let mut world = World::new();
        world.observe_global::<LevelLoaded, _>(|world: &mut World, _e| {
            world.spawn();
        });
        let before = world.entity_count();
        world.trigger_global(LevelLoaded { name: "x".into() });
        assert_eq!(world.entity_count(), before + 1);
    }

    /// Publishing with nobody listening is a no-op, not an error.
    #[test]
    fn a_global_event_with_no_listener_is_a_no_op() {
        let mut world = World::new();
        world.trigger_global(LevelLoaded { name: "x".into() });
    }

    /// Re-entrancy: republishing the **same** type from inside terminates; a **different** type
    /// runs nested. Same rule as the entity path, and for the same reason.
    #[test]
    fn a_global_listener_republishing_its_own_type_terminates() {
        struct Other;
        let mut world = World::new();
        let same = Arc::new(Mutex::new(0));
        let other = Arc::new(Mutex::new(0));
        let (s, o) = (same.clone(), other.clone());

        world.observe_global::<Other, _>(move |_w: &mut World, _e: &Other| {
            *o.lock().unwrap() += 1;
        });
        world.observe_global::<LevelLoaded, _>(move |world: &mut World, _e| {
            *s.lock().unwrap() += 1;
            world.trigger_global(Other);
            world.trigger_global(LevelLoaded { name: "again".into() });
        });

        world.trigger_global(LevelLoaded { name: "first".into() });
        assert_eq!(*same.lock().unwrap(), 1, "the same type re-entered its own dispatch");
        assert_eq!(*other.lock().unwrap(), 1, "a different type failed to run nested");
    }

    /// …and the listeners are put back, so the next publish reaches them again.
    #[test]
    fn global_listeners_survive_the_dispatch_that_took_them() {
        let mut world = World::new();
        let hits = Arc::new(Mutex::new(0));
        let counter = hits.clone();
        world.observe_global::<LevelLoaded, _>(move |_w: &mut World, _e| {
            *counter.lock().unwrap() += 1;
        });

        world.trigger_global(LevelLoaded { name: "a".into() });
        world.trigger_global(LevelLoaded { name: "b".into() });
        assert_eq!(*hits.lock().unwrap(), 2, "the first dispatch consumed the listener");
    }

    /// A listener registered from **inside** a dispatch is kept, and runs from the next one.
    ///
    /// The merge-back has to append rather than overwrite; getting it backwards loses either the
    /// original listeners or the new one, and both losses are silent.
    #[test]
    fn a_listener_registered_during_a_dispatch_is_kept() {
        let mut world = World::new();
        let seen = Arc::new(Mutex::new(Vec::<u8>::new()));
        let (outer, inner) = (seen.clone(), seen.clone());

        world.observe_global::<LevelLoaded, _>(move |world: &mut World, _e| {
            outer.lock().unwrap().push(1);
            let inner = inner.clone();
            world.observe_global::<LevelLoaded, _>(move |_w: &mut World, _e| {
                inner.lock().unwrap().push(2);
            });
        });

        world.trigger_global(LevelLoaded { name: "a".into() });
        assert_eq!(*seen.lock().unwrap(), vec![1], "the new listener ran during its own registration");
        world.trigger_global(LevelLoaded { name: "b".into() });
        assert_eq!(*seen.lock().unwrap(), vec![1, 1, 2], "the registration made inside was lost");
    }

    /// A **global** listener calling `stop_propagation` does not truncate the entity walk it is
    /// nested inside.
    ///
    /// `trigger` saves and restores the flag around its own walk, but a global publish is a
    /// different dispatcher and had been left out of that discipline — a listener on an
    /// unrelated broadcast could end somebody else's bubbling. Found by review 2026-08-24.
    #[test]
    fn a_global_listener_cannot_cancel_an_entity_walk_it_is_inside() {
        struct Beep;
        let mut world = World::new();
        let [root, mid, leaf] = three_deep(&mut world);

        world.observe_global::<Beep, _>(|world: &mut World, _e: &Beep| world.stop_propagation());

        let trail = Arc::new(Mutex::new(Vec::<usize>::new()));
        for (index, entity) in [leaf, mid, root].iter().enumerate() {
            let t = trail.clone();
            world.observe::<Ping, _>(*entity, move |world: &mut World, _on| {
                t.lock().unwrap().push(index);
                if index == 0 {
                    world.trigger_global(Beep);
                }
            });
        }

        world.trigger(Ping { target: leaf, bubble: true });
        assert_eq!(
            *trail.lock().unwrap(),
            vec![0, 1, 2],
            "a global listener's stop_propagation ended the entity walk around it"
        );
    }

    /// A `Parent` cycle terminates the walk instead of hanging the frame.
    ///
    /// `add_child` refuses to build one, which is why the guard is worth having: this test
    /// builds the cycle by writing `Parent` directly, the same way
    /// `gizmo::systems::render::hidden`'s cycle test does, and a scene file can do it too.
    /// `trigger` was the `Parent` walker in this crate without a visited set, while
    /// `despawn_recursive`, `collect_hidden` and transform propagation all carried one. Found
    /// by review 2026-08-24. (Six walkers in other crates still carry none — see the note in
    /// `World::trigger`.)
    ///
    /// A listener attached to an entity does not survive `clear_entities` and fire for that
    /// entity's replacement.
    ///
    /// The same hazard the sparse-set half of this change closes, one turn sharper.
    /// `Entities::clear` resets the GENERATIONS as well as the id counter, so the first entity
    /// spawned after a clear is `Entity(0, gen 0)` — the same 64 bits as the entity 0 that was
    /// just destroyed, not merely the same id. An ordinary `despawn` bumps the generation, and
    /// that bump is exactly what makes a stale `Entity` key harmless everywhere else in the
    /// crate; here there is no bump, so the key matches its unrelated successor exactly.
    ///
    /// The `assert_eq!(b, a)` is a sanity check, not decoration: if the allocator ever stops
    /// resetting generations, the collision this test is about disappears and the rest of it
    /// would pass for the wrong reason.
    ///
    /// Found by review 2026-08-25, in the sweep the sparse-set fix asked for.
    #[test]
    fn clear_entities_drops_entity_listeners() {
        let mut world = World::new();
        let a = world.spawn();

        let hits = Arc::new(Mutex::new(0));
        let c = hits.clone();
        world.observe::<Ping, _>(a, move |_w: &mut World, _on| {
            *c.lock().unwrap() += 1;
        });
        world.trigger(Ping { target: a, bubble: false });
        assert_eq!(*hits.lock().unwrap(), 1, "sanity: the listener is attached and fires");

        world.clear_entities();

        let b = world.spawn();
        assert_eq!(b, a, "sanity: clear_entities hands the handle back bit for bit");

        world.trigger(Ping { target: b, bubble: false });
        assert_eq!(
            *hits.lock().unwrap(),
            1,
            "a listener registered for a cleared entity fired for its replacement"
        );
    }

    /// **The fuse in the listener is load-bearing.** Without the guard the walk is unbounded,
    /// and an unbounded loop has no assertion to disagree with: the plain version of this test
    /// does not fail, it HANGS — measured 2026-08-25, it ran past 60 s and had to be killed.
    /// A suite that hangs covers less than one that goes red, which is the same argument
    /// `CLAUDE.md` makes for `--no-fail-fast`. So the listener cuts the walk itself once the
    /// visit count passes what a two-entity cycle can honestly produce, through the engine's
    /// own `stop_propagation`, and the overrun arrives as a number in the assertion instead of
    /// as a stopped clock.
    #[test]
    fn a_bubbling_walk_terminates_on_a_parent_cycle() {
        use crate::component::Parent;
        /// Two entities, one visit each — anything above this is the walk failing to terminate.
        const FUSE: i32 = 8;

        let mut world = World::new();
        let a = world.spawn();
        let b = world.spawn();
        world.add_component(a, Parent(b.id()));
        world.add_component(b, Parent(a.id()));

        let hits = Arc::new(Mutex::new(0));
        for entity in [a, b] {
            let c = hits.clone();
            world.observe::<Ping, _>(entity, move |w: &mut World, _on| {
                let mut n = c.lock().unwrap();
                *n += 1;
                if *n > FUSE {
                    w.stop_propagation();
                }
            });
        }

        world.trigger(Ping { target: a, bubble: true });

        let hits = *hits.lock().unwrap();
        assert!(
            hits <= FUSE,
            "the walk did not terminate on a `Parent` cycle — the fuse cut it after {hits} visits"
        );
        assert_eq!(hits, 2, "each entity of the cycle is visited exactly once");
    }

    /// Two event types do not see each other's listeners — the map is keyed by the event's own
    /// `TypeId`, and a `downcast_mut` that quietly returned `None` would look like silence.
    #[test]
    fn global_event_types_are_independent() {
        struct A;
        struct B;
        let mut world = World::new();
        let a_hits = Arc::new(Mutex::new(0));
        let counter = a_hits.clone();
        world.observe_global::<A, _>(move |_w: &mut World, _e: &A| *counter.lock().unwrap() += 1);

        world.trigger_global(B);
        assert_eq!(*a_hits.lock().unwrap(), 0, "B woke A's listener");
        world.trigger_global(A);
        assert_eq!(*a_hits.lock().unwrap(), 1);
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
