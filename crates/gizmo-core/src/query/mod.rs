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

impl<'w, Q: WorldQuery> Query<'w, Q> {
    pub(crate) fn new(world: &'w World) -> Option<Self> {
        let mut used_types = Vec::new();
        Q::check_aliasing(&mut used_types);
        let matching = world
            .archetype_index
            .matching_archetypes_readonly(Q::matches_archetype);
        Some(Self {
            world,
            matching_archetypes: matching,
            _marker: PhantomData,
        })
    }

    pub(crate) fn new_cached(world: &'w mut World) -> Option<Self> {
        let mut used_types = Vec::new();
        Q::check_aliasing(&mut used_types);
        let matching = world
            .archetype_index
            .matching_archetypes(TypeId::of::<Q::StaticType>(), Q::matches_archetype)
            .to_vec();
        Some(Self {
            world,
            matching_archetypes: matching,
            _marker: PhantomData,
        })
    }

    // ── PRIVATE primitives ────────────────────────────────────────────────
    // The actual fetch logic, callable from `&self`. The PUBLIC `&self` wrappers
    // bound `Q: ReadOnlyQuery` (so a mutable `Q` can never yield `&mut T` from a
    // shared borrow), while the `&mut self` wrappers tie the returned items to the
    // exclusive borrow (so two live `&mut T` from one query are impossible). Keeping
    // these private is what makes the gating airtight.

    fn iter_inner<'a>(&'a self) -> QueryIter<'a, 'w, Q> {
        QueryIter {
            world: self.world,
            archetype_indices: &self.matching_archetypes,
            current_arch_idx: 0,
            current_row: 0,
            current_fetch: None,
            _marker: PhantomData,
            _marker_w: PhantomData,
        }
    }

    fn iter_chunks_inner<'a>(&'a self) -> QueryChunksIter<'a, 'w, Q> {
        assert!(
            !Q::has_row_filter(),
            "iter_chunks does not support per-row-filtered queries \
             (sparse With/Without, Changed, Added, Or) — they need per-row narrowing that \
             a contiguous chunk cannot express; use iter()/iter_mut() instead"
        );
        QueryChunksIter {
            world: self.world,
            archetype_indices: &self.matching_archetypes,
            current_arch_idx: 0,
            _marker: PhantomData,
        }
    }

    #[inline]
    fn get_inner<'a>(&'a self, entity_id: u32) -> Option<Q::Item<'a>> {
        let loc = self.world.entity_location(entity_id);
        if !loc.is_valid() {
            return None;
        }
        let arch = &self.world.archetype_index.archetypes[loc.archetype_id as usize];
        unsafe {
            let fetch = Q::fetch_raw(self.world, arch, self.world.tick)?;
            if !Q::filter_row(fetch, loc.row as usize, entity_id, self.world.change_ref_tick) {
                return None;
            }
            Some(Q::get_item(fetch, loc.row as usize, entity_id))
        }
    }

    fn par_inner<F>(&self, func: F)
    where
        F: Fn((u32, Q::Item<'_>)) + Send + Sync,
    {
        #[cfg(not(target_arch = "wasm32"))]
        use rayon::prelude::*;
        #[cfg(target_arch = "wasm32")]
        use crate::parallel_compat::*;

        // Pointer taşıyıcı wrapper — Güvenlidir çünkü Query::new() check_aliasing yapmıştır
        #[derive(Copy, Clone)]
        struct FetchWrapper<T>(T);
        unsafe impl<T> Send for FetchWrapper<T> {}
        unsafe impl<T> Sync for FetchWrapper<T> {}

        impl<T: Copy> FetchWrapper<T> {
            fn get(&self) -> T {
                self.0
            }
        }

        let tick = self.world.tick;
        let ref_tick = self.world.change_ref_tick;
        self.matching_archetypes.par_iter().for_each(|&arch_idx| {
            let arch = &self.world.archetype_index.archetypes[arch_idx];
            if let Some(fetch) = unsafe { Q::fetch_raw(self.world, arch, tick) } {
                let len = arch.len();
                let wrapped_fetch = FetchWrapper(fetch);
                let entities_ptr = FetchWrapper(arch.entities().as_ptr());
                let func_ref = &func;

                // Her Archetype'ı cache dostu chunk'lar halinde ayırıp process ediyoruz
                // Chunk size: 512 (Bevy benzeri)
                (0..len)
                    .into_par_iter()
                    .with_min_len(512)
                    .for_each(move |row| unsafe {
                        let id = *entities_ptr.get().add(row);
                        if Q::filter_row(wrapped_fetch.get(), row, id, ref_tick) {
                            let item = Q::get_item(wrapped_fetch.get(), row, id);
                            func_ref((id, item));
                        }
                    });
            }
        });
    }

    // ── MUTABLE accessors (available for every `Q`) ───────────────────────
    // Each ties its result to the EXCLUSIVE `&mut self` borrow, so two live mutable
    // views from one query can't coexist. Combined with `query_mut`/`query_unchecked`
    // gating creation, this closes the dual-`Mut` aliasing hole for safe code.

    /// Eleman-başına `Mut<T>` veren mutable iterasyon. `&mut self` aldığından aynı query
    /// üzerinde ikinci bir canlı mutable iterasyon derleme zamanında engellenir.
    pub fn iter_mut<'a>(&'a mut self) -> QueryIter<'a, 'w, Q> {
        self.iter_inner()
    }

    /// **Toplu (bulk) yazma** için mutable chunk iterasyonu (`&mut [T]` döndürür).
    ///
    /// Ham bir dilim verdiği için hangi elemanların yazıldığını izleyemez; bu yüzden
    /// **verilen tüm satırları temkinli (conservative) olarak "changed" işaretler.**
    /// Bu, gerçek bir değişikliği asla KAÇIRMAZ (change detection için güvenli taraf),
    /// ama yalnızca bir kısmını yazarsanız yazılmayanları da "changed" gösterir
    /// (false positive). Doğru aracı seçin:
    /// - Sadece okuyacaksanız → [`Query::iter_chunks`] (işaretlemez).
    /// - Bir kısmını hassas işaretleyerek yazacaksanız → `iter_mut` (eleman başına `Mut`).
    /// - Hepsini yazacaksanız → bu metot (hepsini işaretlemek zaten doğru).
    pub fn iter_chunks_mut<'a>(&'a mut self) -> QueryChunksIter<'a, 'w, Q> {
        self.iter_chunks_inner()
    }

    /// Ham `u32` id ile mutable erişim — generation kontrolü yapmaz (bkz. [`Query::get`]).
    /// `&mut self` aldığından dönen `Mut` query'yi özel olarak ödünç alır; aynı anda ikinci
    /// bir `get_mut`/`iter_mut` derlenmez.
    #[inline]
    pub fn get_mut(&mut self, entity_id: u32) -> Option<Q::Item<'_>> {
        self.get_inner(entity_id)
    }

    /// Generation-doğrulamalı mutable erişim (bkz. [`Query::get_entity`]).
    #[inline]
    pub fn get_mut_entity(&mut self, entity: Entity) -> Option<Q::Item<'_>> {
        if !self.world.is_alive(entity) {
            return None;
        }
        self.get_inner(entity.id())
    }

    /// İş parçacığı havuzu (Work-Stealing) ile çalışan lock-free paralel mutable iterasyon.
    pub fn par_for_each_mut<F>(&mut self, func: F)
    where
        F: Fn((u32, Q::Item<'_>)) + Send + Sync,
    {
        self.par_inner(func);
    }

    // ── Metadata (no component access → always `&self`) ───────────────────

    #[inline]
    pub fn entity_count(&self) -> usize {
        self.matching_archetypes
            .iter()
            .map(|&idx| self.world.archetype_index.archetypes[idx].len())
            .sum()
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.entity_count()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entity_count() == 0
    }
}

// ── READ-ONLY accessors (only for queries that never yield `&mut T`) ──────
// Sound from a shared `&self` because `Q: ReadOnlyQuery` guarantees `Q::Item` is a
// shared borrow — any number may coexist.
impl<'w, Q: ReadOnlyQuery> Query<'w, Q> {
    pub fn iter<'a>(&'a self) -> QueryIter<'a, 'w, Q> {
        self.iter_inner()
    }

    /// Salt-okunur SIMD-dostu chunk iterasyonu (`&[T]` döndürür). Değişiklik tespitini
    /// (change detection) ETKİLEMEZ — bileşenleri okumak için kullanın.
    ///
    /// # Panics
    /// Satır-başı filtre GEREKTİREN bir query'de (SparseSet `With`/`Without`,
    /// `Changed`/`Added`, `Or`) panikler: chunk iterasyonu arketipin TÜM bitişik dilimini
    /// döndürür, bu filtreler ise satır-başı seçer (bkz. [`WorldQuery::has_row_filter`]).
    /// Sessizce filtrelenmemiş sonuç döndürmek yerine yüksek sesle reddeder — bunun yerine
    /// [`Query::iter`]/[`Query::iter_mut`] kullanın. (Tablo `With`/`Without` güvenlidir.)
    pub fn iter_chunks<'a>(&'a self) -> QueryChunksIter<'a, 'w, Q> {
        self.iter_chunks_inner()
    }

    /// Ham `u32` id ile erişim. **DİKKAT: generation kontrolü YAPMAZ.** Despawn edilip
    /// slotu yeniden kullanılan bir id verilirse, o slottaki YENİ entity'nin verisi
    /// döner (use-after-free benzeri sessiz hata). Elinizde bir [`Entity`] handle'ı varsa
    /// [`Query::get_entity`] kullanın — o, generation'ı doğrular.
    #[inline]
    pub fn get(&self, entity_id: u32) -> Option<Q::Item<'_>> {
        self.get_inner(entity_id)
    }

    /// Generation-doğrulamalı erişim: `entity` artık canlı değilse (despawn edilmiş veya
    /// slotu başka bir entity'ye verilmiş) `None` döner. Stale-handle ile yanlış entity'nin
    /// verisini okumayı engeller. Elinizde bir [`Entity`] handle'ı varsa bunu tercih edin.
    #[inline]
    pub fn get_entity(&self, entity: Entity) -> Option<Q::Item<'_>> {
        if !self.world.is_alive(entity) {
            return None;
        }
        self.get_inner(entity.id())
    }

    /// Belirli bir entity'nin bu query'ye ait olup olmadığını kontrol eder.
    #[inline]
    pub fn contains(&self, entity_id: u32) -> bool {
        self.get_inner(entity_id).is_some()
    }

    pub fn entities<'a>(&'a self) -> impl Iterator<Item = u32> + 'a {
        self.iter_inner().map(|(id, _)| id)
    }

    /// İş parçacığı havuzu (Work-Stealing) ile çalışan lock-free paralel iterasyon
    pub fn par_for_each<F>(&self, func: F)
    where
        F: Fn((u32, Q::Item<'_>)) + Send + Sync,
    {
        self.par_inner(func);
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::impl_component;

    #[derive(Debug, Clone, PartialEq)]
    struct Position {
        x: f32,
        y: f32,
    }
    impl_component!(Position);

    #[derive(Debug, Clone, PartialEq)]
    struct Velocity {
        x: f32,
        y: f32,
    }
    impl_component!(Velocity);

    /// `Query<(Mut<Position>, Mut<Position>)>` gibi aynı tipe çift mutable erişim
    /// denemesi panic ile engellenmeli.
    #[test]
    #[should_panic(expected = "Query aliasing UB detected")]
    fn test_same_type_mut_mut_panics() {
        let mut types = Vec::new();
        // İlk Mut<Position> — sorunsuz eklenir
        check(TypeId::of::<Position>(), true, &mut types);
        // İkinci Mut<Position> — PANIC olmalı!
        check(TypeId::of::<Position>(), true, &mut types);
    }

    /// `Query<(&Position, Mut<Position>)>` — bir immutable, bir mutable aynı tipe erişim:
    /// Bu da panic olmalı çünkü &T + &mut T alias oluşturur.
    #[test]
    #[should_panic(expected = "Query aliasing UB detected")]
    fn test_same_type_ref_mut_panics() {
        let mut types = Vec::new();
        check(TypeId::of::<Position>(), false, &mut types); // &Position
        check(TypeId::of::<Position>(), true, &mut types); // Mut<Position> — PANIC!
    }

    /// `Query<(Mut<Position>, Mut<Velocity>)>` — farklı tipler, sorunsuz çalışmalı.
    #[test]
    fn test_different_types_mut_mut_ok() {
        let mut types = Vec::new();
        check(TypeId::of::<Position>(), true, &mut types);
        check(TypeId::of::<Velocity>(), true, &mut types);
        assert_eq!(types.len(), 2);
    }

    /// `Query<(&Position, &Position)>` — aynı tipe çift immutable erişim güvenlidir.
    #[test]
    fn test_same_type_ref_ref_ok() {
        let mut types = Vec::new();
        check(TypeId::of::<Position>(), false, &mut types);
        check(TypeId::of::<Position>(), false, &mut types);
        assert_eq!(types.len(), 2);
    }

    /// World üzerinden Query oluşturulduğunda aliasing kontrolünün çalıştığını doğrular.
    #[test]
    fn test_query_new_with_valid_types() {
        let mut world = crate::World::new();
        world.register_component_type::<Position>();
        world.register_component_type::<Velocity>();
        let e = world.spawn();
        world.add_component(e, Position { x: 1.0, y: 2.0 });
        world.add_component(e, Velocity { x: 0.0, y: 0.0 });

        // Farklı tipler — Query oluşturulabilmeli
        let q = world.query_mut::<(Mut<Position>, Mut<Velocity>)>();
        assert!(q.is_some());
    }

    /// `Changed<T>`/`Added<T>` artık referans tick'e (son çalıştırma) göre çalışır,
    /// `== current_tick` değil. Kareler arası doğru raporlama doğrulanır.
    #[test]
    fn change_detection_is_relative_to_ref_tick() {
        let mut world = crate::World::new();
        world.register_component_type::<Position>();
        let e = world.spawn();
        world.add_component(e, Position { x: 1.0, y: 2.0 });

        // Frame 1: ref=0 → ilk gözlem eklenen bileşeni görür.
        world.begin_change_frame(0);
        assert_eq!(world.query::<Changed<Position>>().unwrap().iter().count(), 1);
        assert_eq!(world.query::<Added<Position>>().unwrap().iter().count(), 1);

        // Frame 2: değişiklik yok → Changed boş olmalı.
        let prev = world.tick;
        world.begin_change_frame(prev);
        assert_eq!(
            world.query::<Changed<Position>>().unwrap().iter().count(),
            0,
            "değişiklik olmayan frame'de Changed boş olmalı (eski `==` davranışı her şeyi eşliyordu)"
        );

        // Frame 2 içinde mutasyon → Changed yeniden 1 olmalı.
        {
            let mut q = world.query_mut::<Mut<Position>>().unwrap();
            for (_id, mut p) in q.iter_mut() {
                p.x += 1.0;
            }
        }
        assert_eq!(world.query::<Changed<Position>>().unwrap().iter().count(), 1);
    }

    /// `get_entity` generation'ı doğrular: despawn edilip slotu yeniden kullanılan bir
    /// entity'nin eski handle'ı `None` döner; ham `get(id)` ise (footgun) yeni entity'nin
    /// verisini döndürür.
    #[test]
    fn get_entity_rejects_stale_handle_after_despawn_reuse() {
        let mut world = crate::World::new();
        world.register_component_type::<Position>();

        let e1 = world.spawn();
        world.add_component(e1, Position { x: 1.0, y: 1.0 });
        let stale = e1;

        world.despawn(e1);

        // Slotu yeniden kullan — aynı id, artmış generation.
        let e2 = world.spawn();
        world.add_component(e2, Position { x: 2.0, y: 2.0 });
        assert_eq!(e2.id(), stale.id(), "slot yeniden kullanılmalı (aynı id)");
        assert_ne!(e2.generation(), stale.generation(), "generation artmalı");

        let q = world.query::<&Position>().unwrap();
        // Ham id: generation kontrolü yok → yeni entity'nin verisi (footgun).
        assert_eq!(q.get(stale.id()).map(|p| p.x), Some(2.0));
        // Generation-doğrulamalı: stale handle reddedilir.
        assert!(q.get_entity(stale).is_none(), "stale handle None dönmeli");
        // Geçerli handle çalışır.
        assert_eq!(q.get_entity(e2).map(|p| p.x), Some(2.0));
    }

    /// `iter_chunks_mut` ile yapılan toplu yazma, değişiklik tespitini tetiklemeli
    /// (temkinli işaretleme → gerçek yazmayı asla kaçırmaz, false negative yok).
    #[test]
    fn iter_chunks_mut_triggers_change_detection() {
        let mut world = crate::World::new();
        world.register_component_type::<Position>();
        let e = world.spawn();
        world.add_component(e, Position { x: 1.0, y: 1.0 });

        // Referansı bu tick'e ayarla ve frame'i ilerlet (Schedule'ın yaptığı gibi).
        world.begin_change_frame(world.tick);
        // Yazmadan önce: değişiklik yok.
        assert_eq!(world.query::<Changed<Position>>().unwrap().iter().count(), 0);

        // Chunked mutable yazma.
        {
            let mut q = world.query_mut::<Mut<Position>>().unwrap();
            for (_ids, slice) in q.iter_chunks_mut() {
                for p in slice.iter_mut() {
                    p.x += 10.0;
                }
            }
        }

        // Yazmadan sonra: Changed tetiklenmeli ve değer güncellenmeli.
        assert_eq!(world.query::<Changed<Position>>().unwrap().iter().count(), 1);
        assert_eq!(world.query::<&Position>().unwrap().get(e.id()).map(|p| p.x), Some(11.0));
    }

    /// SparseSet bileşenlerinde `Changed`/`Added` artık gerçek tick takibi yapar
    /// (eskiden her zaman `true` idi). Tablo bileşenleriyle aynı kareler-arası semantik.
    #[test]
    fn sparse_set_change_detection_tracks_ticks() {
        #[derive(Clone, Debug, PartialEq)]
        struct SparseComp(i32);
        impl crate::component::Component for SparseComp {
            fn storage_type() -> crate::component::StorageType {
                crate::component::StorageType::SparseSet
            }
        }

        let mut world = crate::World::new();
        world.register_component_type::<SparseComp>();
        let e = world.spawn();
        world.add_component(e, SparseComp(1));

        // Frame 1: ref=0 → eklenen bileşen Added ve Changed olarak görülmeli.
        world.begin_change_frame(0);
        assert_eq!(world.query::<Added<SparseComp>>().unwrap().iter().count(), 1);
        assert_eq!(world.query::<Changed<SparseComp>>().unwrap().iter().count(), 1);

        // Frame 2: değişiklik yok → ikisi de boş (eski davranış burada hep 1 verirdi).
        let prev = world.tick;
        world.begin_change_frame(prev);
        assert_eq!(world.query::<Changed<SparseComp>>().unwrap().iter().count(), 0);
        assert_eq!(world.query::<Added<SparseComp>>().unwrap().iter().count(), 0);

        // Frame 2 içinde mutasyon → Changed yeniden tetiklenmeli.
        {
            let mut q = world.query_mut::<Mut<SparseComp>>().unwrap();
            for (_id, mut c) in q.iter_mut() {
                c.0 += 10;
            }
        }
        assert_eq!(world.query::<Changed<SparseComp>>().unwrap().iter().count(), 1);
        assert_eq!(world.query::<&SparseComp>().unwrap().get(e.id()).map(|c| c.0), Some(11));
    }
}
