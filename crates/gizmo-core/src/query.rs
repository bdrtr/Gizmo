#![allow(non_snake_case)]

use crate::component::{Component, SparseSet, DenseEntry};
use crate::world::World;
use std::cell::{Ref, RefMut};
use std::any::TypeId;

// =========================================================================
// FETCH COMPONENT TRAIT — Tek bir component tipine ref/mut erişimi soyutlar
// =========================================================================

pub trait FetchComponent<'w> {
    type Component: Component;
    type Fetch: 'w;
    type Item<'a> where 'w: 'a;
    
    const IS_MUT: bool;

    fn fetch(world: &'w World) -> Option<Self::Fetch>;
    
    /// Unsafe: gets the component from the raw sparse set without borrow checker ties.
    /// Caller must ensure `entity` exists and aliasing rules are met.
    unsafe fn get_raw<'a>(fetch: &'a Self::Fetch, entity: u32) -> Option<Self::Item<'a>>;
    
    /// Hızlı iterasyon için bu componentin dizisinden entity ID'lerini verir
    fn iter_entities<'a>(fetch: &'a Self::Fetch) -> impl Iterator<Item = u32> + 'a;
}

impl<'w, T: Component> FetchComponent<'w> for &'w T {
    type Component = T;
    type Fetch = Ref<'w, SparseSet<T>>;
    type Item<'a> = &'a T where 'w: 'a;
    const IS_MUT: bool = false;

    fn fetch(world: &'w World) -> Option<Self::Fetch> { world.borrow::<T>() }
    
    unsafe fn get_raw<'a>(fetch: &'a Self::Fetch, entity: u32) -> Option<Self::Item<'a>> {
        let ptr = fetch.dense.as_ptr();
        if let Some(&idx) = fetch.sparse.get(&entity) {
            Some(&(*ptr.add(idx)).data)
        } else {
            None
        }
    }

    fn iter_entities<'a>(fetch: &'a Self::Fetch) -> impl Iterator<Item = u32> + 'a {
        fetch.dense.iter().map(|e| e.entity)
    }
}

impl<'w, T: Component> FetchComponent<'w> for &'w mut T {
    type Component = T;
    // World'den mut olarak istiyoruz
    type Fetch = RefMut<'w, SparseSet<T>>;
    type Item<'a> = &'a mut T where 'w: 'a;
    const IS_MUT: bool = true;

    fn fetch(world: &'w World) -> Option<Self::Fetch> { world.borrow_mut::<T>() }
    
    unsafe fn get_raw<'a>(fetch: &'a Self::Fetch, entity: u32) -> Option<Self::Item<'a>> {
        let ptr = fetch.dense.as_ptr() as *mut DenseEntry<T>;
        if let Some(&idx) = fetch.sparse.get(&entity) {
            Some(&mut (*ptr.add(idx)).data)
        } else {
            None
        }
    }

    fn iter_entities<'a>(fetch: &'a Self::Fetch) -> impl Iterator<Item = u32> + 'a {
        fetch.dense.iter().map(|e| e.entity)
    }
}

// =========================================================================
// WORLD QUERY TRAIT
// =========================================================================

pub trait WorldQuery<'w> {
    type Fetch: 'w;
    type Item<'a> where 'w: 'a;

    fn fetch(world: &'w World) -> Option<Self::Fetch>;
    fn check_aliasing(types: &mut Vec<(TypeId, bool)>);
    
    fn iter_mut<'a>(fetch: &'a mut Self::Fetch) -> impl Iterator<Item = (u32, Self::Item<'a>)> + 'a
    where
        'w: 'a;
}

// =========================================================================
// QUERY STRUCT VE SYSTEM PARAM
// =========================================================================

pub struct Query<'w, Q: WorldQuery<'w>> {
    fetch: Q::Fetch,
}

impl<'w, Q: WorldQuery<'w>> Query<'w, Q> {
    pub fn new(world: &'w World) -> Option<Self> {
        let mut used_types = Vec::new();
        Q::check_aliasing(&mut used_types);

        Some(Self {
            fetch: Q::fetch(world)?,
        })
    }

    pub fn iter_mut<'a>(&'a mut self) -> impl Iterator<Item = (u32, Q::Item<'a>)> + 'a
    where
        'w: 'a,
    {
        Q::iter_mut(&mut self.fetch)
    }
}



// =========================================================================
// TUPLE MACRO
// =========================================================================

#[inline]
fn check(tid: TypeId, is_mut: bool, types: &mut Vec<(TypeId, bool)>) {
    if types.iter().any(|(t, m)| *t == tid && (*m || is_mut)) {
        panic!("Aliasing UB: Bir Component aynı sorguda birden fazla kez mutable (&mut) olarak istenemez!");
    }
    types.push((tid, is_mut));
}

// Tekli öğeler için (Tuple olmadan `Query<&Transform>`)
impl<'w, T0: FetchComponent<'w>> WorldQuery<'w> for T0 {
    type Fetch = T0::Fetch;
    type Item<'a> = T0::Item<'a> where 'w: 'a;

    fn fetch(world: &'w World) -> Option<Self::Fetch> {
        T0::fetch(world)
    }

    fn check_aliasing(types: &mut Vec<(TypeId, bool)>) {
        check(TypeId::of::<T0::Component>(), T0::IS_MUT, types);
    }

    fn iter_mut<'a>(fetch: &'a mut Self::Fetch) -> impl Iterator<Item = (u32, Self::Item<'a>)> + 'a
    where
        'w: 'a,
    {
        let fetch_ref = &*fetch;
        T0::iter_entities(fetch_ref).filter_map(move |e| {
            unsafe { T0::get_raw(fetch_ref, e).map(|item| (e, item)) }
        })
    }
}

macro_rules! impl_query_tuple {
    ($head:ident $(, $tail:ident)*) => {
        impl<'w, $head: FetchComponent<'w>, $($tail: FetchComponent<'w>),*> WorldQuery<'w> for ($head, $($tail,)*) {
            type Fetch = ($head::Fetch, $($tail::Fetch,)*);
            type Item<'a> = ($head::Item<'a>, $($tail::Item<'a>,)*) where 'w: 'a;

            fn fetch(world: &'w World) -> Option<Self::Fetch> {
                Some((
                    $head::fetch(world)?,
                    $($tail::fetch(world)?,)*
                ))
            }

            fn check_aliasing(types: &mut Vec<(TypeId, bool)>) {
                check(TypeId::of::<$head::Component>(), $head::IS_MUT, types);
                $( check(TypeId::of::<$tail::Component>(), $tail::IS_MUT, types); )*
            }

            fn iter_mut<'a>(fetch: &'a mut Self::Fetch) -> impl Iterator<Item = (u32, Self::Item<'a>)> + 'a
            where
                'w: 'a,
            {
                #[allow(non_snake_case)]
                let ($head, $($tail,)*) = fetch;
                
                let head_ref = &*$head;
                $( let $tail = &*$tail; )*
                
                $head::iter_entities(head_ref).filter_map(move |e| {
                    unsafe {
                        let h_item = $head::get_raw(head_ref, e)?;
                        $( let $tail = $tail::get_raw($tail, e)?; )*
                        Some((e, (h_item, $($tail,)*)))
                    }
                })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::World;
    use crate::Component;

    struct Position(f32, f32);

    struct Velocity(f32, f32);

    #[test]
    fn test_single_query() {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Position(1.0, 2.0));

        let mut query = world.query::<&Position>().expect("Failed to get query");
        let mut count = 0;
        for (id, pos) in query.iter_mut() {
            assert_eq!(id, e.id());
            assert_eq!(pos.0, 1.0);
            count += 1;
        }
        assert_eq!(count, 1);
    }

    #[test]
    fn test_tuple_query_mut() {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Position(1.0, 2.0));
        world.add_component(e, Velocity(5.0, 5.0));

        // Let's modify position based on velocity
        let mut query = world.query::<(&mut Position, &Velocity)>().expect("Failed");
        let mut count = 0;
        for (id, (pos, vel)) in query.iter_mut() {
            assert_eq!(id, e.id());
            pos.0 += vel.0;
            pos.1 += vel.1;
            count += 1;
        }
        assert_eq!(count, 1);
        drop(query);

        // Verify mutation
        let mut verify_query = world.query::<&Position>().unwrap();
        for (_, pos) in verify_query.iter_mut() {
            assert_eq!(pos.0, 6.0);
            assert_eq!(pos.1, 7.0);
        }
    }
}
