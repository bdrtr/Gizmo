//! The filter every **system** query carries without asking for it.
//!
//! # What this is for
//!
//! Disabling an entity — ignoring it without destroying it — is not a marker component; the marker
//! is trivial. The feature is the *implicit filter*: an engine that silently adds `Without<Disabled>`
//! to every query makes the safe behaviour the default, and forces the exception to be written down.
//! Without it the default leaks and the safety is what you have to remember, once per query, for
//! ever. `demo/src/bin/entity_disabling.rs` measured the cost of forgetting once: two systems doing
//! the same work, one remembering `Without<Disabled>` and one not, diverge by **543 extra updates
//! by frame 240**.
//!
//! # Where it applies, and why the line is drawn *there*
//!
//! A [`Query`](crate::query::Query) **declared as a system parameter** is filtered. Nothing else
//! is: not `World::query`, not `borrow`, not `query_entity`, and — this is the part that matters —
//! not [`query_unchecked`](crate::world::World::query_unchecked) or `borrow_mut_unchecked`.
//!
//! The first version of this feature filtered `query_unchecked`, on the theory that it was "the
//! system path" because the parameter impl goes through it. **Review rejected that, and it was
//! right.** That one `pub unsafe fn` is simultaneously the parameter route *and* the engine's
//! mutable-from-shared hatch: **98 call sites in 37 files** use it, and **nine of those files are
//! editor panels — seven of them inspector panels** — egui draw functions taking `&World`, with no system anywhere near
//! them. Filtering there did not draw a line; it filtered a population defined by "needed `&mut`
//! from a shared borrow". Four consequences were traced in the code before it shipped:
//!
//! - the **editor inspector went blank** for a disabled entity, and the scene-view manipulator with
//!   it, so there was no UI left to re-enable it from;
//! - **transform propagation froze whole subtrees** — its BFS enqueues children inside the `if let`
//!   that fetches the parent's `GlobalTransform`, so a disabled node stranded every *enabled*
//!   descendant in stale world space;
//! - `sync_bodies` **destroyed** a disabled rigid body rather than pausing it: a body absent from
//!   the incoming list is removed, which permutes the component arrays, leaves joints dangling and
//!   invalidates earlier rollback snapshots. "Disabled" is supposed to mean the data is kept;
//! - netcode rollback captured **unfiltered** and restored **filtered**, which is a desync.
//!
//! So the boundary is the parameter, and it is one function: `SystemParam for Query`'s fetch.
//! Measured for what it covers — **44** demo files and **3** engine files declare a `Query`
//! parameter; game code writes systems, the engine reads the world directly.
//!
//! # What that means it does NOT do, in plain words
//!
//! Disabling an entity hides it from **your systems' queries**. It does **not** pause the engine:
//! physics still simulates it, transform propagation still updates it, audio still plays it,
//! rollback still captures it. Those all read the world through the hatch above, and the four
//! consequences listed there are why they must keep doing so. If you want a disabled entity to
//! stop moving, stop it — the marker is a *view*, not a *state*.
//!
//! # It declares no access, and that is not an oversight
//!
//! A filtered query does **not** report a read of the marker type to the scheduler, so registering
//! a filter does not put every query in the world into conflict with the system that adds or
//! removes the marker. Three reasons, and all three have to hold:
//!
//! 1. The test is `Archetype::has_component` — the archetype's column-index map, not component
//!    memory. `With`/`Without` declare nothing for exactly this reason, and this is a `Without`.
//! 2. Adding or removing the marker is a **structural** change, which a system can only make
//!    through `Commands`, and those are applied between batches. The archetype set is therefore
//!    fixed for the whole of a batch.
//! 3. The filter is evaluated once, when the query is constructed, from the same `&World` every
//!    other query in the batch is reading.
//!
//! Had it declared a read, a game with one filter registered would have serialised its entire
//! schedule against a single marker-writing system — the feature would have cost more than the
//! bug it removes.
//!
//! No test guards this, and that is deliberate rather than an omission: `SystemParam::get_access_info`
//! is a *static* function with no access to a `World`, so it cannot see the registered filters even
//! if it wanted to. The property holds by construction, and a test asserting it could not fail.
//!
//! # The escape hatch
//!
//! [`IgnoreDefaultFilters`](crate::query::IgnoreDefaultFilters) as a query operand. A system that
//! wants to see disabled entities — the one that re-enables them — writes it, and it is visible in
//! the signature rather than in a comment.
//!
//! # What it does not do
//!
//! Only **`Table`-stored** components can be filters: the test is per archetype, and a `SparseSet`
//! component does not live in one. [`DefaultQueryFilters::add`] panics rather than accepting a
//! sparse marker and quietly filtering nothing. There is no per-filter opt-out either — the hatch
//! is all-or-nothing for one query, because "ignore the second of three default filters" is a
//! question no caller has yet had.

use std::any::TypeId;

/// The component types whose presence hides an entity from every **system** query.
///
/// Empty by default: the feature costs nothing and changes nothing until a game asks for it. See
/// the [module docs](self) for where it applies and where it deliberately does not.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct DefaultQueryFilters {
    /// Excluded types, in registration order. A `Vec` rather than a set because it holds a handful
    /// of entries and is walked once per archetype per query construction — a hash lookup would
    /// cost more than the scan it replaced.
    excluded: Vec<TypeId>,
}

impl DefaultQueryFilters {
    /// No filters — the state every `World` starts in.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds `T`, so an entity carrying it is invisible to system queries. Idempotent.
    ///
    /// # Panics
    ///
    /// If `T` is `SparseSet`-stored. The test this drives is per **archetype**, and a sparse
    /// component lives outside archetypes — accepting one would filter nothing at all, silently,
    /// which is the exact failure mode the feature exists to remove. Give the marker the default
    /// `Table` storage.
    pub fn add<T: crate::component::Component>(&mut self) {
        assert_eq!(
            T::storage_type(),
            crate::component::StorageType::Table,
            "a default query filter must be a Table-stored component, and {} is SparseSet. \
             The filter is tested per archetype and a sparse component does not live in one, so \
             this would have filtered nothing — silently.",
            std::any::type_name::<T>(),
        );
        let tid = TypeId::of::<T>();
        if !self.excluded.contains(&tid) {
            self.excluded.push(tid);
        }
    }

    /// Removes `T`. Returns whether it was there.
    pub fn remove<T: crate::component::Component>(&mut self) -> bool {
        let tid = TypeId::of::<T>();
        let before = self.excluded.len();
        self.excluded.retain(|&t| t != tid);
        self.excluded.len() != before
    }

    /// Whether anything is filtered at all — the check that keeps an unused feature free.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.excluded.is_empty()
    }

    /// How many types are registered.
    #[must_use]
    pub fn len(&self) -> usize {
        self.excluded.len()
    }

    /// Whether `T` is registered.
    #[must_use]
    pub fn contains<T: crate::component::Component>(&self) -> bool {
        self.excluded.contains(&TypeId::of::<T>())
    }

    /// Whether this archetype holds any filtered component — i.e. whether a system query should
    /// skip it entirely.
    #[must_use]
    pub fn excludes(&self, arch: &crate::archetype::Archetype) -> bool {
        self.excluded.iter().any(|&tid| arch.has_component(tid))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::StorageType;

    #[derive(Clone, Copy)]
    struct Disabled;
    crate::impl_component!(Disabled);

    #[derive(Clone, Copy)]
    struct Frozen;
    crate::impl_component!(Frozen; StorageType::SparseSet);

    #[test]
    fn a_world_starts_with_no_filters() {
        let f = DefaultQueryFilters::new();
        assert!(f.is_empty());
        assert_eq!(f.len(), 0);
        assert!(!f.contains::<Disabled>());
    }

    #[test]
    fn adding_is_idempotent_and_removal_reports() {
        let mut f = DefaultQueryFilters::new();
        f.add::<Disabled>();
        f.add::<Disabled>();
        assert_eq!(f.len(), 1, "the same type was registered twice");
        assert!(f.contains::<Disabled>());
        assert!(f.remove::<Disabled>());
        assert!(!f.remove::<Disabled>(), "removing what is not there reported success");
        assert!(f.is_empty());
    }

    /// A sparse marker is refused loudly rather than accepted and ignored.
    ///
    /// This is the panic worth having: the archetype test cannot see a sparse component, so
    /// accepting one would leave every entity visible while the code reads as though it had been
    /// filtered — the precise failure the feature removes, reintroduced by its own API.
    #[test]
    #[should_panic(expected = "must be a Table-stored component")]
    fn a_sparse_marker_is_refused() {
        DefaultQueryFilters::new().add::<Frozen>();
    }
}
