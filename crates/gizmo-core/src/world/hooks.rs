//! Lifecycle hooks: callbacks the [`World`] runs when a component is attached to, written
//! to, or detached from an entity, plus a global per-despawn callback.
//!
//! Every hook receives `&mut World`, so a hook may spawn, despawn and mutate freely. While
//! one component type's hooks are running they are temporarily detached from the world and
//! merged back afterwards; consequently a hook that registers *another* hook for the same
//! component type will not see it fire for the event currently in flight — only for
//! subsequent ones, and where the newcomer ends up in the list is not guaranteed to be last.
//! Re-entering the same type's hooks from inside them finds an empty list.
//!
//! Hooks are not part of the deterministic simulation contract: the per-component order in
//! which they fire during a despawn comes from a `HashMap` of component types and is
//! therefore arbitrary.

use super::World;
use crate::entity::Entity;

/// Callback invoked once for every entity [`World::despawn`] actually destroys, regardless of
/// which components it carries. A handle that is already dead when it reaches the despawn loop
/// — a double despawn, or an old generation of a recycled id — is skipped, and no despawn hook
/// runs for it.
///
/// It runs at the very start of the despawn — before any [`RemoveHook`], before the
/// component data is dropped and before the id is returned to the allocator — so the entity
/// is still alive and all of its components are still readable through the `&mut World`.
pub type DespawnHook = Box<dyn FnMut(&mut World, Entity) + Send + Sync>;

/// Callback invoked when the registered component type becomes *newly* present on an
/// entity. Overwriting an existing value does not fire it (that is [`SetHook`] alone).
///
/// It runs after the entity has reached its new archetype, so the component is already
/// readable, and always immediately before that same insert's [`SetHook`]s.
pub type AddHook = Box<dyn FnMut(&mut World, Entity) + Send + Sync>;

/// Callback invoked when the registered component type is detached from an entity.
///
/// When the hook runs relative to the data actually disappearing depends on the path:
/// `World::remove_component`, `World::remove_batch`, `World::remove_bundle` and the sparse
/// half of `World::despawn` fire it *after* the value is gone; the Table-storage half of
/// `World::despawn` fires it *before* the row is dropped, so there the component is still
/// readable. `remove_bundle`'s Table components joined the list on 2026-08-24 — until then
/// they were detached silently, so the same component removed two ways answered
/// differently.
pub type RemoveHook = Box<dyn FnMut(&mut World, Entity) + Send + Sync>;

/// Callback invoked on every write of the registered component type: the initial insert
/// (right after the [`AddHook`]s) and every later overwrite of the same entity's value.
pub type SetHook = Box<dyn FnMut(&mut World, Entity) + Send + Sync>;

/// Callback invoked when a write **overwrote a value the entity already had** — the strict
/// complement of [`AddHook`] within [`SetHook`].
///
/// `on_set` fires on both the first write and every later one, so a caller that wants "the
/// value changed" rather than "the value was written" has to keep its own per-entity ledger
/// to subtract the inserts. This list is that subtraction, done by the dispatcher, which is
/// the only place that knows which case it is in: every site that fires an [`AddHook`] fires
/// no `ReplaceHook`, and every site that fires a `SetHook` without an `AddHook` fires one.
///
/// It is handed the entity only — like every other hook here, it can see neither the value
/// that was overwritten nor the one that replaced it. Reading the component back out of the
/// world gives the **new** value; the old one is already dropped by then, because the write
/// is an assignment (`*ptr = ..`) precisely so a `T: Drop` does not leak.
pub type ReplaceHook = Box<dyn FnMut(&mut World, Entity) + Send + Sync>;

/// The hook lists registered against one component type; the world keeps one of these per
/// `TypeId`. The `World::register_on_*` APIs are what fill them.
///
/// Within one list the hooks run in registration order, with one exception: a hook registered
/// from inside that same type's dispatch is merged back afterwards, and its position relative
/// to the ones already there is not guaranteed (see the module docs above).
///
/// Not every path that gives a component a value runs them. The per-component paths do —
/// `World::add_component`, `remove_component`, `insert_batch`, `remove_batch` and `despawn` —
/// and so does anything routed through them: `World::spawn_bundle` applies a bundle one
/// component at a time whatever it contains, and `World::add_bundle` falls back to that same
/// routing when the bundle carries a sparse component. The bulk paths that write archetype
/// columns directly fire nothing — `World::add_bundle` on an all-Table bundle, every entity
/// after the first in `World::spawn_batch`, and `World::clone_entity`, which materialises
/// whole component sets by copying columns and sparse entries with no hook call anywhere.
/// Hooks are therefore not a complete audit trail of every value the component ever took: a
/// cloned prefab, in particular, appears without firing `on_add` or `on_set`.
#[derive(Default)]
pub struct ComponentHooks {
    /// Filled by `World::register_on_add`, and by `World::add_observer`, which is nothing more
    /// than an `on_add` hook that builds an `On<Insert, T>` for its closure. Entirely skipped
    /// by an overwrite of an existing value.
    pub on_add: Vec<AddHook>,
    /// The only one of the four lists a whole-entity `World::despawn` runs: it fires once per
    /// component type the entity still holds. The order *across* component types comes from a
    /// `HashMap` and is arbitrary.
    pub on_remove: Vec<RemoveHook>,
    /// Run on every *write*, which is not the same as every change: a store of a value equal
    /// to the old one fires them too, and the hook is handed only the entity, so it cannot see
    /// either the old or the new value without reading it back out of the world. On a first
    /// insert they run after [`Self::on_add`], never before.
    pub on_set: Vec<SetHook>,
    /// Run on a write that **replaced an existing value**, and never on a first insert — so
    /// `on_add` and `on_replace` partition the writes that `on_set` sees, with no overlap and
    /// no gap. Filled by `World::register_on_replace` and by `World::add_observer` when its
    /// closure asks for `On<Replace, T>`.
    ///
    /// They run immediately after [`Self::on_set`], not before, so a replace observer sees a
    /// world in which every set hook for the same write has already had its turn.
    pub on_replace: Vec<ReplaceHook>,
}
