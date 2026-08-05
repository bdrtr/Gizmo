//! Push-side notification — callbacks that run at the instant something happens, as opposed
//! to the [`event`](crate::event) queues, which systems poll a frame later.
//!
//! Two unrelated mechanisms are spelled with the types in this module:
//!
//! * **Component lifecycle.** `World::add_observer::<T, _>` appends an `on_add` hook for `T`:
//!   a closure called with [`On<Insert, T>`](On) when `T` becomes *newly* present on an
//!   entity. Overwriting a value the entity already carries does not call it. Being a hook, it
//!   also inherits the hook system's gaps — the bulk paths that write archetype columns
//!   directly notify nothing (see [`ComponentHooks`](crate::world::ComponentHooks)).
//! * **Entity-targeted events.** `World::observe` attaches a listener for one
//!   [`EntityEvent`] type to one entity; `World::trigger` dispatches a value to that
//!   entity's listeners and, if the event opts in, on up the `Parent` chain.
//!
//! Both run synchronously on the calling thread, inside the `&mut World` call that caused
//! them, and neither hands the callback a world. A callback that has to change something
//! must go through state it captured — a channel, a shared cell, a
//! [`CommandQueue`](crate::CommandQueue) applied later — which also means a callback cannot
//! re-enter the world and perturb the dispatch it is part of.
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
/// Reserved marker for "component `T` was detached from this entity" — **no dispatch path
/// yet**.
///
/// Nothing in the engine ever constructs one, and no API accepts an `On<Remove, _>`:
/// `World::add_observer` always builds an [`Insert`]. Detachment is observable today only
/// through a raw [`RemoveHook`](crate::world::RemoveHook), which is handed an entity rather
/// than an [`On`].
#[derive(Clone, Copy)]
pub struct Remove;
/// Reserved marker for "an existing component value was overwritten" — **no dispatch path
/// yet**, same as [`Remove`].
///
/// Overwrites are observable today only through a raw [`SetHook`](crate::world::SetHook).
#[derive(Clone, Copy)]
pub struct Replace;

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
/// return channel to the dispatcher — a listener cannot report that it handled the delivery.
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
