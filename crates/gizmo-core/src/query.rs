use crate::archetype::Archetype;
use crate::entity::Entity;
use crate::world::World;
use std::any::TypeId;
use std::marker::PhantomData;

// =========================================================================
// FETCH COMPONENT TRAIT
// =========================================================================

pub trait FetchComponent {
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
}

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
}

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

impl<T: crate::component::Component> FetchComponent for Mut<'_, T> {
    type Component = T;
    type Fetch<'w> = (*mut u8, *mut crate::archetype::ComponentTicks, u32, Option<*mut crate::archetype::sparse_set::ComponentSparseSet>);
    type Item<'w> = Mut<'w, T>;
    type Slice<'w> = &'w mut [T];
    const IS_MUT: bool = true;

    unsafe fn fetch_raw<'w>(world: &'w World, arch: &Archetype, system_tick: u32) -> Option<Self::Fetch<'w>> {
        if T::storage_type() == crate::component::StorageType::SparseSet {
            let world_mut = world as *const World as *mut World;
            let set = (*world_mut).sparse_sets.get_mut(&TypeId::of::<T>())?;
            Some((std::ptr::null_mut(), std::ptr::null_mut(), system_tick, Some(set as *mut _)))
        } else {
            let col = arch.get_column_mut(TypeId::of::<T>())?;
            Some((col.data_ptr_mut(), col.ticks_ptr_mut(), system_tick, None))
        }
    }

    unsafe fn get_item<'w>(fetch: Self::Fetch<'w>, row: usize, entity_id: u32) -> Self::Item<'w> {
        let (data_ptr, ticks_ptr, system_tick, set_opt) = fetch;
        if let Some(set_ptr) = set_opt {
            let set = &mut *set_ptr;
            // Get index of entity in dense array to access ticks
            let e = entity_id as usize;
            let dense_row = set.sparse[e] as usize;
            let ptr = set.dense.get_unchecked_mut(dense_row) as *mut T;
            Mut {
                value: &mut *ptr,
                ticks: &mut set.ticks[dense_row],
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

    unsafe fn get_slice<'w>(fetch: Self::Fetch<'w>, len: usize) -> Self::Slice<'w> {
        let (data_ptr, ticks_ptr, system_tick, set_opt) = fetch;
        if set_opt.is_some() {
            panic!("Cannot use iter_chunks with SparseSet components");
        }
        let ticks = std::slice::from_raw_parts_mut(ticks_ptr, len);
        for tick in ticks.iter_mut() {
            tick.changed = system_tick;
        }
        std::slice::from_raw_parts_mut(data_ptr as *mut T, len)
    }
}

// =========================================================================
// WORLD QUERY TRAIT
// =========================================================================

pub trait WorldQuery {
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
}

// =========================================================================
// QUERY STRUCT
// =========================================================================

pub struct Query<'w, Q: WorldQuery + ?Sized> {
    world: &'w World,
    matching_archetypes: Vec<usize>,
    _marker: PhantomData<Q>,
}

impl<'w, Q: WorldQuery> Query<'w, Q> {
    pub fn new(world: &'w World) -> Option<Self> {
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

    pub fn new_cached(world: &'w mut World) -> Option<Self> {
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

    pub fn iter<'a>(&'a self) -> QueryIter<'a, 'w, Q> {
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

    pub fn iter_mut<'a>(&'a mut self) -> QueryIter<'a, 'w, Q> {
        self.iter()
    }

    pub fn iter_chunks<'a>(&'a self) -> QueryChunksIter<'a, 'w, Q> {
        QueryChunksIter {
            world: self.world,
            archetype_indices: &self.matching_archetypes,
            current_arch_idx: 0,
            _marker: PhantomData,
        }
    }

    pub fn iter_chunks_mut<'a>(&'a mut self) -> QueryChunksIter<'a, 'w, Q> {
        self.iter_chunks()
    }

    /// Ham `u32` id ile erişim. **DİKKAT: generation kontrolü YAPMAZ.** Despawn edilip
    /// slotu yeniden kullanılan bir id verilirse, o slottaki YENİ entity'nin verisi
    /// döner (use-after-free benzeri sessiz hata). Elinizde bir [`Entity`] handle'ı varsa
    /// [`Query::get_entity`] kullanın — o, generation'ı doğrular.
    #[inline]
    pub fn get(&self, entity_id: u32) -> Option<Q::Item<'_>> {
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

    /// Ham `u32` id ile mutable erişim — generation kontrolü yapmaz (bkz. [`Query::get`]).
    #[inline]
    pub fn get_mut(&self, entity_id: u32) -> Option<Q::Item<'_>> {
        self.get(entity_id)
    }

    /// Generation-doğrulamalı erişim: `entity` artık canlı değilse (despawn edilmiş veya
    /// slotu başka bir entity'ye verilmiş) `None` döner. Stale-handle ile yanlış entity'nin
    /// verisini okumayı engeller. Elinizde bir [`Entity`] handle'ı varsa bunu tercih edin.
    #[inline]
    pub fn get_entity(&self, entity: Entity) -> Option<Q::Item<'_>> {
        if !self.world.is_alive(entity) {
            return None;
        }
        self.get(entity.id())
    }

    /// Generation-doğrulamalı mutable erişim (bkz. [`Query::get_entity`]).
    #[inline]
    pub fn get_mut_entity(&self, entity: Entity) -> Option<Q::Item<'_>> {
        self.get_entity(entity)
    }

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

    /// Belirli bir entity'nin bu query'ye ait olup olmadığını kontrol eder.
    #[inline]
    pub fn contains(&self, entity_id: u32) -> bool {
        self.get(entity_id).is_some()
    }

    pub fn entities<'a>(&'a self) -> impl Iterator<Item = u32> + 'a {
        self.iter().map(|(id, _)| id)
    }

    /// İş parçacığı havuzu (Work-Stealing) ile çalışan lock-free paralel iterasyon
    pub fn par_for_each<F>(&self, func: F)
    where
        F: Fn((u32, Q::Item<'_>)) + Send + Sync,
    {
        use rayon::prelude::*;

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

    pub fn par_for_each_mut<F>(&mut self, func: F)
    where
        F: Fn((u32, Q::Item<'_>)) + Send + Sync,
    {
        self.par_for_each(func);
    }
}

// =========================================================================
// QUERY ITERATOR
// =========================================================================

pub struct QueryIter<'a, 'w, Q: WorldQuery> {
    world: &'a World,
    archetype_indices: &'a [usize],
    current_arch_idx: usize,
    current_row: usize,
    current_fetch: Option<Q::Fetch<'a>>,
    _marker: PhantomData<Q>,
    _marker_w: PhantomData<&'w ()>,
}

impl<'a, 'w, Q: WorldQuery> Iterator for QueryIter<'a, 'w, Q>
where
    'w: 'a,
{
    type Item = (u32, Q::Item<'a>);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.current_arch_idx >= self.archetype_indices.len() {
                return None;
            }

            let arch_idx = self.archetype_indices[self.current_arch_idx];
            let arch = &self.world.archetype_index.archetypes[arch_idx];

            let fetch = match self.current_fetch {
                Some(f) => f,
                None => {
                    match unsafe { Q::fetch_raw(self.world, arch, self.world.tick) } {
                        Some(f) => {
                            self.current_fetch = Some(f);
                            self.current_row = 0;
                            f
                        }
                        None => {
                            // Bu archetype bu query'ye uymuyor, sonrakine geç
                            self.current_arch_idx += 1;
                            continue;
                        }
                    }
                }
            };

            if self.current_row < arch.len() {
                let row = self.current_row;
                self.current_row += 1;
                let id = arch.entities()[row];
                if unsafe { Q::filter_row(fetch, row, id, self.world.change_ref_tick) } {
                    let item = unsafe { Q::get_item(fetch, row, id) };
                    return Some((id, item));
                }
                continue;
            }

            self.current_fetch = None;
            self.current_arch_idx += 1;
        }
    }

    #[inline(always)]
    fn for_each<F>(self, mut f: F)
    where
        Self: Sized,
        F: FnMut(Self::Item),
    {
        for &arch_idx in self.archetype_indices {
            let arch = &self.world.archetype_index.archetypes[arch_idx];
            let len = arch.len();
            if len == 0 {
                continue;
            }
            if let Some(fetch) = unsafe { Q::fetch_raw(self.world, arch, self.world.tick) } {
                let entities = arch.entities();
                for (row, &id) in entities.iter().enumerate().take(len) {
                    if unsafe { Q::filter_row(fetch, row, id, self.world.change_ref_tick) } {
                        let item = unsafe { Q::get_item(fetch, row, id) };
                        f((id, item));
                    }
                }
            }
        }
    }
}

// =========================================================================
// QUERY CHUNKS ITERATOR
// =========================================================================

pub struct QueryChunksIter<'a, 'w, Q: WorldQuery> {
    world: &'a World,
    archetype_indices: &'a [usize],
    current_arch_idx: usize,
    _marker: PhantomData<&'w Q>,
}

impl<'a, 'w, Q: WorldQuery> Iterator for QueryChunksIter<'a, 'w, Q>
where
    'w: 'a,
{
    type Item = (&'a [u32], Q::Slice<'a>);

    fn next(&mut self) -> Option<Self::Item> {
        while self.current_arch_idx < self.archetype_indices.len() {
            let arch_idx = self.archetype_indices[self.current_arch_idx];
            self.current_arch_idx += 1;

            let arch = &self.world.archetype_index.archetypes[arch_idx];
            let len = arch.len();
            if len == 0 {
                continue;
            }

            let fetch = match unsafe { Q::fetch_raw(self.world, arch, self.world.tick) } {
                Some(f) => f,
                None => continue,
            };

            let ids = unsafe { std::slice::from_raw_parts(arch.entities().as_ptr(), len) };
            let slice = unsafe { Q::get_slice(fetch, len) };

            return Some((ids, slice));
        }
        None
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
        if <T0::Component as crate::component::Component>::storage_type() == crate::component::StorageType::SparseSet { true } else { arch.has_component(TypeId::of::<T0::Component>()) }
    }

    unsafe fn get_item<'w>(fetch: Self::Fetch<'w>, row: usize, entity_id: u32) -> Self::Item<'w> {
        T0::get_item(fetch, row, entity_id)
    }

    unsafe fn filter_row<'w>(_fetch: Self::Fetch<'w>, _row: usize, _entity_id: u32, _tick: u32) -> bool {
        true
    }

    unsafe fn get_slice<'w>(fetch: Self::Fetch<'w>, len: usize) -> Self::Slice<'w> {
        T0::get_slice(fetch, len)
    }
}

pub struct Changed<T>(PhantomData<T>);

impl<T: crate::component::Component> WorldQuery for Changed<T> {
    type StaticType = Changed<T>;
    type Fetch<'w> = *const crate::archetype::ComponentTicks;
    type Item<'w> = ();
    type Slice<'w> = ();

    unsafe fn fetch_raw<'w>(_world: &'w World, arch: &Archetype, _tick: u32) -> Option<Self::Fetch<'w>> {
        let col = arch.get_column(TypeId::of::<T>())?;
        Some(col.ticks_ptr())
    }

    fn check_aliasing(_types: &mut Vec<(TypeId, bool)>) {}

    fn matches_archetype(arch: &Archetype) -> bool {
        if T::storage_type() == crate::component::StorageType::SparseSet { true } else { arch.has_component(TypeId::of::<T>()) }
    }

    unsafe fn filter_row<'w>(fetch: Self::Fetch<'w>, row: usize, _entity_id: u32, tick: u32) -> bool {
        if T::storage_type() == crate::component::StorageType::SparseSet {
            // TODO: Proper sparse set changed tracking, for now true if exists
            true 
        } else {
            let ticks = &*fetch.add(row);
            // `tick` burada change_ref_tick (son çalıştırmanın tick'i); referanstan
            // SONRA değişenleri eşler. (Eskiden `== current_tick` idi → kareler arası
            // çalışmıyordu.)
            ticks.changed > tick
        }
    }

    unsafe fn get_item<'w>(_fetch: Self::Fetch<'w>, _row: usize, _entity_id: u32) -> Self::Item<'w> {}

    unsafe fn get_slice<'w>(_fetch: Self::Fetch<'w>, _len: usize) -> Self::Slice<'w> {}
}

pub struct Added<T>(PhantomData<T>);

impl<T: crate::component::Component> WorldQuery for Added<T> {
    type StaticType = Added<T>;
    type Fetch<'w> = *const crate::archetype::ComponentTicks;
    type Item<'w> = ();
    type Slice<'w> = ();

    unsafe fn fetch_raw<'w>(_world: &'w World, arch: &Archetype, _tick: u32) -> Option<Self::Fetch<'w>> {
        let col = arch.get_column(TypeId::of::<T>())?;
        Some(col.ticks_ptr())
    }

    fn check_aliasing(_types: &mut Vec<(TypeId, bool)>) {}

    fn matches_archetype(arch: &Archetype) -> bool {
        if T::storage_type() == crate::component::StorageType::SparseSet { true } else { arch.has_component(TypeId::of::<T>()) }
    }

    unsafe fn filter_row<'w>(fetch: Self::Fetch<'w>, row: usize, _entity_id: u32, tick: u32) -> bool {
        if T::storage_type() == crate::component::StorageType::SparseSet {
            true 
        } else {
            let ticks = &*fetch.add(row);
            // change_ref_tick'ten sonra eklenenleri eşler (bkz. Changed).
            ticks.added > tick
        }
    }

    unsafe fn get_item<'w>(_fetch: Self::Fetch<'w>, _row: usize, _entity_id: u32) -> Self::Item<'w> {}

    unsafe fn get_slice<'w>(_fetch: Self::Fetch<'w>, _len: usize) -> Self::Slice<'w> {}
}

macro_rules! impl_query_tuple {
    ($($t:ident),*) => {
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

pub struct With<T>(PhantomData<T>);

impl<T: crate::component::Component> WorldQuery for With<T> {
    type StaticType = With<T>;
    type Fetch<'w> = ();
    type Item<'w> = ();
    type Slice<'w> = ();

    unsafe fn fetch_raw<'w>(_world: &'w World, _arch: &Archetype, _tick: u32) -> Option<Self::Fetch<'w>> {
        Some(())
    }

    fn check_aliasing(_types: &mut Vec<(TypeId, bool)>) {}

    fn matches_archetype(arch: &Archetype) -> bool {
        if T::storage_type() == crate::component::StorageType::SparseSet { true } else { arch.has_component(TypeId::of::<T>()) }
    }

    unsafe fn filter_row<'w>(_fetch: Self::Fetch<'w>, _row: usize, _entity_id: u32, _tick: u32) -> bool {
        true
    }
    unsafe fn get_item<'w>(_fetch: Self::Fetch<'w>, _row: usize, _entity_id: u32) -> Self::Item<'w> {}
    unsafe fn get_slice<'w>(_fetch: Self::Fetch<'w>, _len: usize) -> Self::Slice<'w> {}
}

pub struct Without<T>(PhantomData<T>);

impl<T: crate::component::Component> WorldQuery for Without<T> {
    type StaticType = Without<T>;
    type Fetch<'w> = ();
    type Item<'w> = ();
    type Slice<'w> = ();

    unsafe fn fetch_raw<'w>(_world: &'w World, _arch: &Archetype, _tick: u32) -> Option<Self::Fetch<'w>> {
        Some(())
    }

    fn check_aliasing(_types: &mut Vec<(TypeId, bool)>) {}

    fn matches_archetype(arch: &Archetype) -> bool {
        if T::storage_type() == crate::component::StorageType::SparseSet { true } else { !arch.has_component(TypeId::of::<T>()) }
    }

    unsafe fn filter_row<'w>(_fetch: Self::Fetch<'w>, _row: usize, _entity_id: u32, _tick: u32) -> bool {
        true
    }
    unsafe fn get_item<'w>(_fetch: Self::Fetch<'w>, _row: usize, _entity_id: u32) -> Self::Item<'w> {}
    unsafe fn get_slice<'w>(_fetch: Self::Fetch<'w>, _len: usize) -> Self::Slice<'w> {}
}

pub struct Or<T1, T2>(PhantomData<(T1, T2)>);

impl<T1: WorldQuery, T2: WorldQuery> WorldQuery for Or<T1, T2> {
    type StaticType = Or<T1::StaticType, T2::StaticType>;
    type Fetch<'w> = ();
    type Item<'w> = ();
    type Slice<'w> = ();

    unsafe fn fetch_raw<'w>(_world: &'w World, _arch: &Archetype, _tick: u32) -> Option<Self::Fetch<'w>> {
        Some(())
    }

    fn check_aliasing(_types: &mut Vec<(TypeId, bool)>) {}

    fn matches_archetype(arch: &Archetype) -> bool {
        T1::matches_archetype(arch) || T2::matches_archetype(arch)
    }

    unsafe fn filter_row<'w>(_fetch: Self::Fetch<'w>, _row: usize, _entity_id: u32, _tick: u32) -> bool {
        true
    }
    unsafe fn get_item<'w>(_fetch: Self::Fetch<'w>, _row: usize, _entity_id: u32) -> Self::Item<'w> {}
    unsafe fn get_slice<'w>(_fetch: Self::Fetch<'w>, _len: usize) -> Self::Slice<'w> {}
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
        let q = world.query::<(Mut<Position>, Mut<Velocity>)>();
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
            let mut q = world.query::<Mut<Position>>().unwrap();
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
}
