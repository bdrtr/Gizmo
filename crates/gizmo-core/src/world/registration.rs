use super::{AddHook, DespawnHook, RemoveHook, SetHook, World};
use crate::archetype::ComponentInfo;
use crate::component::Component;
use crate::entity::Entity;

use std::any::TypeId;
use std::collections::HashMap;

impl World {
    /// Registers the runtime metadata of a given component type.
    /// It is used to create columns during the archetype storage migration stages.
    #[inline]
    pub fn register_component_type<T: Component>(&mut self) {
        let type_id = TypeId::of::<T>();
        self.component_infos
            .entry(type_id)
            .or_insert_with(ComponentInfo::of::<T>);
    }

    /// Registers a lifecycle observer for `T`, on the phase named by the closure's own
    /// `On<E, T>` — [`Insert`](crate::observer::Insert),
    /// [`Replace`](crate::observer::Replace) or [`Remove`](crate::observer::Remove).
    ///
    /// ```
    /// # use gizmo_core::world::World;
    /// # use gizmo_core::observer::{On, Insert, Remove};
    /// # #[derive(Clone, Copy)] pub struct Hp(u32);
    /// # gizmo_core::impl_component!(Hp);
    /// # let mut world = World::new();
    /// world.add_observer(|e: On<Insert, Hp>| println!("gained {:?}", e.entity));
    /// world.add_observer(|e: On<Remove, Hp>| println!("lost {:?}", e.entity));
    /// ```
    ///
    /// Until 2026-08-24 this took only `Insert`, and `Remove`/`Replace` were markers no API
    /// accepted; the generic parameter is why the call has three of them now and the one
    /// turbofish in the tree grew a term. Inference covers the ordinary case — the marker is
    /// already written in the closure's argument type.
    ///
    /// The **hatch** is [`register_on_add`](Self::register_on_add) and its three siblings:
    /// this is exactly one of those hooks with an [`On`](crate::observer::On) built for it, so
    /// a caller who needs the `&mut World` the hook gets and the observer does not can drop
    /// down one layer without giving up the phase. Everything the hook lists document applies
    /// unchanged — including which write paths fire nothing at all.
    ///
    /// Registrations accumulate for the life of the world; there is no unregister, and
    /// registering the same closure twice makes it fire twice.
    pub fn add_observer<E: crate::observer::Lifecycle, T: Component, F>(
        &mut self,
        mut system: F,
    ) -> &mut Self
    where
        F: FnMut(crate::observer::On<E, T>) + Send + Sync + 'static,
    {
        let type_id = TypeId::of::<T>();
        let hooks = self.component_hooks.entry(type_id).or_default();

        E::list(hooks).push(Box::new(move |_world, entity| {
            let event = crate::observer::On {
                event: E::witness(),
                entity,
                _marker: std::marker::PhantomData,
            };
            system(event);
        }));

        self
    }

    /// Attaches a listener for one [`EntityEvent`](crate::observer::EntityEvent) type to one
    /// entity. [`trigger`](Self::trigger) dispatches to it, and on up the `Parent` chain if the
    /// event opts in.
    ///
    /// The listener is handed **`&mut World`** — the same world the dispatch is running inside,
    /// so it can spawn, despawn, write components, read anything, and call `trigger` again:
    ///
    /// ```
    /// # use gizmo_core::world::World;
    /// # use gizmo_core::observer::{EntityEvent, On};
    /// # use gizmo_core::entity::Entity;
    /// # #[derive(Clone)] struct Ping(Entity);
    /// # impl EntityEvent for Ping { fn target(&self) -> Entity { self.0 } }
    /// # let mut world = World::new();
    /// # let e = world.spawn();
    /// world.observe::<Ping, _>(e, |world: &mut World, _on: On<Ping>| {
    ///     world.spawn();
    /// });
    /// ```
    ///
    /// Until 2026-08-24 it was handed the `On` and nothing else, which is why a chain reaction
    /// had to be written as a queue drained by some later system — and therefore advanced one
    /// link per frame. The five lifecycle hook types had taken a `&mut World` all along; this
    /// path was the one with no layer underneath it to drop down to.
    ///
    /// **Re-entrancy.** The listeners for the entity currently being notified are detached from
    /// the world for the duration of the call — the same shape as component hooks — so a
    /// listener that triggers the same event back at its own entity terminates instead of
    /// recursing. Any *other* entity's listeners are live and will run nested.
    ///
    /// Listeners on one entity and event type run in registration order. There is no
    /// unregister; registering the same closure twice makes it fire twice.
    pub fn observe<E: crate::observer::EntityEvent, F>(&mut self, entity: Entity, listener: F) -> &mut Self
    where
        F: FnMut(&mut World, crate::observer::On<E>) + Send + Sync + 'static,
    {
        let type_id = TypeId::of::<E>();
        let map_any = self.entity_observers.entry(type_id).or_insert_with(|| {
            Box::new(HashMap::<Entity, Vec<crate::observer::EntityListener<E>>>::new())
        });

        let map = map_any
            .downcast_mut::<HashMap<Entity, Vec<crate::observer::EntityListener<E>>>>()
            .unwrap();
        map.entry(entity).or_default().push(Box::new(listener));
        self
    }

    /// Triggers an Event and propagates it upwards through the hierarchy (bubble-up)
    pub fn trigger<E: crate::observer::EntityEvent>(&mut self, event: E) {
        use crate::component::Parent;
        let mut current_entity = event.target();

        // Saved and restored around this walk. A listener may itself `trigger`, and that nested
        // walk gets a clean flag; whatever it decided about its own propagation must not end
        // the walk it was called from. Restored on every exit path — hence the single `break`
        // discipline in the loop below rather than early returns.
        let outer_stop = std::mem::replace(&mut self.propagation_stopped, false);

        loop {
            // Observer'ları bu entity için bul ve çalıştır
            let mut hooks_to_run = Vec::new();

            if let Some(map_any) = self.entity_observers.get_mut(&TypeId::of::<E>()) {
                if let Some(map) =
                    map_any.downcast_mut::<HashMap<Entity, Vec<crate::observer::EntityListener<E>>>>()
                {
                    if let Some(listeners) = map.remove(&current_entity) {
                        hooks_to_run = listeners;
                    }
                }
            }

            for listener in hooks_to_run.iter_mut() {
                let e = crate::observer::On {
                    event: event.clone(),
                    entity: current_entity,
                    _marker: std::marker::PhantomData,
                };
                // This entity's WHOLE list is out of the map for the whole of this loop, so
                // triggering the event back at this entity from inside finds nothing and
                // terminates. Any other entity's listeners are live and run nested.
                listener(self, e);
            }

            // Put back after the loop, not inside it. Inside, a listener that had already
            // finished was live again by the time the next one ran, so a nested trigger at this
            // same entity re-ran it — the exact case the termination guarantee is about, and
            // invisible to a test with only one listener. Found by review 2026-08-24.
            //
            // Anything registered during the dispatch is in the map now; the originals go back
            // in front of it, which is what keeps "listeners run in registration order" true.
            if !hooks_to_run.is_empty() {
                if let Some(map_any) = self.entity_observers.get_mut(&TypeId::of::<E>()) {
                    if let Some(map) = map_any
                        .downcast_mut::<HashMap<Entity, Vec<crate::observer::EntityListener<E>>>>()
                    {
                        let slot = map.entry(current_entity).or_default();
                        hooks_to_run.append(slot);
                        *slot = hooks_to_run;
                        hooks_to_run = Vec::new();
                    }
                }
            }

            // The listener's answer, read after it returns rather than taken from it: the
            // return channel is the `&mut World` it already holds.
            if self.propagation_stopped || !event.can_propagate() {
                break;
            }

            // Propagate to parent
            if let Some(parent_ptr) = self.get_component_ptr(current_entity, TypeId::of::<Parent>()) {
                // `Parent` stores a bare id with no generation; a plain `despawn(parent)`
                // (not despawn_recursive) leaves children with a dangling `Parent(id)`.
                // Resolve it safely — a dead id stops propagation instead of panicking.
                // SAFETY: keyed by `TypeId::of::<Parent>()`, so the cast matches the bytes; the
                // id is copied out immediately and resolved through `entity()`, which is what
                // makes a dead parent stop the walk instead of dangling.
                let parent_id = unsafe { (*(parent_ptr as *const Parent)).0 };
                match self.entity(parent_id) {
                    Some(e) => current_entity = e,
                    None => break,
                }
            } else {
                break;
            }
        }

        self.propagation_stopped = outer_stop;
    }

    /// Attaches a listener for an event that belongs to **no entity and no component type** —
    /// "the level loaded", "the round ended", "the save finished".
    ///
    /// The third door. [`add_observer`](Self::add_observer) is keyed to a component type and
    /// [`observe`](Self::observe) to a target entity; until 2026-08-24 an event that was neither
    /// had no synchronous route at all and had to be an [`Events<T>`](crate::event::Events)
    /// queue, read by a system a frame later.
    ///
    /// ```
    /// # use gizmo_core::world::World;
    /// # let mut world = World::new();
    /// struct LevelLoaded { name: String }
    ///
    /// world.observe_global::<LevelLoaded, _>(|world: &mut World, e: &LevelLoaded| {
    ///     let _ = (&e.name, world.entity_count());
    /// });
    /// world.trigger_global(LevelLoaded { name: "hangar".into() });
    /// ```
    ///
    /// The listener takes the event **by reference** and the event needs no `Clone`: there is no
    /// hierarchy walk here, so no second recipient needs a second copy. An event carrying a
    /// `String` or a `Vec` costs nothing to publish.
    ///
    /// **Choosing between this and `Events<T>`.** This is synchronous — the listeners run inside
    /// the `trigger_global` call, in registration order, and can see and change the world before
    /// it returns. An `Events<T>` queue is the opposite trade and still the right one when the
    /// reaction wants to be a scheduled system: batched, parallelisable, and able to declare its
    /// access.
    ///
    /// Re-entrancy follows the same rule as the rest of this module: the listener list is owned
    /// by the dispatch for its duration, so a listener that publishes the **same** event type
    /// terminates instead of recursing, while a different event type runs nested.
    ///
    /// Registrations accumulate for the life of the world; there is no unregister.
    pub fn observe_global<E: Send + Sync + 'static, F>(&mut self, listener: F) -> &mut Self
    where
        F: FnMut(&mut World, &E) + Send + Sync + 'static,
    {
        let type_id = TypeId::of::<E>();
        let list_any = self
            .global_observers
            .entry(type_id)
            .or_insert_with(|| Box::new(Vec::<crate::observer::GlobalListener<E>>::new()));

        let list = list_any
            .downcast_mut::<Vec<crate::observer::GlobalListener<E>>>()
            .expect("global observer list is keyed by the event's own TypeId");
        list.push(Box::new(listener));
        self
    }

    /// Publishes an untargeted event to every [`observe_global`](Self::observe_global) listener
    /// for `E`, synchronously, in registration order.
    ///
    /// Nothing happens if there are none — publishing into an empty world is not an error, which
    /// is what lets a subsystem announce things nobody has subscribed to yet.
    pub fn trigger_global<E: Send + Sync + 'static>(&mut self, event: E) {
        let type_id = TypeId::of::<E>();
        // Taken out for the duration, like every other dispatch here: a listener that publishes
        // `E` again finds an empty list and stops, rather than recursing forever.
        let mut listeners: Vec<crate::observer::GlobalListener<E>> = match self
            .global_observers
            .get_mut(&type_id)
            .and_then(|any| any.downcast_mut::<Vec<crate::observer::GlobalListener<E>>>())
        {
            Some(list) => std::mem::take(list),
            None => return,
        };

        // Saved and restored for the same reason `trigger` does it: a global publish can happen
        // from inside an entity walk, and a global listener calling `stop_propagation` must not
        // truncate that walk. Found by review 2026-08-24 — the global path had been left out.
        let outer_stop = std::mem::replace(&mut self.propagation_stopped, false);

        for listener in &mut listeners {
            listener(self, &event);
        }

        self.propagation_stopped = outer_stop;

        // Put back, and keep anything registered from inside the dispatch — appended after,
        // matching how component hooks merge a registration made during their own run.
        if let Some(list) = self
            .global_observers
            .get_mut(&type_id)
            .and_then(|any| any.downcast_mut::<Vec<crate::observer::GlobalListener<E>>>())
        {
            listeners.append(list);
            *list = listeners;
        }
    }

    /// Ends the current [`trigger`](Self::trigger) walk after the running listener returns.
    ///
    /// Call it from inside a [`observe`](Self::observe) listener: the remaining listeners on
    /// *this* entity still run — the flag is read once the entity is finished, not between
    /// listeners — and the walk then stops instead of continuing to the ancestors.
    ///
    /// ```
    /// # use gizmo_core::world::World;
    /// # use gizmo_core::observer::{EntityEvent, On};
    /// # use gizmo_core::entity::Entity;
    /// # #[derive(Clone)] struct Damage { target: Entity, amount: f32 }
    /// # impl EntityEvent for Damage {
    /// #     fn target(&self) -> Entity { self.target }
    /// #     fn can_propagate(&self) -> bool { true }
    /// # }
    /// # let mut world = World::new();
    /// # let e = world.spawn();
    /// world.observe::<Damage, _>(e, |world: &mut World, on: On<Damage>| {
    ///     if on.event.amount > 50.0 {
    ///         world.stop_propagation();   // armour absorbed it; the parent never hears
    ///     }
    /// });
    /// ```
    ///
    /// **Nesting.** `trigger` saves the flag and restores it when its walk ends, so a listener
    /// that triggers another event and *that* event's listener stops propagation does not end
    /// the outer walk. Each walk answers for itself.
    ///
    /// Outside a dispatch it sets a flag the next `trigger` immediately clears, so it is inert
    /// rather than an error — there is no dispatch to end.
    ///
    /// The channel is the world rather than the listener's return type on purpose: handing the
    /// listener a `&mut World` (2026-08-24) is what made a return channel exist at all, and
    /// `CAPABILITY_GAPS.md` had recorded the missing cancel as needing one.
    pub fn stop_propagation(&mut self) {
        self.propagation_stopped = true;
    }

    /// Is a given component type registered?
    #[inline]
    pub fn is_component_registered<T: Component>(&self) -> bool {
        self.component_infos.contains_key(&TypeId::of::<T>())
    }

    /// The number of registered component metadata entries.
    #[inline]
    pub fn registered_component_count(&self) -> usize {
        self.component_infos.len()
    }

    /// Appends a hook fired when `T` becomes newly present on an entity — not when an
    /// existing value is overwritten. See [`AddHook`] for exactly when in the insert it
    /// runs.
    ///
    /// Hooks accumulate: registering the same closure twice makes it fire twice, and there
    /// is no unregister. Within a type they run in registration order — except for one
    /// registered from inside that same type's dispatch, whose position afterwards is not
    /// guaranteed (see [`ComponentHooks`](crate::world::ComponentHooks)) — and all `on_add`
    /// hooks precede that insert's `on_set` hooks.
    pub fn register_on_add<T: Component>(&mut self, hook: AddHook) {
        self.component_hooks
            .entry(TypeId::of::<T>())
            .or_default()
            .on_add
            .push(hook);
    }

    /// Appends a hook fired when `T` is detached from an entity, whether explicitly or
    /// because the entity was despawned. Same accumulate-and-never-unregister rules as
    /// [`World::register_on_add`].
    ///
    /// Read [`RemoveHook`] before relying on it: whether the component is still readable
    /// when the hook runs depends on the removal path and on `T`'s storage type.
    pub fn register_on_remove<T: Component>(&mut self, hook: RemoveHook) {
        self.component_hooks
            .entry(TypeId::of::<T>())
            .or_default()
            .on_remove
            .push(hook);
    }

    /// Appends a hook fired on every write of `T`: the initial insert (right after the
    /// `on_add` hooks) and every later overwrite of the same entity's value. Same
    /// accumulate-and-never-unregister rules as [`World::register_on_add`].
    ///
    /// It is a *write* notification, not a change notification — the hook fires even when
    /// the new value equals the old one, and it cannot see either value except by reading
    /// the entity out of the `&mut World` it is handed.
    pub fn register_on_set<T: Component>(&mut self, hook: SetHook) {
        self.component_hooks
            .entry(TypeId::of::<T>())
            .or_default()
            .on_set
            .push(hook);
    }

    /// Appends a hook fired only when a write **overwrote a value the entity already had** —
    /// the strict complement of [`World::register_on_add`] within [`World::register_on_set`].
    /// Same accumulate-and-never-unregister rules.
    ///
    /// Use it when "the value changed hands" is the question and the first insert is not an
    /// answer to it: an `on_set` hook counting writes has to subtract the inserts itself, and
    /// doing that from inside the hook means keeping a per-entity ledger, because the hook is
    /// handed only the entity and cannot tell the two cases apart. The dispatcher can, so it
    /// does it here.
    ///
    /// Runs immediately after that same write's `on_set` hooks. See [`ReplaceHook`](crate::world::ReplaceHook).
    ///
    /// This is the **hatch under [`add_observer`](Self::add_observer)**. That one hands the
    /// closure an `On<Replace, T>` and nothing else; this one hands it the `&mut World` the
    /// dispatch is running inside, which is the only way a replace notification can read the
    /// new value, look at the rest of the entity, or queue work. The observer is the shorter
    /// spelling of the same list — not a different mechanism.
    pub fn register_on_replace<T: Component>(&mut self, hook: crate::world::ReplaceHook) {
        self.component_hooks
            .entry(TypeId::of::<T>())
            .or_default()
            .on_replace
            .push(hook);
    }

    /// Appends a hook fired once for every entity [`World::despawn`] actually destroys,
    /// whatever components it carries — the place for teardown that no single component owns.
    /// Handles that are already dead when despawn reaches them are skipped, so a double
    /// despawn fires the hook once, not twice.
    ///
    /// Unlike the `on_*` hooks this one is global, not keyed by component type. It runs
    /// before any `on_remove` hook and before the id is freed, so the entity is still alive
    /// and fully readable. Hooks accumulate and cannot be unregistered; one registered from
    /// inside a despawn hook does not run for the entity currently being despawned.
    pub fn register_despawn_hook(&mut self, hook: DespawnHook) {
        self.despawn_hooks.push(hook);
    }
}
