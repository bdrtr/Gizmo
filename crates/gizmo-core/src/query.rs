use crate::archetype::Archetype;
use crate::world::World;
use std::any::TypeId;
use std::marker::PhantomData;

// =========================================================================
// FETCH COMPONENT TRAIT
// =========================================================================

pub trait FetchComponent<'w> {
    type Component: 'static;
    type Fetch: Copy; // Raw pointers are Copy
    type Item<'a> where 'w: 'a;
    type Slice<'a> where 'w: 'a;
    
    const IS_MUT: bool;

    /// Bir archetype bazında ham pointer fetch hazırlar.
    unsafe fn fetch_raw(arch: &Archetype, system_tick: u32) -> Option<Self::Fetch>;
    
    /// Ham pointer'dan veriyi getirir.
    unsafe fn get_item<'a>(fetch: Self::Fetch, row: usize) -> Self::Item<'a>
    where
        'w: 'a;

    /// Chunk olarak ardışık belleği Slice şeklinde getirir (SIMD).
    unsafe fn get_slice<'a>(fetch: Self::Fetch, len: usize) -> Self::Slice<'a>
    where
        'w: 'a;
}

impl<'w, T: 'static> FetchComponent<'w> for &'w T {
    type Component = T;
    type Fetch = *const u8;
    type Item<'a> = &'a T where 'w: 'a;
    type Slice<'a> = &'a [T] where 'w: 'a;
    const IS_MUT: bool = false;

    unsafe fn fetch_raw(arch: &Archetype, _system_tick: u32) -> Option<Self::Fetch> {
        let col = arch.get_column(TypeId::of::<T>())?;
        Some(col.data_ptr())
    }
    
    unsafe fn get_item<'a>(fetch: Self::Fetch, row: usize) -> Self::Item<'a>
    where
        'w: 'a
    {
        let ptr = fetch.add(row * std::mem::size_of::<T>()) as *const T;
        &*ptr
    }

    unsafe fn get_slice<'a>(fetch: Self::Fetch, len: usize) -> Self::Slice<'a>
    where
        'w: 'a
    {
        std::slice::from_raw_parts(fetch as *const T, len)
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
    fn deref(&self) -> &T { self.value }
}

impl<T> std::ops::DerefMut for Mut<'_, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        self.ticks.changed = self.current_tick;
        self.value
    }
}

impl<'w, T: 'static> FetchComponent<'w> for Mut<'w, T> {
    type Component = T;
    type Fetch = (*mut u8, *mut crate::archetype::ComponentTicks, u32);
    type Item<'a> = Mut<'a, T> where 'w: 'a;
    type Slice<'a> = &'a mut [T] where 'w: 'a;
    const IS_MUT: bool = true;

    unsafe fn fetch_raw(arch: &Archetype, system_tick: u32) -> Option<Self::Fetch> {
        let col = arch.get_column_mut(TypeId::of::<T>())?;
        Some((col.data_ptr_mut(), col.ticks_ptr_mut(), system_tick))
    }
    
    unsafe fn get_item<'a>(fetch: Self::Fetch, row: usize) -> Self::Item<'a>
    where
        'w: 'a
    {
        let (data_ptr, ticks_ptr, system_tick) = fetch;
        let ptr = data_ptr.add(row * std::mem::size_of::<T>()) as *mut T;
        Mut {
            value: &mut *ptr,
            ticks: &mut *ticks_ptr.add(row),
            current_tick: system_tick,
        }
    }

    unsafe fn get_slice<'a>(fetch: Self::Fetch, len: usize) -> Self::Slice<'a>
    where
        'w: 'a
    {
        let (data_ptr, ticks_ptr, system_tick) = fetch;
        
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

pub trait WorldQuery<'w> {
    type StaticType: 'static;
    type Fetch: Copy;
    type Item<'a> where 'w: 'a;
    type Slice<'a> where 'w: 'a;

    unsafe fn fetch_raw(arch: &Archetype, system_tick: u32) -> Option<Self::Fetch>;
    fn check_aliasing(types: &mut Vec<(TypeId, bool)>);
    fn matches_archetype(arch: &Archetype) -> bool;
    
    unsafe fn get_item<'a>(fetch: Self::Fetch, row: usize) -> Self::Item<'a>
    where
        'w: 'a;

    unsafe fn filter_row(fetch: Self::Fetch, row: usize, system_tick: u32) -> bool;

    unsafe fn get_slice<'a>(fetch: Self::Fetch, len: usize) -> Self::Slice<'a>
    where
        'w: 'a;
}

// =========================================================================
// QUERY STRUCT
// =========================================================================

pub struct Query<'w, Q: WorldQuery<'w>> {
    world: &'w World,
    matching_archetypes: Vec<usize>,
    _marker: PhantomData<Q>,
}

impl<'w, Q: WorldQuery<'w>> Query<'w, Q> {
    pub fn new(world: &'w World) -> Option<Self> {
        let mut used_types = Vec::new();
        Q::check_aliasing(&mut used_types);
        let matching = world.archetype_index.matching_archetypes_readonly(Q::matches_archetype);
        Some(Self { world, matching_archetypes: matching, _marker: PhantomData })
    }

    pub fn new_cached(world: &'w mut World) -> Option<Self> {
        let mut used_types = Vec::new();
        Q::check_aliasing(&mut used_types);
        let matching = world.archetype_index.matching_archetypes(TypeId::of::<Q::StaticType>(), Q::matches_archetype).to_vec();
        Some(Self { world, matching_archetypes: matching, _marker: PhantomData })
    }

    pub fn iter<'a>(&'a self) -> QueryIter<'a, 'w, Q> {
        QueryIter {
            world: self.world,
            archetype_indices: &self.matching_archetypes,
            current_arch_idx: 0,
            current_row: 0,
            current_fetch: None,
            _marker: PhantomData,
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

    #[inline]
    pub fn get(&self, entity_id: u32) -> Option<Q::Item<'_>> {
        let loc = self.world.entity_location(entity_id);
        if !loc.is_valid() { return None; }
        let arch = &self.world.archetype_index.archetypes[loc.archetype_id as usize];
        unsafe {
            let fetch = Q::fetch_raw(arch, self.world.tick)?;
            if !Q::filter_row(fetch, loc.row as usize, self.world.tick) { return None; }
            Some(Q::get_item(fetch, loc.row as usize))
        }
    }

    #[inline]
    pub fn entity_count(&self) -> usize {
        self.matching_archetypes.iter()
            .map(|&idx| self.world.archetype_index.archetypes[idx].len())
            .sum()
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
        self.matching_archetypes.par_iter().for_each(|&arch_idx| {
            let arch = &self.world.archetype_index.archetypes[arch_idx];
            if let Some(fetch) = unsafe { Q::fetch_raw(arch, tick) } {
                let len = arch.len();
                let wrapped_fetch = FetchWrapper(fetch);
                let entities_ptr = FetchWrapper(arch.entities().as_ptr());
                let func_ref = &func;

                // Her Archetype'ı cache dostu chunk'lar halinde ayırıp process ediyoruz
                // Chunk size: 512 (Bevy benzeri)
                (0..len).into_par_iter().with_min_len(512).for_each(move |row| {
                    unsafe {
                        if Q::filter_row(wrapped_fetch.get(), row, tick) {
                            let id = *entities_ptr.get().add(row);
                            let item = Q::get_item(wrapped_fetch.get(), row);
                            func_ref((id, item));
                        }
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

pub struct QueryIter<'a, 'w, Q: WorldQuery<'w>> {
    world: &'a World,
    archetype_indices: &'a [usize],
    current_arch_idx: usize,
    current_row: usize,
    current_fetch: Option<Q::Fetch>,
    _marker: PhantomData<Q>,
}

impl<'a, 'w, Q: WorldQuery<'w>> Iterator for QueryIter<'a, 'w, Q> 
where
    'w: 'a
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
                    let f = unsafe { Q::fetch_raw(arch, self.world.tick)? };
                    self.current_fetch = Some(f);
                    self.current_row = 0;
                    f
                }
            };

            if self.current_row < arch.len() {
                let row = self.current_row;
                self.current_row += 1;
                if unsafe { Q::filter_row(fetch, row, self.world.tick) } {
                    let id = arch.entities()[row];
                    let item = unsafe { Q::get_item(fetch, row) };
                    return Some((id, item));
                }
                continue;
            }

            self.current_fetch = None;
            self.current_arch_idx += 1;
        }
    }
}

// =========================================================================
// QUERY CHUNKS ITERATOR
// =========================================================================

pub struct QueryChunksIter<'a, 'w, Q: WorldQuery<'w>> {
    world: &'a World,
    archetype_indices: &'a [usize],
    current_arch_idx: usize,
    _marker: PhantomData<&'w Q>,
}

impl<'a, 'w, Q: WorldQuery<'w>> Iterator for QueryChunksIter<'a, 'w, Q> 
where
    'w: 'a
{
    type Item = (&'a [u32], Q::Slice<'a>);

    fn next(&mut self) -> Option<Self::Item> {
        while self.current_arch_idx < self.archetype_indices.len() {
            let arch_idx = self.archetype_indices[self.current_arch_idx];
            self.current_arch_idx += 1;
            
            let arch = &self.world.archetype_index.archetypes[arch_idx];
            let len = arch.len();
            if len == 0 { continue; }

            let fetch = match unsafe { Q::fetch_raw(arch, self.world.tick) } {
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

#[inline]
fn check(tid: TypeId, is_mut: bool, types: &mut Vec<(TypeId, bool)>) {
    if types.iter().any(|(t, m)| *t == tid && (*m || is_mut)) {
        panic!("Aliasing UB in Query!");
    }
    types.push((tid, is_mut));
}

impl<'w, T0: FetchComponent<'w>> WorldQuery<'w> for T0 {
    type StaticType = T0::Component;
    type Fetch = T0::Fetch;
    type Item<'a> = T0::Item<'a> where 'w: 'a;
    type Slice<'a> = T0::Slice<'a> where 'w: 'a;

    unsafe fn fetch_raw(arch: &Archetype, tick: u32) -> Option<Self::Fetch> { T0::fetch_raw(arch, tick) }
    fn check_aliasing(types: &mut Vec<(TypeId, bool)>) { check(TypeId::of::<T0::Component>(), T0::IS_MUT, types); }
    fn matches_archetype(arch: &Archetype) -> bool { arch.has_component(TypeId::of::<T0::Component>()) }

    unsafe fn get_item<'a>(fetch: Self::Fetch, row: usize) -> Self::Item<'a>
    where
        'w: 'a
    {
        T0::get_item(fetch, row)
    }

    unsafe fn filter_row(_fetch: Self::Fetch, _row: usize, _tick: u32) -> bool { true }

    unsafe fn get_slice<'a>(fetch: Self::Fetch, len: usize) -> Self::Slice<'a>
    where
        'w: 'a
    {
        T0::get_slice(fetch, len)
    }
}

pub struct Changed<T>(PhantomData<T>);

impl<'w, T: 'static> WorldQuery<'w> for Changed<T> {
    type StaticType = Changed<T>;
    type Fetch = *const crate::archetype::ComponentTicks;
    type Item<'a> = () where 'w: 'a;
    type Slice<'a> = () where 'w: 'a;

    unsafe fn fetch_raw(arch: &Archetype, _tick: u32) -> Option<Self::Fetch> {
        let col = arch.get_column(TypeId::of::<T>())?;
        Some(col.ticks_ptr())
    }

    fn check_aliasing(_types: &mut Vec<(TypeId, bool)>) {}

    fn matches_archetype(arch: &Archetype) -> bool {
        arch.has_component(TypeId::of::<T>())
    }

    unsafe fn filter_row(fetch: Self::Fetch, row: usize, tick: u32) -> bool {
        let ticks = &*fetch.add(row);
        ticks.changed == tick
    }

    unsafe fn get_item<'a>(_fetch: Self::Fetch, _row: usize) -> Self::Item<'a> where 'w: 'a {
        ()
    }

    unsafe fn get_slice<'a>(_fetch: Self::Fetch, _len: usize) -> Self::Slice<'a> where 'w: 'a {
        ()
    }
}

macro_rules! impl_query_tuple {
    ($($t:ident),*) => {
        #[allow(non_snake_case)]
        impl<'w, $($t: WorldQuery<'w>),*> WorldQuery<'w> for ($($t,)*) {
            type StaticType = ($($t::StaticType,)*);
            type Fetch = ($($t::Fetch,)*);
            type Item<'a> = ($($t::Item<'a>,)*) where 'w: 'a;
            type Slice<'a> = ($($t::Slice<'a>,)*) where 'w: 'a;

            unsafe fn fetch_raw(arch: &Archetype, tick: u32) -> Option<Self::Fetch> {
                Some(($($t::fetch_raw(arch, tick)?,)*))
            }
            fn check_aliasing(types: &mut Vec<(TypeId, bool)>) {
                $($t::check_aliasing(types);)*
            }
            fn matches_archetype(arch: &Archetype) -> bool {
                $($t::matches_archetype(arch) &&)* true
            }
            unsafe fn get_item<'a>(fetch: Self::Fetch, row: usize) -> Self::Item<'a>
            where
                'w: 'a
            {
                let ($($t,)*) = fetch;
                ($($t::get_item($t, row),)*)
            }
            unsafe fn filter_row(fetch: Self::Fetch, row: usize, tick: u32) -> bool {
                let ($($t,)*) = fetch;
                $($t::filter_row($t, row, tick) &&)* true
            }
            unsafe fn get_slice<'a>(fetch: Self::Fetch, len: usize) -> Self::Slice<'a>
            where
                'w: 'a
            {
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

impl<'w, T: 'static> WorldQuery<'w> for With<T> {
    type StaticType = With<T>;
    type Fetch = ();
    type Item<'a> = () where 'w: 'a;
    type Slice<'a> = () where 'w: 'a;

    unsafe fn fetch_raw(_arch: &Archetype, _tick: u32) -> Option<Self::Fetch> {
        Some(())
    }

    fn check_aliasing(_types: &mut Vec<(TypeId, bool)>) {}

    fn matches_archetype(arch: &Archetype) -> bool {
        arch.has_component(TypeId::of::<T>())
    }

    unsafe fn filter_row(_fetch: Self::Fetch, _row: usize, _tick: u32) -> bool { true }
    unsafe fn get_item<'a>(_fetch: Self::Fetch, _row: usize) -> Self::Item<'a> where 'w: 'a { () }
    unsafe fn get_slice<'a>(_fetch: Self::Fetch, _len: usize) -> Self::Slice<'a> where 'w: 'a { () }
}

pub struct Without<T>(PhantomData<T>);

impl<'w, T: 'static> WorldQuery<'w> for Without<T> {
    type StaticType = Without<T>;
    type Fetch = ();
    type Item<'a> = () where 'w: 'a;
    type Slice<'a> = () where 'w: 'a;

    unsafe fn fetch_raw(_arch: &Archetype, _tick: u32) -> Option<Self::Fetch> {
        Some(())
    }

    fn check_aliasing(_types: &mut Vec<(TypeId, bool)>) {}

    fn matches_archetype(arch: &Archetype) -> bool {
        !arch.has_component(TypeId::of::<T>())
    }

    unsafe fn filter_row(_fetch: Self::Fetch, _row: usize, _tick: u32) -> bool { true }
    unsafe fn get_item<'a>(_fetch: Self::Fetch, _row: usize) -> Self::Item<'a> where 'w: 'a { () }
    unsafe fn get_slice<'a>(_fetch: Self::Fetch, _len: usize) -> Self::Slice<'a> where 'w: 'a { () }
}

pub struct Or<T1, T2>(PhantomData<(T1, T2)>);

impl<'w, T1: WorldQuery<'w>, T2: WorldQuery<'w>> WorldQuery<'w> for Or<T1, T2> {
    type StaticType = Or<T1::StaticType, T2::StaticType>;
    type Fetch = ();
    type Item<'a> = () where 'w: 'a;
    type Slice<'a> = () where 'w: 'a;

    unsafe fn fetch_raw(_arch: &Archetype, _tick: u32) -> Option<Self::Fetch> {
        Some(())
    }

    fn check_aliasing(_types: &mut Vec<(TypeId, bool)>) {}

    fn matches_archetype(arch: &Archetype) -> bool {
        T1::matches_archetype(arch) || T2::matches_archetype(arch)
    }

    unsafe fn filter_row(_fetch: Self::Fetch, _row: usize, _tick: u32) -> bool { true }
    unsafe fn get_item<'a>(_fetch: Self::Fetch, _row: usize) -> Self::Item<'a> where 'w: 'a { () }
    unsafe fn get_slice<'a>(_fetch: Self::Fetch, _len: usize) -> Self::Slice<'a> where 'w: 'a { () }
}
