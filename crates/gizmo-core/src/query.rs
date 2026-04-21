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
    
    const IS_MUT: bool;

    /// Bir archetype bazında ham pointer fetch hazırlar.
    unsafe fn fetch_raw(arch: &Archetype) -> Option<Self::Fetch>;
    
    /// Ham pointer'dan veriyi getirir.
    unsafe fn get_item<'a>(fetch: Self::Fetch, row: usize) -> Self::Item<'a>
    where
        'w: 'a;
}

impl<'w, T: 'static> FetchComponent<'w> for &'w T {
    type Component = T;
    type Fetch = *const u8;
    type Item<'a> = &'a T where 'w: 'a;
    const IS_MUT: bool = false;

    unsafe fn fetch_raw(arch: &Archetype) -> Option<Self::Fetch> {
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
}

impl<'w, T: 'static> FetchComponent<'w> for &'w mut T {
    type Component = T;
    type Fetch = *mut u8;
    type Item<'a> = &'a mut T where 'w: 'a;
    const IS_MUT: bool = true;

    unsafe fn fetch_raw(arch: &Archetype) -> Option<Self::Fetch> {
        let col = arch.get_column_mut(TypeId::of::<T>())?;
        Some(col.data_ptr_mut())
    }
    
    unsafe fn get_item<'a>(fetch: Self::Fetch, row: usize) -> Self::Item<'a>
    where
        'w: 'a
    {
        let ptr = fetch.add(row * std::mem::size_of::<T>()) as *mut T;
        &mut *ptr
    }
}

// =========================================================================
// WORLD QUERY TRAIT
// =========================================================================

pub trait WorldQuery<'w> {
    type Fetch: Copy;
    type Item<'a> where 'w: 'a;

    unsafe fn fetch_raw(arch: &Archetype) -> Option<Self::Fetch>;
    fn check_aliasing(types: &mut Vec<(TypeId, bool)>);
    fn required_types() -> Vec<TypeId>;
    unsafe fn get_item<'a>(fetch: Self::Fetch, row: usize) -> Self::Item<'a>
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
        let required = Q::required_types();
        let matching = world.archetype_index.matching_archetypes_readonly(&required);
        Some(Self { world, matching_archetypes: matching, _marker: PhantomData })
    }

    pub fn new_cached(world: &'w mut World) -> Option<Self> {
        let mut used_types = Vec::new();
        Q::check_aliasing(&mut used_types);
        let required = Q::required_types();
        let matching = world.archetype_index.matching_archetypes(&required).to_vec();
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

    #[inline]
    pub fn get(&self, entity_id: u32) -> Option<Q::Item<'_>> {
        let loc = self.world.entity_location(entity_id);
        if !loc.is_valid() { return None; }
        let arch = &self.world.archetype_index.archetypes[loc.archetype_id as usize];
        unsafe {
            let fetch = Q::fetch_raw(arch)?;
            Some(Q::get_item(fetch, loc.row as usize))
        }
    }

    #[inline]
    pub fn entity_count(&self) -> usize {
        self.matching_archetypes.iter()
            .map(|&idx| self.world.archetype_index.archetypes[idx].len())
            .sum()
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
                    let f = unsafe { Q::fetch_raw(arch)? };
                    self.current_fetch = Some(f);
                    self.current_row = 0;
                    f
                }
            };

            if self.current_row < arch.len() {
                let id = arch.entities()[self.current_row];
                let item = unsafe { Q::get_item(fetch, self.current_row) };
                self.current_row += 1;
                return Some((id, item));
            }

            self.current_fetch = None;
            self.current_arch_idx += 1;
        }
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
    type Fetch = T0::Fetch;
    type Item<'a> = T0::Item<'a> where 'w: 'a;
    unsafe fn fetch_raw(arch: &Archetype) -> Option<Self::Fetch> { T0::fetch_raw(arch) }
    fn check_aliasing(types: &mut Vec<(TypeId, bool)>) { check(TypeId::of::<T0::Component>(), T0::IS_MUT, types); }
    fn required_types() -> Vec<TypeId> { vec![TypeId::of::<T0::Component>()] }
    unsafe fn get_item<'a>(fetch: Self::Fetch, row: usize) -> Self::Item<'a>
    where
        'w: 'a
    {
        T0::get_item(fetch, row)
    }
}

macro_rules! impl_query_tuple {
    ($($t:ident),*) => {
        #[allow(non_snake_case)]
        impl<'w, $($t: FetchComponent<'w>),*> WorldQuery<'w> for ($($t,)*) {
            type Fetch = ($($t::Fetch,)*);
            type Item<'a> = ($($t::Item<'a>,)*) where 'w: 'a;
            unsafe fn fetch_raw(arch: &Archetype) -> Option<Self::Fetch> {
                Some(($($t::fetch_raw(arch)?,)*))
            }
            fn check_aliasing(types: &mut Vec<(TypeId, bool)>) {
                $(check(TypeId::of::<$t::Component>(), $t::IS_MUT, types);)*
            }
            fn required_types() -> Vec<TypeId> {
                vec![$(TypeId::of::<$t::Component>()),*]
            }
            unsafe fn get_item<'a>(fetch: Self::Fetch, row: usize) -> Self::Item<'a>
            where
                'w: 'a
            {
                let ($($t,)*) = fetch;
                ($($t::get_item($t, row),)*)
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
