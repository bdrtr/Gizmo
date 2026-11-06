//! `Query` access + iteration methods (mutable and read-only). Extracted verbatim from
//! query/mod.rs (pure move); these are inherent `impl Query` blocks, so they compose back onto
//! the `Query` struct in the parent module. `use super::*` brings in WorldQuery/ReadOnlyQuery,
//! the archetype/fetch machinery and `Mut`.

use super::iter::QueryRows;
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

    /// [`new`](Self::new), plus the world's [`DefaultQueryFilters`](crate::query::DefaultQueryFilters).
    ///
    /// Called from exactly one place — `SystemParam for Query`'s fetch — and that is the whole
    /// boundary. Every other way of building a query sees the world entire, `query_unchecked` and
    /// `borrow_mut_unchecked` included: those are the engine's mutable-from-shared hatch, used at
    /// 98 sites in 37 files, nine of them editor panels that are not systems at all.
    /// Filtering there blanked the inspector, froze transform subtrees and made `sync_bodies`
    /// destroy a disabled rigid body — see [`DefaultQueryFilters`](crate::query::DefaultQueryFilters).
    ///
    /// The filter is applied to the **archetype list**, so it costs one predicate per archetype at
    /// construction and nothing per row, and `has_row_filter` is untouched — a query carrying no
    /// row filter can still be chunk-iterated after being filtered. That is possible only because
    /// a filter type must be `Table`-stored, which `DefaultQueryFilters::add` enforces.
    pub(crate) fn new_for_system(world: &'w World) -> Option<Self> {
        let mut query = Self::new(world)?;
        // Two early outs, in the order that makes the common case free: no filters registered at
        // all (every world that never asked for the feature), then this query opting out.
        if world.default_query_filters.is_empty() || Q::ignores_default_filters() {
            return Some(query);
        }
        let filters = &world.default_query_filters;
        let archetypes = &world.archetype_index.archetypes;
        query
            .matching_archetypes
            .retain(|&idx| !filters.excludes(&archetypes[idx]));
        Some(query)
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

    fn iter_inner<'a>(&'a self) -> QueryRows<'a, 'w, Q> {
        QueryRows {
            world: self.world,
            archetype_indices: &self.matching_archetypes,
            current_arch_idx: 0,
            current_row: 0,
            current_fetch: None,
            _marker: PhantomData,
            _marker_w: PhantomData,
        }
    }

    fn single_inner(&self) -> Result<Q::Item<'_>, QuerySingleError> {
        let mut rows = self.iter_inner();
        let (_, first) = rows.next().ok_or(QuerySingleError::NoEntities)?;
        if rows.next().is_some() {
            return Err(QuerySingleError::MultipleEntities);
        }
        Ok(first)
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
    fn get_inner<'a, K: super::QueryKey>(&'a self, key: K) -> Option<Q::Item<'a>> {
        let entity_id = key.slot();
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
        if !key.means(arch, loc.row as usize) {
            return None;
        }
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
                        let id = (*entities_ptr.get().add(row)).id();
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
        QueryIter(self.iter_inner())
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

    /// Mutable access to one row, by [`Entity`](crate::entity::Entity) or raw id: see [`Query::get`].
    /// Because it takes `&mut self` the returned `Mut` borrows the query exclusively; a second
    /// simultaneous `get_mut`/`iter_mut` does not compile.
    #[inline]
    pub fn get_mut(&mut self, key: impl super::QueryKey) -> Option<Q::Item<'_>> {
        self.get_inner(key)
    }

    /// Every unordered `K`-element combination of matching rows, **all of them writable**.
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
    /// let mut pairs = q.iter_combinations_mut();
    /// while let Some([mut a, mut b]) = pairs.fetch_next() {
    ///     // Equal and opposite: what a two-pass version has to reconstruct.
    ///     a.0 += 1.0;
    ///     b.0 -= 1.0;
    /// }
    /// ```
    ///
    /// `fetch_next` rather than `for`: see [`QueryCombinationsMut`] for why this cannot be an
    /// iterator.
    ///
    /// # Cost
    ///
    /// The same as the read-only version: one `Vec<u32>` of ids up front, then `K` lookups per
    /// combination.
    pub fn iter_combinations_mut<'a, const K: usize>(
        &'a mut self,
    ) -> QueryCombinationsMut<'a, 'w, Q, K> {
        let ids: Vec<u32> = self.iter_inner().map(|(id, _)| id).collect();
        let subsets = Subsets::new(ids.len());
        QueryCombinationsMut {
            query: self,
            ids,
            subsets,
        }
    }

    /// The one matching row, writable — see [`Query::single`] for the rules, which are the same.
    ///
    /// Sound for the same reason [`iter_mut`](Self::iter_mut) is: the result borrows the query
    /// exclusively. To tell "one" from "several" it has to look at a second row; that second item
    /// is built and dropped without being written, so it is not marked changed.
    pub fn single_mut(&mut self) -> Result<Q::Item<'_>, QuerySingleError> {
        self.single_inner()
    }

    /// Lock-free parallel mutable iteration running on the thread pool (Work-Stealing).
    pub fn par_for_each_mut<F>(&mut self, func: F)
    where
        F: Fn(Q::Item<'_>) + Send + Sync,
    {
        self.par_inner(|(_, item)| func(item));
    }

    // ── Metadata (no component access → always `&self`) ───────────────────

    /// The [`Entity`](crate::entity::Entity) handle — id **and generation** — for a raw id this query yielded, or `None`
    /// if the id is not a live entity.
    ///
    /// Iteration hands out bare `u32` ids, and a scheduled system cannot reach
    /// [`World::entity`](crate::world::World::entity) (it is never given the world). This is the
    /// same lookup through the world the query already holds, so a system can turn an id into a
    /// handle for `Commands` or for storing, without forging one with `Entity::new(id, 0)` — which
    /// compiles and is wrong for every recycled id. Reads the allocator under its lock, as
    /// `Commands` does, so it is safe from systems running in parallel. It reads no component, so
    /// a query that writes has it too.
    pub fn entity(&self, entity_id: u32) -> Option<crate::entity::Entity> {
        self.world.entity(entity_id)
    }

    /// The id of `entity` if it is still alive: how to follow a stored handle, such as a
    /// [`Parent`](crate::component::Parent) or an entry of [`Children`](crate::component::Children),
    /// to the row it means. `None` for a dead one, whose id may belong to someone else by now.
    pub fn live(&self, entity: crate::entity::Entity) -> Option<u32> {
        self.world.is_alive(entity).then_some(entity.id())
    }

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
    /// See [`QueryIter`] for the visit order.
    pub fn iter<'a>(&'a self) -> QueryIter<'a, 'w, Q> {
        QueryIter(self.iter_inner())
    }

    /// Every unordered `K`-element combination of matching rows, each yielded once. `K = 2` is a
    /// **pair**:
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
    /// let pairs: Vec<_> = q.iter_combinations().map(|[a, b]| (a.0, b.0)).collect();
    /// assert_eq!(pairs.len(), 3); // 3 choose 2
    /// ```
    ///
    /// The pattern `[a, b]` is what picks `K = 2`; write `iter_combinations::<3>()` where nothing
    /// names it. A writing loop uses [`iter_combinations_mut`](Self::iter_combinations_mut).
    ///
    /// # Cost
    ///
    /// One `Vec<u32>` of matching ids up front, then `K` [`get`](Self::get) lookups per combination.
    /// The ids are collected once rather than re-scanned, and before the first combination is
    /// yielded, so a structural change during iteration is impossible anyway (the query holds a
    /// borrow), and the visit order is the one [`QueryIter`] documents.
    pub fn iter_combinations<'a, const K: usize>(&'a self) -> QueryCombinations<'a, 'w, Q, K> {
        let ids: Vec<u32> = self.iter_inner().map(|(id, _)| id).collect();
        let subsets = Subsets::new(ids.len());
        QueryCombinations {
            query: self,
            ids,
            subsets,
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

    /// The one matching row's item, or why there is not exactly one.
    ///
    /// For the query a game writes for *the* player, *the* main camera, *the* score: written as a
    /// loop it silently does the wrong thing in both edge cases — no body runs when nothing has
    /// spawned yet, and the last of two matches wins without a word when something spawned twice.
    /// This says which case it is.
    ///
    /// ```
    /// # use gizmo_core::prelude::*;
    /// # use gizmo_core::query::QuerySingleError;
    /// # #[derive(Clone, Debug)] struct Player(u32);
    /// # gizmo_core::impl_component!(Player);
    /// let mut world = World::new();
    /// assert_eq!(world.query::<&Player>().unwrap().single().err(), Some(QuerySingleError::NoEntities));
    ///
    /// let e = world.spawn();
    /// world.add_component(e, Player(7));
    /// let q = world.query::<&Player>().unwrap();
    /// assert_eq!(q.single().unwrap().0, 7);
    /// ```
    ///
    /// It walks at most two rows, and it applies the per-row filters exactly as
    /// [`iter`](Self::iter) does, so a `Changed<T>` query is "the one that changed this frame".
    pub fn single(&self) -> Result<Q::Item<'_>, QuerySingleError> {
        self.single_inner()
    }

    /// One row, by [`Entity`](crate::entity::Entity) or raw id. An `Entity` is checked against the row,
    /// so one that has died returns `None` even after its slot went to someone else. **A raw id cannot
    /// tell them apart** and returns whoever holds the slot now.
    #[inline]
    pub fn get(&self, key: impl super::QueryKey) -> Option<Q::Item<'_>> {
        self.get_inner(key)
    }

    /// Checks whether a given entity belongs to this query.
    #[inline]
    pub fn contains(&self, key: impl super::QueryKey) -> bool {
        self.get_inner(key).is_some()
    }

    /// Lock-free parallel iteration running on the thread pool (Work-Stealing)
    pub fn par_for_each<F>(&self, func: F)
    where
        F: Fn(Q::Item<'_>) + Send + Sync,
    {
        self.par_inner(|(_, item)| func(item));
    }
}

/// Walks every `K`-element subset of `0..n` in lexicographic order, each exactly once.
///
/// Indices strictly increase within a subset, so no two positions of one combination are ever the
/// same row. That is what makes [`QueryCombinationsMut`] sound, and it lives here, in one place.
struct Subsets<const K: usize> {
    next: Option<[usize; K]>,
    n: usize,
}

impl<const K: usize> Subsets<K> {
    fn new(n: usize) -> Self {
        Self {
            // `K <= n` also covers `K == 0`: there is exactly one empty subset of anything.
            next: (K <= n).then(|| std::array::from_fn(|i| i)),
            n,
        }
    }

    fn advance(&mut self) -> Option<[usize; K]> {
        let current = self.next?;
        let mut following = current;
        // The rightmost position that can still move: position `i` tops out at `n - K + i`.
        self.next = (0..K).rev().find(|&i| following[i] < self.n - K + i).map(|i| {
            following[i] += 1;
            for j in i + 1..K {
                following[j] = following[j - 1] + 1;
            }
            following
        });
        Some(current)
    }

    /// `C(n, K)`, or `None` if it does not fit a `usize`.
    fn total(&self) -> Option<usize> {
        if K > self.n {
            return Some(0);
        }
        let k = K.min(self.n - K) as u128;
        let mut c = 1u128;
        for i in 0..k {
            c = c.checked_mul(self.n as u128 - i)? / (i + 1);
        }
        usize::try_from(c).ok()
    }
}

/// Every unordered `K`-element combination of matching rows. Built by
/// [`Query::iter_combinations`].
pub struct QueryCombinations<'a, 'w, Q: ReadOnlyQuery, const K: usize> {
    query: &'a Query<'w, Q>,
    /// The matching rows, collected once. Re-scanning per combination would be `n` times the work
    /// for the same answer.
    ids: Vec<u32>,
    subsets: Subsets<K>,
}

impl<'a, 'w, Q: ReadOnlyQuery, const K: usize> Iterator for QueryCombinations<'a, 'w, Q, K> {
    type Item = [Q::Item<'a>; K];

    fn next(&mut self) -> Option<Self::Item> {
        let chosen = self.subsets.advance()?;
        // `ids` came from this query a moment ago and the query holds a borrow, so every row still
        // resolves.
        Some(chosen.map(|i| {
            self.query
                .get_inner(self.ids[i])
                .expect("a row matched by this query's own scan resolves")
        }))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, self.subsets.total())
    }
}

/// Every unordered `K`-element combination of matching rows, all writable, one at a time. Built by
/// [`Query::iter_combinations_mut`].
///
/// Not an [`Iterator`], deliberately: an iterator's items may outlive the call that made them, and
/// row 0 is in the pairs `(0, 1)` and `(0, 2)` alike, so collecting them would be two live `&mut` to
/// one row. [`fetch_next`](Self::fetch_next) ties each combination to a borrow of this value, so the
/// previous one is gone before the next exists.
///
/// ```compile_fail,E0499
/// # use gizmo_core::prelude::*;
/// # use gizmo_core::query::Mut;
/// # #[derive(Clone)] struct Tag(u32);
/// # gizmo_core::impl_component!(Tag);
/// # let mut world = World::new();
/// let mut q = world.query_mut::<Mut<Tag>>().unwrap();
/// let mut pairs = q.iter_combinations_mut::<2>();
/// let first = pairs.fetch_next();
/// let second = pairs.fetch_next(); // the first combination is still alive
/// drop((first, second));
/// ```
pub struct QueryCombinationsMut<'a, 'w, Q: WorldQuery, const K: usize> {
    query: &'a mut Query<'w, Q>,
    ids: Vec<u32>,
    subsets: Subsets<K>,
}

impl<'a, 'w, Q: WorldQuery, const K: usize> QueryCombinationsMut<'a, 'w, Q, K> {
    /// The next combination, valid until this is called again.
    pub fn fetch_next(&mut self) -> Option<[Q::Item<'_>; K]> {
        let chosen = self.subsets.advance()?;
        // `Subsets` gives strictly increasing positions and `ids` holds each matching row once, so
        // these are different rows and the items do not alias. Exclusive access to the query for
        // this borrow is what keeps anyone else from looking at them.
        let query: &Query<'w, Q> = self.query;
        let ids = &self.ids;
        Some(chosen.map(|i| {
            query
                .get_inner(ids[i])
                .expect("a row matched by this query's own scan resolves")
        }))
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
                .map(|[a, b]| (a.0, b.0))
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

    /// A handle in the query travels with its own items.
    ///
    /// A loop that yielded the right pair of components under the wrong pair of handles would pass
    /// the test above and still write forces to the wrong entities.
    #[test]
    fn each_half_carries_its_own_entity() {
        let w = world_with(4);
        let q = w.query::<(crate::entity::Entity, &Tag)>().expect("registered");
        for [(ea, a), (eb, b)] in q.iter_combinations() {
            assert_eq!(q.get(ea).expect("a resolves").1 .0, a.0, "first handle, wrong item");
            assert_eq!(q.get(eb).expect("b resolves").1 .0, b.0, "second handle, wrong item");
        }
    }

    /// `K` is not only 2: every subset once, none with a repeated row, and the edges of `C(n, K)`.
    #[test]
    fn every_k_visits_exactly_n_choose_k_distinct_rows() {
        fn c(n: usize, k: usize) -> usize {
            if k > n {
                return 0;
            }
            (0..k).fold(1, |acc, i| acc * (n - i) / (i + 1))
        }
        fn check<const K: usize>(n: u32) {
            let w = world_with(n);
            let q = w.query::<&Tag>().expect("registered");
            let mut seen: Vec<[u32; K]> = q.iter_combinations::<K>().map(|c| c.map(|t| t.0)).collect();
            assert_eq!(seen.len(), c(n as usize, K), "n = {n}, K = {K}");
            for combo in &seen {
                assert!(combo.windows(2).all(|p| p[0] != p[1]), "a row repeated: {combo:?}");
            }
            seen.sort_unstable();
            let before = seen.len();
            seen.dedup();
            assert_eq!(before, seen.len(), "n = {n}, K = {K}: a combination came twice");
            assert_eq!(q.iter_combinations::<K>().size_hint().1, Some(c(n as usize, K)));
        }
        for n in 0..7 {
            check::<0>(n);
            check::<1>(n);
            check::<2>(n);
            check::<3>(n);
            check::<4>(n);
        }
    }

    /// The writable version visits what the read-only one does, in any `K`.
    #[test]
    fn three_writable_rows_at_once_do_not_alias() {
        let mut w = world_with(5);
        {
            let mut q = w.query_mut::<crate::query::Mut<Tag>>().expect("registered");
            let mut triples = q.iter_combinations_mut::<3>();
            let mut count = 0;
            while let Some([mut a, mut b, mut c]) = triples.fetch_next() {
                a.0 += 100;
                b.0 += 10;
                c.0 += 1;
                count += 1;
            }
            assert_eq!(count, 10, "5 choose 3");
        }
        // Row 0 is first in all 6 triples containing it, row 4 last in all 6 of its own.
        let q = w.query::<&Tag>().expect("registered");
        let got: Vec<u32> = q.iter().map(|t| t.0).collect();
        // Tag starts at the row number. Row i is first in C(4-i, 2) triples, middle in i*(4-i),
        // last in C(i, 2).
        assert_eq!(got, vec![600, 1 + 3 * 100 + 3 * 10, 2 + 100 + 4 * 10 + 1, 3 + 3 * 10 + 3, 4 + 6]);
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
            let mut pairs = q.iter_combinations_mut();
            while let Some([mut a, mut b]) = pairs.fetch_next() {
                a.0 += 10;
                b.0 -= 10;
            }
        }
        // 5 entities, 10 pairs. Each entity is `a` in (4 - i) pairs and `b` in i, so its net
        // change is 10 * (4 - 2i) — and the total across all of them is zero.
        let q = w.query::<&Tag>().expect("registered");
        let values: Vec<i64> = q.iter().map(|t| i64::from(t.0)).collect();
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
                q.iter_combinations().map(|[a, b]| (a.0, b.0)).collect()
            };
            let mutable: Vec<(u32, u32)> = {
                let mut q = w.query_mut::<crate::query::Mut<Tag>>().expect("registered");
                let mut pairs = q.iter_combinations_mut();
                let mut out = Vec::new();
                while let Some([a, b]) = pairs.fetch_next() {
                    out.push((a.0, b.0));
                }
                out
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
            let mut pairs = q.iter_combinations_mut();
            while let Some([mut a, mut b]) = pairs.fetch_next() {
                // Each pair doubles both halves. Entity 0 is in pairs (0,1) and (0,2), so if the
                // second sees the first's write it ends at 0 * 4; a stale copy would give 0 * 2.
                let (va, vb) = (a.0, b.0);
                a.0 = va * 2;
                b.0 = vb * 2;
            }
        }
        let q = w.query::<&Tag>().expect("registered");
        let mut values: Vec<u32> = q.iter().map(|t| t.0).collect();
        values.sort_unstable();
        // 1 is in pairs (0,1) and (1,2) → ×4; 2 is in (0,2) and (1,2) → ×4.
        assert_eq!(values, vec![0, 4, 8], "a write was not visible to the following pair");
    }

    /// `size_hint`'s upper bound has to hold, or a caller that pre-allocates from it under-sizes.
    #[test]
    fn the_size_hint_bounds_the_real_count() {
        let w = world_with(6);
        let q = w.query::<&Tag>().expect("registered");
        let it = q.iter_combinations::<2>();
        let (_, upper) = it.size_hint();
        assert_eq!(upper, Some(15));
        assert_eq!(it.count(), 15);
    }
}
