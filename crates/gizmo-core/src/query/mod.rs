//! Reading and writing components: [`Query`], its filters, and its iterators.
//!
//! A query names the components it touches and the engine derives from that both which
//! entities match and which other systems it can run beside. Access is checked, so two
//! aliasing `&mut` views of the same component cannot be built in safe code.
//!
//! Filters (`With`, `Without`, `Changed`, `Added`) narrow the match without contributing
//! data. Note which ones are per-ROW rather than per-archetype: the chunked iterators cannot
//! serve those and reject them up front.
use crate::archetype::Archetype;
use crate::entity::Entity;
use crate::world::World;
use std::any::TypeId;
use std::marker::PhantomData;

mod fetch;
mod iter;

pub use fetch::{FetchComponent, Mut};
pub use iter::{QueryChunksIter, QueryIter};

// =========================================================================
// SEALED PATTERN
// =========================================================================
//
// `FetchComponent` ve `WorldQuery` motorun içsel, tamamı `unsafe` metodlardan
// oluşan DSL trait'leridir. Kullanıcının manuel impl etmesi İSTENMEZ (yanlış
// bir impl aliasing/UB ihlali doğurur) ve cross-crate hiçbir impl yoktur.
// Sealed supertrait deseni hem kaçak manuel impl'leri engeller hem de gelecekte
// metod eklemeyi non-breaking yapar.
mod sealed {
    pub trait SealedFetch {}
    pub trait SealedQuery {}
    pub trait SealedReadOnly {}
}

// =========================================================================
// WORLD QUERY TRAIT
// =========================================================================

/// One operand — or a tuple of operands — of a [`Query`].
///
/// Implemented for `&T`, [`Mut<T>`](Mut), the filters ([`With`], [`Without`], [`Changed`],
/// [`Added`], [`Or`]) and for tuples of 2 to 12 of those. It is *sealed*: nothing outside this
/// crate can implement it, and every accessor is `unsafe` and speaks in raw pointers, so it is
/// an internal DSL rather than an extension point. Being sealed also means methods may be added
/// in a point release without that counting as a breaking change.
///
/// Matching happens in two stages, and the split is the thing to understand before touching an
/// impl:
///
/// 1. [`matches_archetype`](WorldQuery::matches_archetype) — coarse, per archetype, evaluated
///    once when the query is built. Selects which archetypes are visited at all.
/// 2. [`filter_row`](WorldQuery::filter_row) — fine, per row, evaluated on every visited row.
///
/// Stage 1 is allowed to be *wider* than the truth and stage 2 narrows it. Components stored in
/// a `SparseSet` live outside archetypes, so their stage 1 answers `true` everywhere and the
/// real test happens in stage 2. [`has_row_filter`](WorldQuery::has_row_filter) reports whether
/// stage 2 carries any meaning for this query, which is what lets chunk iteration reject the
/// queries it cannot honour.
pub trait WorldQuery: sealed::SealedQuery {
    /// A `'static` stand-in used as the cache key for this query's archetype-match list
    /// (see `World::query_cached`). The query machinery never constructs a value of it — only
    /// `TypeId::of::<Self::StaticType>()` is ever taken — so it is free to name an ordinary
    /// data-carrying type, and for `&T`/[`Mut<T>`](Mut) it names the component `T` itself.
    /// Queries that map to the same `StaticType` share one cache entry, so they must have
    /// identical `matches_archetype` behaviour; `&T` and `Mut<T>` deliberately collapse to the
    /// same key because their archetype predicates are identical.
    type StaticType: 'static;
    /// Per-archetype resolved access: the raw pointers (or, for sparse storage, the address of
    /// the sparse set) that `get_item`/`filter_row`/`get_slice` index into. Produced once per
    /// archetype by [`fetch_raw`](WorldQuery::fetch_raw).
    ///
    /// `Copy` is required because the same fetch is handed to every row of the archetype and is
    /// copied into each worker task of a parallel iteration. It holds no borrow of its own: it
    /// is valid only while the `&'w World` it was derived from is still borrowed and the
    /// archetype has not been structurally modified.
    type Fetch<'w>: Copy;
    /// What one row yields: `&'w T` for `&T`, [`Mut<'w, T>`](Mut) for `Mut<T>`, and `()` for
    /// every filter — `With`/`Without`/`Changed`/`Added`/`Or` decide membership but carry no
    /// data. For a tuple, the tuple of the operands' items; filter operands still occupy their
    /// `()` slot in it.
    type Item<'w>;
    /// What one *whole archetype* yields in chunk iteration: `&'w [T]` for `&T`, `&'w mut [T]`
    /// for `Mut<T>`, `()` for filters. Only Table storage is contiguous — for a `SparseSet`
    /// component [`get_slice`](WorldQuery::get_slice) panics rather than fabricating a slice.
    type Slice<'w>;

    /// # Safety
    /// The archetype must be valid and the returned fetch pointer must stay valid for the whole lifetime of the archetype.
    unsafe fn fetch_raw<'w>(world: &'w World, arch: &Archetype, system_tick: u32) -> Option<Self::Fetch<'w>>;

    /// Appends this query's component access to `types` as `(TypeId, is_mut)` pairs, and
    /// **panics** if the accumulated set would be unsound.
    ///
    /// Unsound means: the same `TypeId` appears twice with at least one of the two mutable,
    /// which would hand out two `&mut T` — or a `&mut T` next to a `&T` — for the same row.
    /// So `(Mut<A>, Mut<A>)` and `(&A, Mut<A>)` panic, while `(&A, &A)` and `(Mut<A>, Mut<B>)`
    /// are fine. The panic happens when the query is *constructed*, not when it is iterated.
    /// This cannot be a compile-time check because `TypeId` equality is not comparable in a
    /// const context.
    ///
    /// Filters must declare their access too, even though they yield no data:
    /// `Changed<T>`/`Added<T>` declare a *read* of `T` (they read the same `ComponentTicks`
    /// that `Mut<T>` writes), and `Or` forwards both operands. `With`/`Without` declare
    /// nothing — they touch no component memory. The same pairs are consumed by the system
    /// scheduler to classify a system as a reader or writer of each component.
    fn check_aliasing(types: &mut Vec<(TypeId, bool)>);

    /// Coarse, per-archetype admission test. Evaluated once for every archetype when the query
    /// is built; only archetypes answering `true` are ever visited, and an archetype rejected
    /// here can never be recovered by a row filter.
    ///
    /// It may be deliberately WIDER than the real predicate, and for `SparseSet`-stored
    /// components it is: their data lives outside archetypes, so those impls answer `true` for
    /// every archetype and leave the actual membership test to
    /// [`filter_row`](WorldQuery::filter_row). It must never be narrower than the truth.
    fn matches_archetype(arch: &Archetype) -> bool;

    /// # Safety
    /// The `row` value must be smaller than the archetype's element count.
    unsafe fn get_item<'w>(fetch: Self::Fetch<'w>, row: usize, entity_id: u32) -> Self::Item<'w>;

    /// # Safety
    /// A valid fetch and a `row` within the archetype's bounds must be supplied.
    unsafe fn filter_row<'w>(fetch: Self::Fetch<'w>, row: usize, entity_id: u32, system_tick: u32) -> bool;

    /// # Safety
    /// The `len` value must not exceed the archetype's element count.
    unsafe fn get_slice<'w>(fetch: Self::Fetch<'w>, len: usize) -> Self::Slice<'w>;

    /// Does this query REQUIRE per-row (`filter_row`) narrowing — that is, is
    /// `matches_archetype` deliberately WIDE and the real test in `filter_row`?
    /// `true` for SparseSet `With`/`Without` (matches is true on every archetype) and for
    /// `Changed`/`Added`/`Or` (per-row by their nature). Since `iter_chunks` returns the
    /// archetype's ENTIRE contiguous slice it CANNOT HONOUR these filters → it rejects such
    /// queries (see [`Query::iter_chunks`]). `false` for Table `With`/`Without`
    /// (matches_archetype suffices) → chunk iteration with them is safe.
    fn has_row_filter() -> bool {
        false
    }
}

// =========================================================================
// READ-ONLY QUERY MARKER
// =========================================================================
//
// Marks queries that yield ONLY shared (`&T`) access — never `&mut T`. Such a query
// is sound to construct and iterate from a shared `&World`: any number can coexist
// because no `&mut T` ever escapes. `Mut<T>` is deliberately NOT `ReadOnlyQuery`.
//
// This is what makes the safe entry points sound:
// - [`World::query`](crate::world::World::query) bounds `Q: ReadOnlyQuery`, so a
//   mutable query can never be built from `&World` in safe code (the dual-`Mut` UB).
// - [`Query`] gates its `&self` accessors (`iter`/`get`/`iter_chunks`/`par_for_each`)
//   behind `ReadOnlyQuery`; mutable access goes through the `&mut self` variants, so
//   two live `&mut T` to the same storage are impossible without `unsafe`.
//
// Sealed: only this crate implements it (a wrong impl on a `Mut`-bearing query would
// reopen the hole), and the supertrait `WorldQuery` keeps it inside the sealed DSL.
/// Marker for queries that yield **only** shared access — `&T` and the data-less filters,
/// never [`Mut<T>`](Mut).
///
/// This is the bound that makes the shared entry points sound. Because no `&mut T` can escape
/// such a query, any number of them may coexist over the same `&World`. Accordingly:
///
/// - [`World::query`](crate::world::World::query) requires it, so a mutable query cannot be
///   built from a shared borrow in safe code;
/// - [`Query`] gates its `&self` accessors (`iter`, `get`, `iter_chunks`, `par_for_each`)
///   behind it, so mutable access has to go through the `&mut self` variants.
///
/// `Mut<T>` is pointedly *not* an implementor. A tuple implements it only when every element
/// does, and `Or<A, B>` only when both operands do.
///
/// Sealed — implementing it for a `Mut`-bearing query would reopen exactly the aliasing hole it
/// exists to close.
pub trait ReadOnlyQuery: WorldQuery + sealed::SealedReadOnly {}

// =========================================================================
// QUERY STRUCT
// =========================================================================

/// A resolved view over every entity whose components satisfy `Q`.
///
/// Constructed by [`World::query`](crate::world::World::query) (shared, `Q: ReadOnlyQuery`),
/// [`World::query_mut`](crate::world::World::query_mut) (exclusive) or
/// [`World::query_unchecked`](crate::world::World::query_unchecked) (`unsafe`, for systems that
/// only hold a `&World`). Construction runs [`WorldQuery::check_aliasing`] — which **panics**
/// on a `Q` that would alias, e.g. `(Mut<A>, Mut<A>)` — and resolves the set of matching
/// archetypes once. That set is a snapshot; the query borrows the world for `'w`, so no
/// archetype can appear, vanish or be reordered while it is alive.
///
/// Which accessors exist depends on `Q`. The `&mut self` family (`iter_mut`, `get_mut`,
/// `iter_chunks_mut`, `par_for_each_mut`) is available for every `Q` and ties each result to
/// the exclusive borrow of the query, so two live mutable views cannot be obtained from one
/// query. The `&self` family (`iter`, `get`, `iter_chunks`, `par_for_each`, `contains`,
/// `entities`) additionally requires `Q: ReadOnlyQuery`. Together those two rules are what
/// makes duplicate `&mut T` unreachable without `unsafe`.
///
/// # Iteration order
///
/// The sequential accessors — `iter`, `iter_mut`, `iter_chunks`, `iter_chunks_mut`,
/// `entities` — visit archetypes in ascending archetype index and, within an archetype, rows
/// in ascending row order. That much is *reproducible*: an identical sequence of world
/// operations produces an identical visit order, so repeated runs of the same simulation see
/// rows in the same order.
///
/// `par_for_each`/`par_for_each_mut` promise no order at all. They hand the archetype list, and
/// each archetype's rows, to a work-stealing pool; visit order is whatever the pool decides and
/// varies between runs. Only use them for work whose result does not depend on order.
///
/// Even where the order is defined it is not *meaningful*. It is neither spawn order nor
/// entity-id order, and it is not stable across edits to the world: despawning an entity or
/// adding/removing a component relocates rows by swap-remove, and archetype indices themselves
/// get renumbered when empty archetypes are collected. Never treat the order as a sort.
pub struct Query<'w, Q: WorldQuery + ?Sized> {
    world: &'w World,
    matching_archetypes: Vec<usize>,
    _marker: PhantomData<Q>,
}


// =========================================================================
// ALIASING & IMPLS
// =========================================================================

/// Mutable aliasing check — if there are two mutable accesses to the same `TypeId` it is **UB**.
///
/// # Invariant
/// More than one mutable access (`Mut<T>`) to the same component type within a single query
/// is **strictly forbidden**. A use such as `Query<(Mut<Position>, Mut<Position>)>` panics at
/// run time. This check cannot be done at compile-time because Rust's type system cannot
/// compare `TypeId` equality in a const-context.
///
/// # Safe Usage
/// - `Query<(&Position, Mut<Velocity>)>` → ✅ (different types)
/// - `Query<(Mut<Position>, Mut<Velocity>)>` → ✅ (different types)
/// - `Query<(Mut<Position>, Mut<Position>)>` → ❌ PANIC!
/// - `Query<(&Position, &Position)>` → ✅ (both immutable — aliasing safe)
#[inline]
fn check(tid: TypeId, is_mut: bool, types: &mut Vec<(TypeId, bool)>) {
    for &(existing_tid, existing_mut) in types.iter() {
        if existing_tid == tid && (existing_mut || is_mut) {
            panic!(
                "Query aliasing UB detected! Component TypeId {:?} is accessed mutably more than once \
                 in the same query. This would cause undefined behavior. \
                 Use separate queries for components of the same type that need independent mutable access.",
                tid
            );
        }
    }
    types.push((tid, is_mut));
}

/// Archetype-level match shared by every component-keyed filter. SparseSet storage is
/// stored outside archetypes, so `matches_archetype` is intentionally WIDE there (every
/// archetype; the real per-row test lives in `filter_row`). For Table storage it matches
/// on presence: `want_present` is `true` for `With`/`Changed`/`Added`/`&T`, `false` for
/// `Without`. Centralizing this kills the copy-pasted `if sparse {true} else {has}` that
/// diverged across impls (the round-1/2 sibling-divergence bug class).
#[inline]
fn arch_matches<T: crate::component::Component>(arch: &Archetype, want_present: bool) -> bool {
    if T::storage_type() == crate::component::StorageType::SparseSet {
        true
    } else {
        arch.has_component(TypeId::of::<T>()) == want_present
    }
}

/// Generates the `WorldQuery` impl for a change-detection filter (`Changed`/`Added`).
/// They differ ONLY in which `ComponentTicks` field they read, so they share one body —
/// adding a new tick filter can't forget `check_aliasing` (the data-race guard) or
/// `has_row_filter` (the iter_chunks guard).
macro_rules! impl_tick_filter {
    ($(#[$meta:meta])* $name:ident, $field:ident) => {
        $(#[$meta])*
        pub struct $name<T>(PhantomData<T>);

        impl<T: crate::component::Component> sealed::SealedQuery for $name<T> {}
        // Tick filters carry no data (`Item = ()`) → read-only.
        impl<T: crate::component::Component> sealed::SealedReadOnly for $name<T> {}
        impl<T: crate::component::Component> ReadOnlyQuery for $name<T> {}
        impl<T: crate::component::Component> WorldQuery for $name<T> {
            type StaticType = $name<T>;
            // (table ticks ptr, or the sparse set ptr for SparseSet storage)
            type Fetch<'w> = (
                *const crate::archetype::ComponentTicks,
                Option<*const crate::archetype::sparse_set::ComponentSparseSet>,
            );
            type Item<'w> = ();
            type Slice<'w> = ();

            unsafe fn fetch_raw<'w>(world: &'w World, arch: &Archetype, _tick: u32) -> Option<Self::Fetch<'w>> {
                if T::storage_type() == crate::component::StorageType::SparseSet {
                    let set = world.sparse_sets.get(&TypeId::of::<T>())?;
                    Some((std::ptr::null(), Some(set as *const _)))
                } else {
                    let col = arch.get_column(TypeId::of::<T>())?;
                    Some((col.ticks_ptr(), None))
                }
            }

            fn check_aliasing(types: &mut Vec<(TypeId, bool)>) {
                // Tick filters READ T's ComponentTicks — the same memory `Mut<T>` writes in
                // deref_mut. Declare a READ so the scheduler can't co-batch a `Mut<T>` writer
                // (unsynchronized read+write = data race).
                check(TypeId::of::<T>(), false, types);
            }

            fn matches_archetype(arch: &Archetype) -> bool {
                arch_matches::<T>(arch, true)
            }

            unsafe fn filter_row<'w>(fetch: Self::Fetch<'w>, row: usize, entity_id: u32, tick: u32) -> bool {
                // `tick` = change_ref_tick (last run); rows stamped after it match.
                if let Some(set_ptr) = fetch.1 {
                    (*set_ptr).ticks_for(entity_id).is_some_and(|t| t.$field > tick)
                } else {
                    (*fetch.0.add(row)).$field > tick
                }
            }

            unsafe fn get_item<'w>(_f: Self::Fetch<'w>, _r: usize, _e: u32) -> Self::Item<'w> {}
            unsafe fn get_slice<'w>(_f: Self::Fetch<'w>, _l: usize) -> Self::Slice<'w> {}

            fn has_row_filter() -> bool {
                true // the tick test lives entirely in filter_row
            }
        }
    };
}

/// Generates the `WorldQuery` impl for a presence filter (`With`/`Without`). They differ
/// ONLY by the `$present` polarity, so one body guarantees they stay in lockstep — the
/// sparse per-row check, `matches_archetype`, and `has_row_filter` can't diverge.
macro_rules! impl_presence_filter {
    ($(#[$meta:meta])* $name:ident, $present:expr) => {
        $(#[$meta])*
        pub struct $name<T>(PhantomData<T>);

        impl<T: crate::component::Component> sealed::SealedQuery for $name<T> {}
        // Presence filters carry no data (`Item = ()`) → read-only.
        impl<T: crate::component::Component> sealed::SealedReadOnly for $name<T> {}
        impl<T: crate::component::Component> ReadOnlyQuery for $name<T> {}
        impl<T: crate::component::Component> WorldQuery for $name<T> {
            type StaticType = $name<T>;
            // (is_sparse, sparse set ptr). Table storage is always `(false, None)`.
            type Fetch<'w> = (
                bool,
                Option<*const crate::archetype::sparse_set::ComponentSparseSet>,
            );
            type Item<'w> = ();
            type Slice<'w> = ();

            unsafe fn fetch_raw<'w>(world: &'w World, _arch: &Archetype, _tick: u32) -> Option<Self::Fetch<'w>> {
                if T::storage_type() == crate::component::StorageType::SparseSet {
                    Some((true, world.sparse_sets.get(&TypeId::of::<T>()).map(|s| s as *const _)))
                } else {
                    Some((false, None))
                }
            }

            fn check_aliasing(_types: &mut Vec<(TypeId, bool)>) {}

            fn matches_archetype(arch: &Archetype) -> bool {
                arch_matches::<T>(arch, $present)
            }

            unsafe fn filter_row<'w>(fetch: Self::Fetch<'w>, _row: usize, entity_id: u32, _tick: u32) -> bool {
                // Table: matches_archetype already selected by presence → always true.
                // Sparse: matches_archetype is wide → test actual presence per row.
                match fetch {
                    (false, _) => true,
                    (true, Some(set_ptr)) => (*set_ptr).contains(entity_id) == $present,
                    (true, None) => !$present, // no sparse set yet → nobody has the component
                }
            }

            unsafe fn get_item<'w>(_f: Self::Fetch<'w>, _r: usize, _e: u32) -> Self::Item<'w> {}
            unsafe fn get_slice<'w>(_f: Self::Fetch<'w>, _l: usize) -> Self::Slice<'w> {}

            fn has_row_filter() -> bool {
                // Sparse needs the per-row presence test; table is archetype-level only.
                T::storage_type() == crate::component::StorageType::SparseSet
            }
        }
    };
}

impl<T0: FetchComponent> sealed::SealedQuery for T0 where T0::Component: crate::component::Component {}
impl<T0: FetchComponent> WorldQuery for T0 where T0::Component: crate::component::Component {
    type StaticType = T0::Component;
    type Fetch<'w> = T0::Fetch<'w>;
    type Item<'w> = T0::Item<'w>;
    type Slice<'w> = T0::Slice<'w>;

    unsafe fn fetch_raw<'w>(world: &'w World, arch: &Archetype, tick: u32) -> Option<Self::Fetch<'w>> {
        T0::fetch_raw(world, arch, tick)
    }
    fn check_aliasing(types: &mut Vec<(TypeId, bool)>) {
        check(TypeId::of::<T0::Component>(), T0::IS_MUT, types);
    }
    fn matches_archetype(arch: &Archetype) -> bool {
        arch_matches::<T0::Component>(arch, true)
    }

    unsafe fn get_item<'w>(fetch: Self::Fetch<'w>, row: usize, entity_id: u32) -> Self::Item<'w> {
        T0::get_item(fetch, row, entity_id)
    }

    unsafe fn filter_row<'w>(fetch: Self::Fetch<'w>, _row: usize, entity_id: u32, _tick: u32) -> bool {
        // SparseSet bileşenleri için `matches_archetype` her arketipte `true` döndüğünden
        // satır-başı varlık kontrolü ŞART (yoksa get_item sparse set'i sınır-dışı indeksler).
        // Table depolamada `contains_entity` daima `true`.
        T0::contains_entity(fetch, entity_id)
    }

    unsafe fn get_slice<'w>(fetch: Self::Fetch<'w>, len: usize) -> Self::Slice<'w> {
        T0::get_slice(fetch, len)
    }
}

// `&T` yields shared access only → read-only. `Mut<T>` (also a `FetchComponent`) is
// pointedly excluded: no `SealedReadOnly`/`ReadOnlyQuery` impl exists for it.
impl<T: crate::component::Component> sealed::SealedReadOnly for &T {}
impl<T: crate::component::Component> ReadOnlyQuery for &T {}

impl_tick_filter!(
    /// Filter matching only entities whose `T` changed since the system last ran
    /// (`deref_mut` on `Mut<T>` stamps the change tick). Use as a query operand.
    Changed,
    changed
);

impl_tick_filter!(
    /// Filter matching only entities to which `T` was added since the system last ran.
    Added,
    added
);

macro_rules! impl_query_tuple {
    ($($t:ident),*) => {
        impl<$($t: WorldQuery),*> sealed::SealedQuery for ($($t,)*) {}
        // A tuple is read-only iff EVERY element is read-only.
        impl<$($t: ReadOnlyQuery),*> sealed::SealedReadOnly for ($($t,)*) {}
        impl<$($t: ReadOnlyQuery),*> ReadOnlyQuery for ($($t,)*) {}
        #[allow(non_snake_case)]
        impl<$($t: WorldQuery),*> WorldQuery for ($($t,)*) {
            type StaticType = ($($t::StaticType,)*);
            type Fetch<'w> = ($($t::Fetch<'w>,)*);
            type Item<'w> = ($($t::Item<'w>,)*);
            type Slice<'w> = ($($t::Slice<'w>,)*);

            unsafe fn fetch_raw<'w>(world: &'w World, arch: &Archetype, tick: u32) -> Option<Self::Fetch<'w>> {
                Some(($($t::fetch_raw(world, arch, tick)?,)*))
            }
            fn check_aliasing(types: &mut Vec<(TypeId, bool)>) {
                $($t::check_aliasing(types);)*
            }
            fn matches_archetype(arch: &Archetype) -> bool {
                $($t::matches_archetype(arch) &&)* true
            }
            unsafe fn get_item<'w>(fetch: Self::Fetch<'w>, row: usize, entity_id: u32) -> Self::Item<'w> {
                let ($($t,)*) = fetch;
                ($($t::get_item($t, row, entity_id),)*)
            }
            unsafe fn filter_row<'w>(fetch: Self::Fetch<'w>, row: usize, entity_id: u32, tick: u32) -> bool {
                let ($($t,)*) = fetch;
                $($t::filter_row($t, row, entity_id, tick) &&)* true
            }
            unsafe fn get_slice<'w>(fetch: Self::Fetch<'w>, len: usize) -> Self::Slice<'w> {
                let ($($t,)*) = fetch;
                ($($t::get_slice($t, len),)*)
            }
            fn has_row_filter() -> bool {
                $($t::has_row_filter() ||)* false
            }
        }
    };
}

impl_query_tuple!(T0, T1);
impl_query_tuple!(T0, T1, T2);
impl_query_tuple!(T0, T1, T2, T3);
impl_query_tuple!(T0, T1, T2, T3, T4);
impl_query_tuple!(T0, T1, T2, T3, T4, T5);
impl_query_tuple!(T0, T1, T2, T3, T4, T5, T6);
impl_query_tuple!(T0, T1, T2, T3, T4, T5, T6, T7);
impl_query_tuple!(T0, T1, T2, T3, T4, T5, T6, T7, T8);
impl_query_tuple!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9);
impl_query_tuple!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10);
impl_query_tuple!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11);

// =========================================================================
// ADVANCED QUERY FILTERS
// =========================================================================

impl_presence_filter!(
    /// Filter matching entities that HAVE `T` (without borrowing it). Use as a query operand.
    With,
    true
);

impl_presence_filter!(
    /// Filter matching entities that do NOT have `T`. Use as a query operand.
    Without,
    false
);

/// Filter matching a row accepted by **either** operand. Use as a query operand; like the other
/// filters it yields no data (`Item = ()`).
///
/// An archetype is visited when either operand matches it, but the decision is then repeated
/// per row: an operand may contribute only where its own `matches_archetype` held, and within
/// that, only where its `filter_row` accepts. So `Or<Changed<A>, Changed<B>>` yields the rows
/// where A *or* B actually changed — not every row of every archetype that holds A or B.
///
/// Because the real test is per row, `Or` always reports [`WorldQuery::has_row_filter`], and
/// therefore cannot be used with [`Query::iter_chunks`], which panics on such queries.
///
/// It propagates both operands' component access to [`WorldQuery::check_aliasing`], so a system
/// taking `Or<Changed<A>, Changed<B>>` is correctly recorded as reading both A and B and will
/// not be scheduled in parallel with a writer of either.
///
/// Binary only — nest for more operands: `Or<A, Or<B, C>>`.
pub struct Or<T1, T2>(PhantomData<(T1, T2)>);

impl<T1: WorldQuery, T2: WorldQuery> sealed::SealedQuery for Or<T1, T2> {}
// `Or` is itself a no-data filter; it's read-only when both operands are.
impl<T1: ReadOnlyQuery, T2: ReadOnlyQuery> sealed::SealedReadOnly for Or<T1, T2> {}
impl<T1: ReadOnlyQuery, T2: ReadOnlyQuery> ReadOnlyQuery for Or<T1, T2> {}
impl<T1: WorldQuery, T2: WorldQuery> WorldQuery for Or<T1, T2> {
    type StaticType = Or<T1::StaticType, T2::StaticType>;
    // Each operand's fetch (or `None` when that operand doesn't apply to this archetype).
    // `Or` is a FILTER, so it carries no data — but it must keep the operand fetches so
    // it can evaluate their per-row `filter_row` (the part the old `()` Fetch dropped).
    type Fetch<'w> = (Option<T1::Fetch<'w>>, Option<T2::Fetch<'w>>);
    type Item<'w> = ();
    type Slice<'w> = ();

    unsafe fn fetch_raw<'w>(world: &'w World, arch: &Archetype, tick: u32) -> Option<Self::Fetch<'w>> {
        // Fetch each operand only where it applies; `matches_archetype` gates which
        // operand can contribute, and a `Some` fetch is the per-archetype proof of that.
        let f1 = if T1::matches_archetype(arch) {
            T1::fetch_raw(world, arch, tick)
        } else {
            None
        };
        let f2 = if T2::matches_archetype(arch) {
            T2::fetch_raw(world, arch, tick)
        } else {
            None
        };
        Some((f1, f2))
    }

    fn check_aliasing(types: &mut Vec<(TypeId, bool)>) {
        // Propagate operand access — otherwise `Or<Changed<A>, Changed<B>>` would declare
        // NOTHING and the scheduler could race a `Mut` writer (the round-1 bug class).
        T1::check_aliasing(types);
        T2::check_aliasing(types);
    }

    fn matches_archetype(arch: &Archetype) -> bool {
        T1::matches_archetype(arch) || T2::matches_archetype(arch)
    }

    unsafe fn filter_row<'w>(fetch: Self::Fetch<'w>, row: usize, entity_id: u32, tick: u32) -> bool {
        // A row passes `Or` if EITHER applicable operand accepts it. `matches_archetype`
        // alone is not enough: sparse `With` matches every archetype and Changed/Added do
        // their whole test here, so the per-row `filter_row` MUST be consulted.
        let a = fetch
            .0
            .is_some_and(|f| T1::filter_row(f, row, entity_id, tick));
        let b = fetch
            .1
            .is_some_and(|f| T2::filter_row(f, row, entity_id, tick));
        a || b
    }
    unsafe fn get_item<'w>(_fetch: Self::Fetch<'w>, _row: usize, _entity_id: u32) -> Self::Item<'w> {}
    unsafe fn get_slice<'w>(_fetch: Self::Fetch<'w>, _len: usize) -> Self::Slice<'w> {}

    fn has_row_filter() -> bool {
        true
    }
}


// `impl Query` access/iteration methods live in `access`; the unit tests in `tests`.
mod access;

#[cfg(test)]
mod tests;
