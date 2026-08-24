//! Push-side notification — callbacks that run at the instant something happens, as opposed
//! to the [`event`](crate::event) queues, which systems poll a frame later.
//!
//! Two unrelated mechanisms are spelled with the types in this module:
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
//!   entity's listeners and, if the event opts in, on up the `Parent` chain.
//!
//! Both run synchronously on the calling thread, inside the `&mut World` call that caused
//! them, and **both reach a `&mut World`** — the entity-event listener is handed one
//! directly, and a lifecycle observer's `On` is built by a hook that has one, so the world is
//! one layer down at [`register_on_add`](crate::world::World::register_on_add) and its
//! siblings. Until 2026-08-24 neither did, which is why a chain reaction had to be written as
//! a captured queue drained by a later system, and therefore advanced one link per frame.
//!
//! **Re-entrancy is bounded by detachment, not by prohibition.** While a callback runs, the
//! list it came from is out of the world: a component type's hook lists during a hook, and one
//! entity's listeners during a trigger. So a callback that provokes the same notification at
//! the same target terminates instead of recursing, while any *other* target's callbacks are
//! live and run nested.
//!
//! Listeners registered against the same entity and event type run in registration order.
//! Neither mechanism has an unregister: registrations accumulate for the life of the world,
//! and registering the same closure twice makes it fire twice.

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
/// The strict complement of [`Insert`] within a write: every write fires the component's set
/// hooks, and exactly one of `On<Insert, T>` or `On<Replace, T>` alongside them — never both,
/// never neither. That is the distinction an observer cannot draw for itself, because it is
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
/// `World::register_on_despawn`, which is not per-component and so cannot be an `On<_, T>`.
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
    /// walks up the `Parent` chain, running each ancestor's listeners for this event type,
    /// and stops at the first entity that has no `Parent` component or whose recorded parent
    /// id is no longer alive. Listeners along the chain all receive equal clones of the same
    /// event; none of them can cancel the walk.
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
/// It is handed to the listener *by value* and the listener returns `()`, so there is no
/// return channel to the dispatcher — a listener cannot report that it handled the delivery,
/// and in particular cannot stop a propagating walk (that gap is still open). What it *can*
/// do is act, because the entity-event listener also receives the `&mut World`.
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
