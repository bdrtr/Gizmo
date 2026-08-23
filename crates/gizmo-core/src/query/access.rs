//! `Query` access + iteration methods (mutable and read-only). Extracted verbatim from
//! query/mod.rs (pure move); these are inherent `impl Query` blocks, so they compose back onto
//! the `Query` struct in the parent module. `use super::*` brings in WorldQuery/ReadOnlyQuery,
//! the archetype/fetch machinery and `Mut`.

use super::*;

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
        // iter()/par_inner() yalnız `matching_archetypes`'i (archetype-seviyeli With/Without
        // predicate ile kurulmuş) gezer. get/contains ise entity'nin KENDİ archetype'ını
        // doğrudan indeksler; table-storage With/Without archetype seviyesinde kontrol edilir,
        // filter_row DEĞİL → bu kapı olmadan get()/contains() iter()'in dışladığı entity için
        // Some/true döner (soundness-bitişik tutarsızlık). Aynı archetype kümesine uy.
        if !self
            .matching_archetypes
            .contains(&(loc.archetype_id as usize))
        {
            return None;
        }
        let arch = &self.world.archetype_index.archetypes[loc.archetype_id as usize];
        // SAFETY: the entity's own location names this archetype and it was checked against
        // `matching_archetypes` above, so the fetch is for an archetype this query matches and
        // `loc.row` is a live row in it. The fetch borrows the world for `'w`; aliasing was
        // settled at construction by `check_aliasing`.
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
        // SAFETY: the wrapper exists to carry a fetch (a bundle of raw pointers) into rayon's
        // closures, which demand `Send`/`Sync`. Sending it is sound because the pointers address
        // component storage whose values are `Send + Sync` by the `Component` bound, and because
        // `Query::new`'s `check_aliasing` already established that this query's access set does
        // not overlap itself — the parallel rows below are disjoint by construction (one row
        // each), so no two threads touch the same element.
        unsafe impl<T> Send for FetchWrapper<T> {}
        // SAFETY: as above — sharing the wrapper only ever hands out a copy of the same pointers.
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
            // SAFETY: `arch_idx` is from `matching_archetypes` and the archetype is borrowed from
            // the world for the whole parallel section, so the fetch outlives every task.
            if let Some(fetch) = unsafe { Q::fetch_raw(self.world, arch, tick) } {
                let len = arch.len();
                let wrapped_fetch = FetchWrapper(fetch);
                let entities_ptr = FetchWrapper(arch.entities().as_ptr());
                let func_ref = &func;

                // Her Archetype'ı cache dostu chunk'lar halinde ayırıp process ediyoruz
                // Chunk size: 512 — bir arketip satırı için önbellek dostu bir dilim;
                // rayon'un görev başına yükünü amorti edecek kadar büyük, L1'i taşırmayacak kadar küçük.
                (0..len)
                    .into_par_iter()
                    .with_min_len(512)
                    // SAFETY: `row < len == arch.len()`, so each task addresses a live row of
                    // this archetype, and every task gets a DIFFERENT row — that disjointness is
                    // what makes the shared fetch sound here.
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

    /// Mutable iteration yielding a per-element `Mut<T>`. Because it takes `&mut self`, a second
    /// live mutable iteration over the same query is blocked at compile time.
    pub fn iter_mut<'a>(&'a mut self) -> QueryIter<'a, 'w, Q> {
        self.iter_inner()
    }

    /// Mutable chunk iteration for **bulk writes** (returns `&mut [T]`).
    ///
    /// Because it hands out a raw slice it cannot track which elements were written; therefore it
    /// **conservatively marks all the rows it hands out as "changed".**
    /// This never MISSES a real change (the safe side for change detection),
    /// but if you write only some of them it shows the unwritten ones as "changed" too
    /// (false positive). Choose the right tool:
    /// - If you will only read → [`Query::iter_chunks`] (does not mark).
    /// - If you will write some of them with precise marking → `iter_mut` (per-element `Mut`).
    /// - If you will write all of them → this method (marking all of them is already correct).
    pub fn iter_chunks_mut<'a>(&'a mut self) -> QueryChunksIter<'a, 'w, Q> {
        self.iter_chunks_inner()
    }

    /// Mutable access by raw `u32` id — does not check the generation (see [`Query::get`]).
    /// Because it takes `&mut self` the returned `Mut` borrows the query exclusively; a second
    /// simultaneous `get_mut`/`iter_mut` does not compile.
    #[inline]
    pub fn get_mut(&mut self, entity_id: u32) -> Option<Q::Item<'_>> {
        self.get_inner(entity_id)
    }

    /// Generation-validated mutable access (see [`Query::get_entity`]).
    #[inline]
    pub fn get_mut_entity(&mut self, entity: Entity) -> Option<Q::Item<'_>> {
        if !self.world.is_alive(entity) {
            return None;
        }
        self.get_inner(entity.id())
    }

    /// Lock-free parallel mutable iteration running on the thread pool (Work-Stealing).
    pub fn par_for_each_mut<F>(&mut self, func: F)
    where
        F: Fn((u32, Q::Item<'_>)) + Send + Sync,
    {
        self.par_inner(func);
    }

    // ── Metadata (no component access → always `&self`) ───────────────────

    /// Total rows held by the matching archetypes — an **upper bound** on what iteration
    /// yields, not the number of items it will produce.
    ///
    /// It sums archetype lengths and never runs a per-row filter, so it is exact only for
    /// queries whose entire test is archetype-level: Table-stored `&T`/`Mut<T>`/`With`/`Without`
    /// and tuples of those. It over-counts for `Changed`/`Added`/`Or`, and for `SparseSet`-stored
    /// operands, which narrow nothing here at all — they match *every* archetype and do their
    /// real work per row. A tuple still ANDs its operands' archetype tests, so one Table-stored
    /// operand is enough to keep the count sane; a query whose operands are *all* sparse matches
    /// every archetype and counts every row in the world.
    ///
    /// Cost is O(matching archetypes); rows are not walked.
    #[inline]
    pub fn entity_count(&self) -> usize {
        self.matching_archetypes
            .iter()
            .map(|&idx| self.world.archetype_index.archetypes[idx].len())
            .sum()
    }

    /// Alias of [`Query::entity_count`], carrying the same caveat: it counts *unfiltered* rows,
    /// so `len()` may exceed the number of items `iter()`/`iter_mut()` actually yields. It is
    /// not the length of any slice or collection.
    #[inline]
    pub fn len(&self) -> usize {
        self.entity_count()
    }

    /// `true` when no archetype matched, or when every matching archetype is empty.
    ///
    /// Derived from [`Query::entity_count`], so it inherits its blind spot in one direction
    /// only: `is_empty() == true` does guarantee that iteration yields nothing, but
    /// `is_empty() == false` does **not** guarantee it yields something — a per-row filter
    /// (`Changed`/`Added`/`Or`, or a sparse operand) can still reject every row. If you need
    /// the truth, iterate.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entity_count() == 0
    }
}

// ── READ-ONLY accessors (only for queries that never yield `&mut T`) ──────
// Sound from a shared `&self` because `Q: ReadOnlyQuery` guarantees `Q::Item` is a
// shared borrow — any number may coexist.
impl<'w, Q: ReadOnlyQuery> Query<'w, Q> {
    /// Shared row-by-row iteration, yielding `(entity_id, item)` and skipping rows rejected by
    /// per-row filters.
    ///
    /// Takes `&self`, so several of these may be alive over the same query at once — sound only
    /// because `Q: ReadOnlyQuery` guarantees no item can be a `&mut T`. For a mutable `Q` use
    /// [`Query::iter_mut`], which ties the iterator to an exclusive borrow instead.
    ///
    /// See [`QueryIter`] for the visit order and for what the yielded `u32` id does and does not
    /// guarantee.
    pub fn iter<'a>(&'a self) -> QueryIter<'a, 'w, Q> {
        self.iter_inner()
    }

    /// Read-only SIMD-friendly chunk iteration (returns `&[T]`). It does NOT AFFECT change
    /// detection — use it for reading components.
    ///
    /// # Panics
    /// Panics on a query that REQUIRES a per-row filter (SparseSet `With`/`Without`,
    /// `Changed`/`Added`, `Or`): chunk iteration returns the archetype's ENTIRE contiguous slice,
    /// whereas those filters select per row (see [`WorldQuery::has_row_filter`]).
    /// Rather than silently returning an unfiltered result it refuses loudly — use
    /// [`Query::iter`]/[`Query::iter_mut`] instead. (Table `With`/`Without` is safe.)
    pub fn iter_chunks<'a>(&'a self) -> QueryChunksIter<'a, 'w, Q> {
        self.iter_chunks_inner()
    }

    /// Access by raw `u32` id. **CAUTION: it does NOT check the generation.** If an id is given
    /// that was despawned and whose slot has been reused, the data of the NEW entity in that
    /// slot is returned (a silent use-after-free-like bug). If you hold an [`Entity`] handle,
    /// use [`Query::get_entity`] — that one validates the generation.
    #[inline]
    pub fn get(&self, entity_id: u32) -> Option<Q::Item<'_>> {
        self.get_inner(entity_id)
    }

    /// Generation-validated access: returns `None` if `entity` is no longer alive (despawned or
    /// its slot handed to another entity). Prevents reading the wrong entity's data through a
    /// stale handle. If you hold an [`Entity`] handle, prefer this one.
    #[inline]
    pub fn get_entity(&self, entity: Entity) -> Option<Q::Item<'_>> {
        if !self.world.is_alive(entity) {
            return None;
        }
        self.get_inner(entity.id())
    }

    /// Checks whether a given entity belongs to this query.
    #[inline]
    pub fn contains(&self, entity_id: u32) -> bool {
        self.get_inner(entity_id).is_some()
    }

    /// The matching entity ids alone, in the same order and with the same per-row filtering as
    /// [`Query::iter`] — it *is* `iter()` with the item discarded, so a row rejected by
    /// `Changed`/`Added`/`Or` or by a sparse presence test does not appear here either. This
    /// makes it a truer count than [`Query::len`], at the cost of walking every row.
    ///
    /// Ids are raw `u32` indices without a generation counter; they say nothing about liveness
    /// once the query is dropped.
    pub fn entities<'a>(&'a self) -> impl Iterator<Item = u32> + 'a {
        self.iter_inner().map(|(id, _)| id)
    }

    /// Lock-free parallel iteration running on the thread pool (Work-Stealing)
    pub fn par_for_each<F>(&self, func: F)
    where
        F: Fn((u32, Q::Item<'_>)) + Send + Sync,
    {
        self.par_inner(func);
    }
}
