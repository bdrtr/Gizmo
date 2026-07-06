use super::sealed;
use crate::archetype::Archetype;
use crate::world::World;
use std::any::TypeId;

// =========================================================================
// FETCH COMPONENT TRAIT
// =========================================================================

pub trait FetchComponent: sealed::SealedFetch {
    type Component: 'static;
    type Fetch<'w>: Copy; // Raw pointers are Copy
    type Item<'w>;
    type Slice<'w>;

    const IS_MUT: bool;

    /// Bir archetype bazında ham pointer fetch hazırlar.
    ///
    /// # Safety
    /// Archetype geçerli olmalı ve döndürülen fetch pointer'ı archetype'ın yaşam süresi boyunca geçerli kalmalıdır.
    unsafe fn fetch_raw<'w>(world: &'w World, arch: &Archetype, system_tick: u32) -> Option<Self::Fetch<'w>>;

    /// Ham pointer'dan veriyi getirir.
    ///
    /// # Safety
    /// `row` değeri archetype'ın eleman sayısından küçük olmalıdır.
    unsafe fn get_item<'w>(fetch: Self::Fetch<'w>, row: usize, entity_id: u32) -> Self::Item<'w>;

    /// Chunk olarak ardışık belleği Slice şeklinde getirir (SIMD).
    ///
    /// # Safety
    /// `len` değeri archetype'ın eleman sayısını aşmamalıdır.
    unsafe fn get_slice<'w>(fetch: Self::Fetch<'w>, len: usize) -> Self::Slice<'w>;

    /// `entity_id`'in bu bileşeni gerçekten taşıyıp taşımadığını döndürür.
    ///
    /// `Table` depolamada bu DAİMA `true`'dur: `matches_archetype` zaten iterasyonu
    /// bileşeni içeren arketiplere kısıtlamıştır. `SparseSet` depolamada ise
    /// `matches_archetype` bilinçli olarak GENİŞTİR (her arketip için `true` döner),
    /// bu yüzden satır-başı varlık kontrolü BURADA yapılmalıdır — aksi halde `get_item`,
    /// bileşeni OLMAYAN entity'ler için sparse set'i sınır-dışı indeksler (güvenli koddan
    /// ulaşılabilen panik veya — tombstone slot'unda — release derlemede UB).
    ///
    /// # Safety
    /// `fetch`, iterlenen dünya için `fetch_raw`'dan gelmelidir.
    unsafe fn contains_entity<'w>(fetch: Self::Fetch<'w>, entity_id: u32) -> bool {
        let _ = (fetch, entity_id);
        true
    }
}

impl<T: crate::component::Component> sealed::SealedFetch for &T {}
impl<T: crate::component::Component> FetchComponent for &T {
    type Component = T;
    type Fetch<'w> = (*const u8, Option<*const crate::archetype::sparse_set::ComponentSparseSet>);
    type Item<'w> = &'w T;
    type Slice<'w> = &'w [T];
    const IS_MUT: bool = false;

    unsafe fn fetch_raw<'w>(world: &'w World, arch: &Archetype, _system_tick: u32) -> Option<Self::Fetch<'w>> {
        if T::storage_type() == crate::component::StorageType::SparseSet {
            let set = world.sparse_sets.get(&TypeId::of::<T>())?;
            Some((std::ptr::null(), Some(set as *const _)))
        } else {
            let col = arch.get_column(TypeId::of::<T>())?;
            Some((col.data_ptr(), None))
        }
    }

    unsafe fn get_item<'w>(fetch: Self::Fetch<'w>, row: usize, entity_id: u32) -> Self::Item<'w> {
        if let Some(set_ptr) = fetch.1 {
            let set = &*set_ptr;
            let ptr = set.get_ptr(entity_id).unwrap() as *const T;
            &*ptr
        } else {
            let ptr = fetch.0.add(row * std::mem::size_of::<T>()) as *const T;
            &*ptr
        }
    }

    unsafe fn get_slice<'w>(fetch: Self::Fetch<'w>, len: usize) -> Self::Slice<'w> {
        if fetch.1.is_some() {
            panic!("Cannot use iter_chunks with SparseSet components");
        }
        std::slice::from_raw_parts(fetch.0 as *const T, len)
    }

    unsafe fn contains_entity<'w>(fetch: Self::Fetch<'w>, entity_id: u32) -> bool {
        match fetch.1 {
            Some(set_ptr) => (*set_ptr).contains(entity_id),
            None => true,
        }
    }
}

/// Bir component'e değiştirme-takipli (`Changed<T>`) mutable erişim.
///
/// **Aliasing:** `world.query::<Mut<T>>()` / [`World::borrow_mut`](crate::world::World::borrow_mut)
/// `&self`'ten `&mut T` verir; aynı `T` için iki *canlı* `Mut` query'si (veya bir `Mut`
/// ile bir `&T`) aynı anda UB'dir. Çağıran sözleşmesi ve güvenli alternatifler için
/// [`World::query`](crate::world::World::query) aliasing bölümüne bakın.
pub struct Mut<'a, T: 'static> {
    value: &'a mut T,
    ticks: &'a mut crate::archetype::ComponentTicks,
    current_tick: u32,
}

impl<T> std::ops::Deref for Mut<'_, T> {
    type Target = T;
    #[inline]
    fn deref(&self) -> &T {
        self.value
    }
}

impl<T> std::ops::DerefMut for Mut<'_, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        self.ticks.changed = self.current_tick;
        self.value
    }
}

impl<'a, T> Mut<'a, T> {
    #[inline]
    pub fn bypass_change_detection(&mut self) -> &mut T {
        self.value
    }
}

impl<T: crate::component::Component> sealed::SealedFetch for Mut<'_, T> {}
impl<T: crate::component::Component> FetchComponent for Mut<'_, T> {
    type Component = T;
    type Fetch<'w> = (*mut u8, *mut crate::archetype::ComponentTicks, u32, Option<*mut crate::archetype::sparse_set::ComponentSparseSet>);
    type Item<'w> = Mut<'w, T>;
    type Slice<'w> = &'w mut [T];
    const IS_MUT: bool = true;

    unsafe fn fetch_raw<'w>(world: &'w World, arch: &Archetype, system_tick: u32) -> Option<Self::Fetch<'w>> {
        if T::storage_type() == crate::component::StorageType::SparseSet {
            // SHARED lookup — NOT `&World -> &mut World`. Casting `&World` to
            // `*mut World` to call `HashMap::get_mut` was aliasing UB (retag from
            // SharedReadOnly to a mutable permission), reachable from 100% safe
            // code via `query_mut::<Mut<Sparse>>().iter_mut()`. We only need the
            // set's address; `get_item` reaches its elements through a shared ref +
            // interior mutability (BlobVec / `UnsafeCell<ComponentTicks>`), so no
            // `&mut ComponentSparseSet` is ever formed (that would race under
            // `par_for_each_mut`).
            let set = world.sparse_sets.get(&TypeId::of::<T>())?;
            Some((std::ptr::null_mut(), std::ptr::null_mut(), system_tick, Some(set as *const _ as *mut _)))
        } else {
            let col = arch.get_column_mut(TypeId::of::<T>())?;
            Some((col.data_ptr_mut(), col.ticks_ptr_mut(), system_tick, None))
        }
    }

    unsafe fn get_item<'w>(fetch: Self::Fetch<'w>, row: usize, entity_id: u32) -> Self::Item<'w> {
        let (data_ptr, ticks_ptr, system_tick, set_opt) = fetch;
        if let Some(set_ptr) = set_opt {
            // SHARED `&*set_ptr` (not `&mut`). `par_for_each_mut` runs archetype/row tasks in
            // parallel; every entity lives in exactly one dense row, so the rows written are
            // disjoint across tasks. Forming an exclusive `&mut *set_ptr` per task would be
            // instant aliasing UB (many live `&mut ComponentSparseSet`) — a real data race
            // reachable from 100% safe code (`query_mut::<Mut<Sparse>>().par_for_each_mut`).
            // BlobVec::get_unchecked_mut takes `&self` (interior mutability) and Vec::as_ptr
            // gives the ticks base, so we reach the disjoint element through a shared ref only.
            let set = &*set_ptr;
            let e = entity_id as usize;
            let dense_row = set.sparse[e] as usize;
            let ptr = set.dense.get_unchecked_mut(dense_row) as *mut T;
            // `ticks` is `Vec<UnsafeCell<ComponentTicks>>`; `UnsafeCell::get` yields a
            // write-provenance `*mut` through the shared `&set`, so mutating disjoint
            // rows from parallel tasks is sound (a raw `Vec::as_ptr` would not be).
            let ticks_ptr = (*set.ticks.as_ptr().add(dense_row)).get();
            Mut {
                value: &mut *ptr,
                ticks: &mut *ticks_ptr,
                current_tick: system_tick,
            }
        } else {
            let ptr = data_ptr.add(row * std::mem::size_of::<T>()) as *mut T;
            Mut {
                value: &mut *ptr,
                ticks: &mut *ticks_ptr.add(row),
                current_tick: system_tick,
            }
        }
    }

    unsafe fn contains_entity<'w>(fetch: Self::Fetch<'w>, entity_id: u32) -> bool {
        match fetch.3 {
            Some(set_ptr) => (*set_ptr).contains(entity_id),
            None => true,
        }
    }

    unsafe fn get_slice<'w>(fetch: Self::Fetch<'w>, len: usize) -> Self::Slice<'w> {
        let (data_ptr, ticks_ptr, system_tick, set_opt) = fetch;
        if set_opt.is_some() {
            panic!("Cannot use iter_chunks with SparseSet components");
        }
        // Temkinli (conservative) işaretleme: ham `&mut [T]` dilimde hangi elemanın
        // yazıldığı izlenemediğinden verilen tüm satırlar "changed" işaretlenir. Bu,
        // gerçek bir yazmayı asla kaçırmaz (güvenli); detay için bkz. `iter_chunks_mut`.
        let ticks = std::slice::from_raw_parts_mut(ticks_ptr, len);
        for tick in ticks.iter_mut() {
            tick.changed = system_tick;
        }
        std::slice::from_raw_parts_mut(data_ptr as *mut T, len)
    }
}
