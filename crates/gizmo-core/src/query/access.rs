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

    /// Every unordered pair of matching rows, **both halves mutable**.
    ///
    /// The writing counterpart of [`iter_combinations`](Self::iter_combinations). An n-body force
    /// loop applies equal and opposite impulses to `a` and `b` in one visit; without this it reads
    /// pairs, buffers accelerations into a `Vec`, and writes them back in a second pass.
    ///
    /// ```
    /// # use gizmo_core::prelude::*;
    /// # use gizmo_core::query::Mut;
    /// # #[derive(Clone, Debug)] struct Charge(f32);
    /// # gizmo_core::impl_component!(Charge);
    /// # let mut world = World::new();
    /// # for _ in 0..3 { let e = world.spawn(); world.add_component(e, Charge(1.0)); }
    /// let mut q = world.query_mut::<Mut<Charge>>().unwrap();
    /// for ((_, mut a), (_, mut b)) in q.iter_combinations_mut() {
    ///     // Equal and opposite: what a two-pass version has to reconstruct.
    ///     a.0 += 1.0;
    ///     b.0 -= 1.0;
    /// }
    /// ```
    ///
    /// # Why this needs `unsafe`, and what makes it sound
    ///
    /// Both halves of a pair are `&mut` into the same component storage at the same time. That is
    /// sound **because `i != j`** — the two halves are always different rows — and unsound the
    /// moment they are not. The borrow checker cannot see that, so the invariant is carried by the
    /// iterator's own structure rather than by a type: `j` starts at `i + 1` and only ever runs
    /// ahead, and the ids come from one `iter()` scan, which yields each matching row exactly once.
    ///
    /// The `&mut self` borrow is what keeps that invariant meaningful from the outside: no second
    /// view of this query can exist while the iterator does, so nothing else can be looking at
    /// either row.
    ///
    /// # Cost
    ///
    /// The same as the read-only version: one `Vec<u32>` of ids up front, then `n(n-1)/2` pairs of
    /// lookups.
    pub fn iter_combinations_mut<'a>(&'a mut self) -> QueryCombinationsMut<'a, 'w, Q> {
        let ids: Vec<u32> = self.iter_mut().map(|(id, _)| id).collect();
        QueryCombinationsMut {
            query: self as *mut Self,
            ids,
            i: 0,
            j: 1,
            _marker: PhantomData,
        }
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

    /// Every unordered **pair** of matching rows, each yielded once.
    ///
    /// `[a, b, c]` gives `(a,b)`, `(a,c)`, `(b,c)` — never `(a,a)`, and never both `(a,b)` and
    /// `(b,a)`. That is the property an n-body force loop or a broad-phase collision check needs,
    /// and getting it wrong is not loud: a pair counted twice, or an entity paired with itself,
    /// produces motion that still looks plausible.
    ///
    /// ```
    /// # use gizmo_core::prelude::*;
    /// # #[derive(Clone, Debug)] struct Mass(f32);
    /// # gizmo_core::impl_component!(Mass);
    /// # let mut world = World::new();
    /// # for m in [1.0, 2.0, 3.0] { let e = world.spawn(); world.add_component(e, Mass(m)); }
    /// let q = world.query::<&Mass>().unwrap();
    /// let pairs: Vec<_> = q.iter_combinations().map(|((_, a), (_, b))| (a.0, b.0)).collect();
    /// assert_eq!(pairs.len(), 3); // 3 choose 2
    /// ```
    ///
    /// # Read-only, and why there is no `_mut`
    ///
    /// This takes `&self` and requires `Q: ReadOnlyQuery`, so both halves of a pair are shared
    /// borrows and any number may coexist. A mutable version would have to hand out two `&mut`
    /// into the same storage at once; that is sound only because `i != j`, which the borrow
    /// checker cannot see, so it would need `unsafe`. Until that is written and justified, a loop
    /// that *writes* still reads pairs here and applies the results in a second pass — which is
    /// what `demo/src/bin/iter_combinations.rs` does and measures.
    ///
    /// # Cost
    ///
    /// One `Vec<u32>` of matching ids up front, then `n(n-1)/2` pairs of [`get`](Self::get)
    /// lookups. The ids are collected once rather than re-scanned per pair, and they are captured
    /// before the first pair is yielded — so a structural change during iteration is impossible
    /// anyway (the query holds a borrow), and the visit order is the one [`QueryIter`] documents.
    pub fn iter_combinations<'a>(&'a self) -> QueryCombinations<'a, 'w, Q> {
        let ids: Vec<u32> = self.iter().map(|(id, _)| id).collect();
        QueryCombinations {
            query: self,
            ids,
            i: 0,
            j: 1,
        }
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

/// Every unordered pair of matching rows. Built by [`Query::iter_combinations`].
///
/// `(i, j)` walks the upper triangle: `j` runs ahead of `i`, so `i == j` never happens and each
/// pair is produced in exactly one order.
pub struct QueryCombinations<'a, 'w, Q: ReadOnlyQuery> {
    query: &'a Query<'w, Q>,
    /// The matching ids, collected once. Re-scanning per pair would be `n` times the work for the
    /// same answer.
    ids: Vec<u32>,
    i: usize,
    j: usize,
}

impl<'a, 'w, Q: ReadOnlyQuery> Iterator for QueryCombinations<'a, 'w, Q> {
    type Item = ((u32, Q::Item<'a>), (u32, Q::Item<'a>));

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.i + 1 >= self.ids.len() {
                return None;
            }
            if self.j >= self.ids.len() {
                self.i += 1;
                self.j = self.i + 1;
                continue;
            }
            let (a_id, b_id) = (self.ids[self.i], self.ids[self.j]);
            self.j += 1;
            // Both must still resolve. They were collected from this same query a moment ago and
            // the query holds a borrow, so this is belt-and-braces rather than a real branch —
            // but returning a half pair would be worse than skipping one.
            if let (Some(a), Some(b)) = (self.query.get(a_id), self.query.get(b_id)) {
                return Some(((a_id, a), (b_id, b)));
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.ids.len();
        (0, Some(n.saturating_mul(n.saturating_sub(1)) / 2))
    }
}



/// Every unordered pair of matching rows, both halves mutable. Built by
/// [`Query::iter_combinations_mut`].
///
/// Holds the query as a raw pointer because two `Q::Item<'a>` from one `&mut Query` cannot be
/// expressed safely — see [`Query::iter_combinations_mut`] for the invariant that makes it sound.
/// The `PhantomData` re-ties the lifetime the pointer erased, so this cannot outlive the query.
pub struct QueryCombinationsMut<'a, 'w, Q: WorldQuery> {
    query: *mut Query<'w, Q>,
    ids: Vec<u32>,
    i: usize,
    j: usize,
    _marker: PhantomData<&'a mut Query<'w, Q>>,
}

impl<'a, 'w, Q: WorldQuery> Iterator for QueryCombinationsMut<'a, 'w, Q> {
    type Item = ((u32, Q::Item<'a>), (u32, Q::Item<'a>));

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.i + 1 >= self.ids.len() {
                return None;
            }
            if self.j >= self.ids.len() {
                self.i += 1;
                self.j = self.i + 1;
                continue;
            }
            let (a_id, b_id) = (self.ids[self.i], self.ids[self.j]);
            self.j += 1;

            // The invariant, stated where it is relied on: `j` begins at `i + 1` and only
            // increases, so `a_id` and `b_id` are different entries of `ids`; and `ids` came from
            // one `iter_mut()` scan, which yields each matching row exactly once, so different
            // entries are different entities and therefore different rows.
            debug_assert_ne!(a_id, b_id, "a pair must never be an entity with itself");

            // SAFETY: `get_inner` takes `&self` and hands back `Q::Item` tied to that shared
            // borrow, so two calls produce two items with no `&mut Query` anywhere — the same
            // route `iter_mut` takes through `iter_inner`. What the two items may contain is
            // `&mut T` into component storage, and *that* is what `a_id != b_id` settles: the two
            // ids name two different rows (`j` starts at `i + 1` and only advances; `ids` came
            // from one scan, which yields each row once), so the two `&mut` are disjoint.
            //
            // The pointer is live for `'a`: it came from a `&'a mut Query`, `PhantomData` keeps
            // this iterator from outliving it, and that `&mut` means no other view of the query
            // exists while this runs.
            //
            // An earlier version reached the same lifetime with `transmute_copy` + `forget`.
            // Miri rejected it: `forget` *moves* its argument, a move is a retag, and the retag
            // invalidated the copy's own tag — "trying to retag from <...> but that tag does not
            // exist in the borrow stack". The tests passed. Going through `get_inner` needs no
            // transmute at all.
            let q: &'a Query<'w, Q> = unsafe { &*self.query };
            let a = q.get_inner(a_id)?;
            let b = q.get_inner(b_id)?;
            return Some(((a_id, a), (b_id, b)));
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.ids.len();
        (0, Some(n.saturating_mul(n.saturating_sub(1)) / 2))
    }
}

#[cfg(test)]
mod combination_tests {
    use crate::world::World;

    #[derive(Clone, Debug, PartialEq)]
    struct Tag(u32);
    crate::impl_component!(Tag);

    fn world_with(n: u32) -> World {
        let mut w = World::new();
        for i in 0..n {
            let e = w.spawn();
            w.add_component(e, Tag(i));
        }
        w
    }

    /// The three properties that make a pair loop correct, each of which fails quietly.
    ///
    /// A pair counted twice, a pair missed, or an entity paired with itself all produce motion
    /// that still looks plausible in an n-body demo — which is why this is asserted on the
    /// combinatorics rather than on a simulation's output.
    #[test]
    fn every_pair_appears_exactly_once_and_nothing_pairs_with_itself() {
        for n in [0u32, 1, 2, 3, 5, 8] {
            let w = world_with(n);
            let q = w.query::<&Tag>().expect("Tag is registered");
            let pairs: Vec<(u32, u32)> = q
                .iter_combinations()
                .map(|((_, a), (_, b))| (a.0, b.0))
                .collect();

            let expected = (n as usize * (n as usize).saturating_sub(1)) / 2;
            assert_eq!(pairs.len(), expected, "n = {n}: wrong number of pairs");

            for (a, b) in &pairs {
                assert_ne!(a, b, "n = {n}: an entity was paired with itself");
            }

            // Unordered: (a,b) and (b,a) are the same pair and only one may appear.
            let mut seen: Vec<(u32, u32)> =
                pairs.iter().map(|&(a, b)| if a < b { (a, b) } else { (b, a) }).collect();
            seen.sort_unstable();
            let before = seen.len();
            seen.dedup();
            assert_eq!(before, seen.len(), "n = {n}: a pair appeared more than once");
        }
    }

    /// The ids travel with the items, and they are the ids of *those* items.
    ///
    /// A loop that yielded the right pair of components under the wrong pair of ids would pass
    /// the test above and still write forces to the wrong entities.
    #[test]
    fn each_half_carries_its_own_entity_id() {
        let w = world_with(4);
        let q = w.query::<&Tag>().expect("registered");
        for ((a_id, a), (b_id, b)) in q.iter_combinations() {
            let ea = q.get(a_id).expect("a resolves");
            let eb = q.get(b_id).expect("b resolves");
            assert_eq!(ea.0, a.0, "the first id does not name the first item");
            assert_eq!(eb.0, b.0, "the second id does not name the second item");
        }
    }

    /// Both halves are writable, and each write lands on **its own** entity.
    ///
    /// The failure this guards is the one `unsafe` makes possible: if the two halves aliased, the
    /// second write would land on the first's row and the totals would come out wrong in a way no
    /// borrow error announces. Equal-and-opposite deltas make that visible — with correct pairing
    /// the sum is conserved, with aliasing it is not.
    #[test]
    fn both_halves_of_a_pair_write_to_their_own_row() {
        // Starting values are large enough that the negative half cannot underflow a `u32` —
        // the first run of this test panicked on `attempt to subtract with overflow`, which
        // measured the fixture rather than the pairing.
        let mut w = World::new();
        for i in 0..5u32 {
            let e = w.spawn();
            w.add_component(e, Tag(1000 + i));
        }
        {
            let mut q = w.query_mut::<crate::query::Mut<Tag>>().expect("registered");
            for ((_, mut a), (_, mut b)) in q.iter_combinations_mut() {
                a.0 += 10;
                b.0 -= 10;
            }
        }
        // 5 entities, 10 pairs. Each entity is `a` in (4 - i) pairs and `b` in i, so its net
        // change is 10 * (4 - 2i) — and the total across all of them is zero.
        let q = w.query::<&Tag>().expect("registered");
        let values: Vec<i64> = q.iter().map(|(_, t)| i64::from(t.0)).collect();
        assert_eq!(values.len(), 5);
        let initial: i64 = (0..5).map(|i| 1000 + i).sum();
        let total: i64 = values.iter().sum();
        assert_eq!(
            total, initial,
            "equal and opposite deltas did not cancel: {values:?} — a pair aliased or was \
             counted twice"
        );
        // And they really did move, so a run that wrote nothing cannot pass the sum check.
        assert_ne!(
            values,
            (0..5).map(|i| 1000 + i).collect::<Vec<i64>>(),
            "nothing was written"
        );
    }

    /// The mutable version pairs exactly like the read-only one.
    #[test]
    fn the_mutable_version_visits_the_same_pairs() {
        for n in [0u32, 1, 2, 3, 6] {
            let mut w = world_with(n);
            let readonly: Vec<(u32, u32)> = {
                let q = w.query::<&Tag>().expect("registered");
                q.iter_combinations()
                    .map(|((a, _), (b, _))| (a, b))
                    .collect()
            };
            let mutable: Vec<(u32, u32)> = {
                let mut q = w.query_mut::<crate::query::Mut<Tag>>().expect("registered");
                q.iter_combinations_mut()
                    .map(|((a, _), (b, _))| (a, b))
                    .collect()
            };
            assert_eq!(readonly, mutable, "n = {n}: the two versions disagree about pairs");
        }
    }

    /// Writes made through a pair are visible to the next pair.
    ///
    /// Not an accident of ordering — it is what makes an accumulating loop (forces, constraints)
    /// work at all, and a version that handed out stale copies would still pass the sum test above.
    #[test]
    fn a_write_is_visible_to_the_pairs_that_follow() {
        let mut w = world_with(3);
        {
            let mut q = w.query_mut::<crate::query::Mut<Tag>>().expect("registered");
            for ((_, mut a), (_, mut b)) in q.iter_combinations_mut() {
                // Each pair doubles both halves. Entity 0 is in pairs (0,1) and (0,2), so if the
                // second sees the first's write it ends at 0 * 4; a stale copy would give 0 * 2.
                let (va, vb) = (a.0, b.0);
                a.0 = va * 2;
                b.0 = vb * 2;
            }
        }
        let q = w.query::<&Tag>().expect("registered");
        let mut values: Vec<u32> = q.iter().map(|(_, t)| t.0).collect();
        values.sort_unstable();
        // 1 is in pairs (0,1) and (1,2) → ×4; 2 is in (0,2) and (1,2) → ×4.
        assert_eq!(values, vec![0, 4, 8], "a write was not visible to the following pair");
    }

    /// `size_hint`'s upper bound has to hold, or a caller that pre-allocates from it under-sizes.
    #[test]
    fn the_size_hint_bounds_the_real_count() {
        let w = world_with(6);
        let q = w.query::<&Tag>().expect("registered");
        let it = q.iter_combinations();
        let (_, upper) = it.size_hint();
        assert_eq!(upper, Some(15));
        assert_eq!(it.count(), 15);
    }
}
