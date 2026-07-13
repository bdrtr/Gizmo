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

pub trait WorldQuery: sealed::SealedQuery {
    type StaticType: 'static;
    type Fetch<'w>: Copy;
    type Item<'w>;
    type Slice<'w>;

    /// # Safety
    /// Archetype geçerli olmalı ve döndürülen fetch pointer'ı archetype'ın yaşam süresi boyunca geçerli kalmalıdır.
    unsafe fn fetch_raw<'w>(world: &'w World, arch: &Archetype, system_tick: u32) -> Option<Self::Fetch<'w>>;
    fn check_aliasing(types: &mut Vec<(TypeId, bool)>);
    fn matches_archetype(arch: &Archetype) -> bool;

    /// # Safety
    /// `row` değeri archetype'ın eleman sayısından küçük olmalıdır.
    unsafe fn get_item<'w>(fetch: Self::Fetch<'w>, row: usize, entity_id: u32) -> Self::Item<'w>;

    /// # Safety
    /// Geçerli bir fetch ve archetype sınırları içinde bir `row` sağlanmalıdır.
    unsafe fn filter_row<'w>(fetch: Self::Fetch<'w>, row: usize, entity_id: u32, system_tick: u32) -> bool;

    /// # Safety
    /// `len` değeri archetype'ın eleman sayısını aşmamalıdır.
    unsafe fn get_slice<'w>(fetch: Self::Fetch<'w>, len: usize) -> Self::Slice<'w>;

    /// Bu query satır-başı (`filter_row`) daraltma GEREKTİRİYOR mu — yani
    /// `matches_archetype` bilinçli olarak GENİŞ mi ve gerçek test `filter_row`'da mı?
    /// SparseSet `With`/`Without` (matches her arketipte true) ile `Changed`/`Added`/`Or`
    /// (doğası gereği satır-başı) için `true`. `iter_chunks` arketipin TÜM bitişik
    /// dilimini döndürdüğünden bu filtreleri ONURLANDIRAMAZ → bu tür query'leri reddeder
    /// (bkz. [`Query::iter_chunks`]). Tablo `With`/`Without` için `false` (matches_archetype
    /// yeterli) → onlarla chunk iterasyonu güvenli.
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
pub trait ReadOnlyQuery: WorldQuery + sealed::SealedReadOnly {}

// =========================================================================
// QUERY STRUCT
// =========================================================================

pub struct Query<'w, Q: WorldQuery + ?Sized> {
    world: &'w World,
    matching_archetypes: Vec<usize>,
    _marker: PhantomData<Q>,
}


// =========================================================================
// ALIASING & IMPLS
// =========================================================================

/// Mutable aliasing kontrolü — aynı `TypeId`'ye iki mutable erişim varsa **UB** olur.
///
/// # Invariant
/// Bir query içinde aynı component tipine birden fazla mutable erişim (`Mut<T>`)
/// **kesinlikle yasaktır**. `Query<(Mut<Position>, Mut<Position>)>` gibi bir kullanım
/// çalışma zamanında panic atar. Bu kontrol compile-time'da yapılamaz çünkü Rust'ın
/// tip sistemi `TypeId` eşitliğini const-context'te karşılaştıramaz.
///
/// # Güvenli Kullanım
/// - `Query<(&Position, Mut<Velocity>)>` → ✅ (farklı tipler)
/// - `Query<(Mut<Position>, Mut<Velocity>)>` → ✅ (farklı tipler)
/// - `Query<(Mut<Position>, Mut<Position>)>` → ❌ PANIC!
/// - `Query<(&Position, &Position)>` → ✅ (ikisi de immutable — aliasing güvenli)
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
