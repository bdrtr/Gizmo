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
