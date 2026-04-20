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
    
    /// Sparse set'ten belirtilen entity'nin component'ını raw pointer ile getirir.
    ///
    /// # Safety
    /// - `entity` bu sparse set'te mevcut olmalıdır (kontrolü get_raw yapar, None döner).
    /// - Çağıran, aynı SparseSet'e eşzamanlı &mut erişim olmadığını garanti etmelidir.
    ///   Bu garanti `RefCell` guard'ları (Ref/RefMut) ve `check_aliasing` tarafından sağlanır.
    unsafe fn get_raw<'a>(fetch: &'a Self::Fetch, entity: u32) -> Option<Self::Item<'a>>;
    
    /// Bu component'ın dense array'indeki entity ID'lerini döndürür.
    fn iter_entities<'a>(fetch: &'a Self::Fetch) -> impl Iterator<Item = u32> + 'a;
}

impl<'w, T: Component> FetchComponent<'w> for &'w T {
    type Component = T;
    type Fetch = Ref<'w, SparseSet<T>>;
    type Item<'a> = &'a T where 'w: 'a;
    const IS_MUT: bool = false;

    fn fetch(world: &'w World) -> Option<Self::Fetch> { world.borrow::<T>().expect("ECS Aliasing Error: Component borrow conflict") }
    
    unsafe fn get_raw<'a>(fetch: &'a Self::Fetch, entity: u32) -> Option<Self::Item<'a>> {
        // SAFETY: Ref<SparseSet<T>> immutable borrow guarantee sağlar.
        // Dense array'e shared pointer alıp read-only erişim yapıyoruz.
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
    type Fetch = RefMut<'w, SparseSet<T>>;
    type Item<'a> = &'a mut T where 'w: 'a;
    const IS_MUT: bool = true;

    fn fetch(world: &'w World) -> Option<Self::Fetch> { world.borrow_mut::<T>().expect("ECS Aliasing Error: Component mutable borrow conflict") }
    
    unsafe fn get_raw<'a>(fetch: &'a Self::Fetch, entity: u32) -> Option<Self::Item<'a>> {
        // SAFETY: RefMut<SparseSet<T>> exclusive borrow guarantee sağlar —
        // başka hiçbir Ref veya RefMut aynı SparseSet'e aynı anda erişemez.
        // check_aliasing() aynı component tipinin query'de birden fazla
        // mutable olarak istenmesini runtime'da engelleyerek &mut aliasing'i önler.
        // Bu unsafe blok boyunca fetch yalnızca & olarak dereference edilir
        // (as_ptr çağrısı &self alır), dolayısıyla &mut fetch ve &fetch eşzamanlı
        // kullanılmaz.
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
    
    /// Immutable iterator — tüm query tipleri için çalışır.
    fn iter<'a>(fetch: &'a Self::Fetch) -> impl Iterator<Item = (u32, Self::Item<'a>)> + 'a
    where
        'w: 'a;

    /// Mutable iterator — `&mut T` içeren query'ler için gerekli.
    fn iter_mut<'a>(fetch: &'a mut Self::Fetch) -> impl Iterator<Item = (u32, Self::Item<'a>)> + 'a
    where
        'w: 'a;
}

// =========================================================================
// QUERY STRUCT
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

    /// Immutable iterator — read-only query'ler için tercih edilen yol.
    pub fn iter<'a>(&'a self) -> impl Iterator<Item = (u32, Q::Item<'a>)> + 'a
    where
        'w: 'a,
    {
        Q::iter(&self.fetch)
    }

    /// Mutable iterator — `&mut T` içeren query'ler için.
    /// Read-only query'lerde de çalışır (geriye dönük uyumluluk).
    pub fn iter_mut<'a>(&'a mut self) -> impl Iterator<Item = (u32, Q::Item<'a>)> + 'a
    where
        'w: 'a,
    {
        Q::iter_mut(&mut self.fetch)
    }
}

// =========================================================================
// ALIASING KONTROLÜ
// =========================================================================

/// Aynı Component tipinin query'de birden fazla mutable olarak istenmesini engeller.
/// `(&mut Transform, &Transform)` gibi bir query runtime panic üretir.
#[inline]
fn check(tid: TypeId, is_mut: bool, types: &mut Vec<(TypeId, bool)>) {
    if types.iter().any(|(t, m)| *t == tid && (*m || is_mut)) {
        panic!(
            "Aliasing UB: Bir Component aynı sorguda birden fazla kez mutable (&mut) olarak \
             istenemez veya hem mutable hem immutable olarak kullanılamaz!"
        );
    }
    types.push((tid, is_mut));
}

// =========================================================================
// TEKLİ QUERY IMPL — `Query<&Transform>` veya `Query<&mut Transform>`
// =========================================================================

impl<'w, T0: FetchComponent<'w>> WorldQuery<'w> for T0 {
    type Fetch = T0::Fetch;
    type Item<'a> = T0::Item<'a> where 'w: 'a;

    fn fetch(world: &'w World) -> Option<Self::Fetch> {
        T0::fetch(world)
    }

    fn check_aliasing(types: &mut Vec<(TypeId, bool)>) {
        check(TypeId::of::<T0::Component>(), T0::IS_MUT, types);
    }

    fn iter<'a>(fetch: &'a Self::Fetch) -> impl Iterator<Item = (u32, Self::Item<'a>)> + 'a
    where
        'w: 'a,
    {
        // SAFETY: fetch &'a olarak borrow edilmiş, get_raw &'a lifetime'ında item döndürür.
        // Immutable borrow — RefCell guard zaten shared access garanti eder.
        // IS_MUT = true ise RefMut guard exclusive access sağlar, iter() &self aldığı için
        // get_raw çağrısı sırasında fetch'e başka mutable erişim yoktur.
        let fetch_ref = fetch;
        T0::iter_entities(fetch_ref).filter_map(move |e| {
            unsafe { T0::get_raw(fetch_ref, e).map(|item| (e, item)) }
        })
    }

    fn iter_mut<'a>(fetch: &'a mut Self::Fetch) -> impl Iterator<Item = (u32, Self::Item<'a>)> + 'a
    where
        'w: 'a,
    {
        // SAFETY: fetch &'a mut olarak alınıp &'a olarak reborrow ediliyor.
        // Mutable referans bu noktadan sonra kullanılmıyor — sadece immutable ref iterator'a geçer.
        // RefMut guard exclusive access sağladığı için başka borrow mümkün değil.
        let fetch_ref = &*fetch;
        T0::iter_entities(fetch_ref).filter_map(move |e| {
            unsafe { T0::get_raw(fetch_ref, e).map(|item| (e, item)) }
        })
    }
}

// =========================================================================
// TUPLE QUERY MACRO
// =========================================================================

/// Tuple query'ler için WorldQuery implementasyonunu üretir.
/// Maksimum 12 component desteklenir. Daha fazlası gerekiyorsa
/// query'leri ayrı `world.query()` çağrılarına bölün.
macro_rules! impl_query_tuple {
    ($head:ident $(, $tail:ident)*) => {
        #[allow(non_snake_case)]
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

            fn iter<'a>(fetch: &'a Self::Fetch) -> impl Iterator<Item = (u32, Self::Item<'a>)> + 'a
            where
                'w: 'a,
            {
                #[allow(non_snake_case)]
                let ($head, $($tail,)*) = fetch;

                // SAFETY: Tüm fetch referansları &'a olarak alınıyor.
                // check_aliasing aynı SparseSet'e çoklu mutable erişimi engeller.
                // RefCell guard'ları (Ref/RefMut) exclusive veya shared borrow garanti eder.
                $head::iter_entities($head).filter_map(move |e| {
                    unsafe {
                        let h_item = $head::get_raw($head, e)?;
                        $( let $tail = $tail::get_raw($tail, e)?; )*
                        Some((e, (h_item, $($tail,)*)))
                    }
                })
            }

            fn iter_mut<'a>(fetch: &'a mut Self::Fetch) -> impl Iterator<Item = (u32, Self::Item<'a>)> + 'a
            where
                'w: 'a,
            {
                #[allow(non_snake_case)]
                let ($head, $($tail,)*) = fetch;
                
                // SAFETY: fetch &mut olarak destructure edildikten sonra her eleman
                // ayrı olarak &'a reborrow ediliyor. check_aliasing() query oluşturulurken
                // aynı tipte çoklu mutable erişimi engeller. RefMut guard'ları exclusive
                // access sağlar — başka borrow mümkün değil. Destructure sonrası
                // orijinal &mut artık kullanılmadığı için aliasing oluşmaz.
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
    use crate::impl_component;

    struct Position(f32, f32);
    struct Velocity(f32, f32);
    struct Health(u32);

    impl_component!(Position, Velocity, Health);

    #[test]
    fn test_single_query_iter() {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Position(1.0, 2.0));

        // iter() — immutable
        let query = world.query::<&Position>().expect("Failed to get query");
        let mut count = 0;
        for (id, pos) in query.iter() {
            assert_eq!(id, e.id());
            assert_eq!(pos.0, 1.0);
            count += 1;
        }
        assert_eq!(count, 1);
    }

    #[test]
    fn test_single_query_iter_mut() {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Position(1.0, 2.0));

        // iter_mut() — geriye dönük uyumluluk
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

        // Mutable modifikasyon
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

        // Doğrulama
        let query = world.query::<&Position>().unwrap();
        for (_, pos) in query.iter() {
            assert_eq!(pos.0, 6.0);
            assert_eq!(pos.1, 7.0);
        }
    }

    #[test]
    fn test_tuple_query_immutable_iter() {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Position(3.0, 4.0));
        world.add_component(e, Velocity(1.0, 2.0));

        // iter() — tuple, immutil
        let query = world.query::<(&Position, &Velocity)>().expect("Failed");
        let mut count = 0;
        for (_, (pos, vel)) in query.iter() {
            assert_eq!(pos.0, 3.0);
            assert_eq!(vel.0, 1.0);
            count += 1;
        }
        assert_eq!(count, 1);
    }

    #[test]
    #[should_panic(expected = "Aliasing UB")]
    fn test_aliasing_panic() {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Position(1.0, 2.0));

        // Aynı tip hem &mut hem & — panic olmalı
        let _query = world.query::<(&mut Position, &Position)>();
    }

    #[test]
    fn test_multiple_entities() {
        let mut world = World::new();
        for i in 0..10 {
            let e = world.spawn();
            world.add_component(e, Position(i as f32, 0.0));
        }

        let query = world.query::<&Position>().unwrap();
        let count = query.iter().count();
        assert_eq!(count, 10);
    }

    #[test]
    fn test_partial_match_filters() {
        let mut world = World::new();
        
        // Entity 1: Position + Velocity
        let e1 = world.spawn();
        world.add_component(e1, Position(1.0, 0.0));
        world.add_component(e1, Velocity(1.0, 0.0));
        
        // Entity 2: sadece Position
        let e2 = world.spawn();
        world.add_component(e2, Position(2.0, 0.0));

        // Tuple query — sadece ikisi de olan Entity 1 gelmeli
        let query = world.query::<(&Position, &Velocity)>().unwrap();
        let results: Vec<_> = query.iter().collect();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].1.0.0, 1.0); // e1'in Position'ı
    }
}
