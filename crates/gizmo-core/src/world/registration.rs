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

    /// Entity-based Observer registration for custom EntityEvents
    pub fn observe<E: crate::observer::EntityEvent, F>(&mut self, entity: Entity, listener: F) -> &mut Self
    where
        F: FnMut(crate::observer::On<E>) + Send + Sync + 'static,
    {
        let type_id = TypeId::of::<E>();
        let map_any = self.entity_observers.entry(type_id).or_insert_with(|| {
            Box::new(HashMap::<Entity, Vec<Box<dyn FnMut(crate::observer::On<E>) + Send + Sync + 'static>>>::new())
        });

        let map = map_any.downcast_mut::<HashMap<Entity, Vec<Box<dyn FnMut(crate::observer::On<E>) + Send + Sync + 'static>>>>().unwrap();
        map.entry(entity).or_default().push(Box::new(listener));
        self
    }

    /// Triggers an Event and propagates it upwards through the hierarchy (bubble-up)
    pub fn trigger<E: crate::observer::EntityEvent>(&mut self, event: E) {
        use crate::component::Parent;
        let mut current_entity = event.target();

        loop {
            // Observer'ları bu entity için bul ve çalıştır
            let mut hooks_to_run = Vec::new();

            if let Some(map_any) = self.entity_observers.get_mut(&TypeId::of::<E>()) {
                if let Some(map) = map_any.downcast_mut::<HashMap<Entity, Vec<Box<dyn FnMut(crate::observer::On<E>) + Send + Sync + 'static>>>>() {
                    if let Some(listeners) = map.remove(&current_entity) {
                        hooks_to_run = listeners;
                    }
                }
            }

            for mut listener in hooks_to_run.drain(..) {
                let e = crate::observer::On {
                    event: event.clone(),
                    entity: current_entity,
                    _marker: std::marker::PhantomData,
                };
                listener(e);

                // Geri koy
                if let Some(map_any) = self.entity_observers.get_mut(&TypeId::of::<E>()) {
                    if let Some(map) = map_any.downcast_mut::<HashMap<Entity, Vec<Box<dyn FnMut(crate::observer::On<E>) + Send + Sync + 'static>>>>() {
                        map.entry(current_entity).or_default().push(listener);
                    }
                }
            }

            if !event.can_propagate() {
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
    /// Runs immediately after that same write's `on_set` hooks. See [`ReplaceHook`].
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
