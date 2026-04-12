use crate::component::{Component, SparseSet};
use crate::world::World;
use std::cell::{Ref, RefMut};

// ─── Tek Bileşenli Sorgular ───────────────────────────────────────────────────
//
// QueryMut<T>  — T'yi değiştirilebilir olarak sorgular
// QueryRef<T>  — T'yi salt okunur olarak sorgular
//
// Bu iki tür özel-durumludur; aşağıdaki makrolar N ≥ 2 için kullanılır.

pub struct QueryMut<'a, T1: Component> {
    pub s1: RefMut<'a, SparseSet<T1>>,
}

impl<'a, T1: Component> QueryMut<'a, T1> {
    pub fn new(world: &'a World) -> Option<Self> {
        Some(Self { s1: world.borrow_mut::<T1>()? })
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (u32, &mut T1)> {
        let s1 = &mut *self.s1;
        s1.dense.iter_mut().map(|entry| (entry.entity, &mut entry.data))
    }
}

impl<T1: Component> crate::system::SystemParam for QueryMut<'static, T1> {
    type Item<'a> = QueryMut<'a, T1>;
    fn fetch<'a>(world: &'a World, _dt: f32) -> Option<Self::Item<'a>> {
        QueryMut::new(world)
    }
}

pub struct QueryRef<'a, T1: Component> {
    pub s1: Ref<'a, SparseSet<T1>>,
}

impl<'a, T1: Component> QueryRef<'a, T1> {
    pub fn new(world: &'a World) -> Option<Self> {
        Some(Self { s1: world.borrow::<T1>()? })
    }

    pub fn iter(&self) -> impl Iterator<Item = (u32, &T1)> {
        self.s1.dense.iter().map(|entry| (entry.entity, &entry.data))
    }
}

impl<T1: Component> crate::system::SystemParam for QueryRef<'static, T1> {
    type Item<'a> = QueryRef<'a, T1>;
    fn fetch<'a>(world: &'a World, _dt: f32) -> Option<Self::Item<'a>> {
        QueryRef::new(world)
    }
}

// ─── Özel-Durum: QueryMutMut ─────────────────────────────────────────────────
//
// İki bileşeni aynı anda değiştirilebilir sorgular.
// Aliasing UB'yi önlemek için farklı TypeId kontrolü yapılır.
// iter_mut, iki ayrı SparseSet'e ham pointer ile erişir (ECS'de aliasing imkânsız).

pub struct QueryMutMut<'a, T1: Component, T2: Component> {
    pub s1: RefMut<'a, SparseSet<T1>>,
    pub s2: RefMut<'a, SparseSet<T2>>,
}

impl<'a, T1: Component, T2: Component> QueryMutMut<'a, T1, T2> {
    pub fn new(world: &'a World) -> Option<Self> {
        if std::any::TypeId::of::<T1>() == std::any::TypeId::of::<T2>() {
            panic!("QueryMutMut aliasing UB: T1 and T2 cannot be the same type!");
        }
        Some(Self {
            s1: world.borrow_mut::<T1>()?,
            s2: world.borrow_mut::<T2>()?,
        })
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (u32, &mut T1, &mut T2)> {
        let s2_ptr    = self.s2.dense.as_mut_ptr();
        let s2_sparse = &self.s2;
        let s1        = &mut *self.s1;
        s1.dense.iter_mut().filter_map(move |entry| {
            let e  = entry.entity;
            let t1 = &mut entry.data;
            if let Some(&index) = s2_sparse.sparse.get(&e) {
                // SAFETY: T1 ≠ T2 garanti, ECS'de her entity ayrı storage → aliasing yok.
                let t2 = unsafe { &mut (*s2_ptr.add(index)).data };
                Some((e, t1, t2))
            } else {
                None
            }
        })
    }
}

impl<T1: Component, T2: Component> crate::system::SystemParam for QueryMutMut<'static, T1, T2> {
    type Item<'a> = QueryMutMut<'a, T1, T2>;
    fn fetch<'a>(world: &'a World, _dt: f32) -> Option<Self::Item<'a>> {
        QueryMutMut::new(world)
    }
}

// ─── Çok-Bileşenli Sorgular (Makro ile üretilir) ─────────────────────────────
//
// impl_query_mut_refs! — İlk bileşen mut, geri kalanlar ref:
//   impl_query_mut_refs!(QueryMutRef,     s1:mut T1, s2:ref T2);
//   impl_query_mut_refs!(QueryMutRefRef,  s1:mut T1, s2:ref T2, s3:ref T3);
//   ...
//
// impl_query_refs! — Tümü ref:
//   impl_query_refs!(QueryRefRef,      s1:T1, s2:T2);
//   impl_query_refs!(QueryRefRefRef,   s1:T1, s2:T2, s3:T3);
//   ...
//
// Yeni kombinasyon eklemek için tek satır yeterli.

/// İlk bileşen `&mut`, geri kalanlar `&` olan N-bileşen sorgu struct'ı üretir.
macro_rules! impl_query_mut_refs {
    // Giriş: struct adı, mut alan tanımı, ardından bir veya daha fazla ref alan tanımı
    ($name:ident, $s1:ident : mut $T1:ident, $( $si:ident : ref $Ti:ident ),+) => {
        pub struct $name<'a, $T1: Component, $( $Ti: Component ),+> {
            pub $s1: RefMut<'a, SparseSet<$T1>>,
            $( pub $si: Ref<'a, SparseSet<$Ti>>, )+
        }

        impl<'a, $T1: Component, $( $Ti: Component ),+> $name<'a, $T1, $( $Ti ),+> {
            pub fn new(world: &'a World) -> Option<Self> {
                Some(Self {
                    $s1: world.borrow_mut::<$T1>()?,
                    $( $si: world.borrow::<$Ti>()?, )+
                })
            }

            pub fn iter_mut(
                &mut self,
            ) -> impl Iterator<Item = (u32, &mut $T1, $( &$Ti ),+)> {
                $( let $si = &self.$si; )+
                let $s1 = &mut *self.$s1;
                $s1.dense.iter_mut().filter_map(move |entry| {
                    let e  = entry.entity;
                    let t1 = &mut entry.data;
                    // Tüm ref bileşenlerin bu entity'de var olması gerekir
                    if let ( $( Some($si) ),+ ) = ( $( $si.get(e) ),+ ) {
                        Some((e, t1, $( $si ),+))
                    } else {
                        None
                    }
                })
            }
        }
    };
}

/// Tüm bileşenleri `&` (immutable) olarak sorgulayan N-bileşen struct'ı üretir.
macro_rules! impl_query_refs {
    ($name:ident, $s1:ident : $T1:ident, $( $si:ident : $Ti:ident ),+) => {
        pub struct $name<'a, $T1: Component, $( $Ti: Component ),+> {
            pub $s1: Ref<'a, SparseSet<$T1>>,
            $( pub $si: Ref<'a, SparseSet<$Ti>>, )+
        }

        impl<'a, $T1: Component, $( $Ti: Component ),+> $name<'a, $T1, $( $Ti ),+> {
            pub fn new(world: &'a World) -> Option<Self> {
                Some(Self {
                    $s1: world.borrow::<$T1>()?,
                    $( $si: world.borrow::<$Ti>()?, )+
                })
            }

            pub fn iter(
                &self,
            ) -> impl Iterator<Item = (u32, &$T1, $( &$Ti ),+)> {
                $( let $si = &self.$si; )+
                self.$s1.dense.iter().filter_map(move |entry| {
                    let e  = entry.entity;
                    let t1 = &entry.data;
                    if let ( $( Some($si) ),+ ) = ( $( $si.get(e) ),+ ) {
                        Some((e, t1, $( $si ),+))
                    } else {
                        None
                    }
                })
            }
        }
    };
}

// ─── 2 Bileşenli ─────────────────────────────────────────────────────────────
impl_query_mut_refs!(QueryMutRef,  s1:mut T1, s2:ref T2);
impl_query_refs!   (QueryRefRef,   s1:T1,     s2:T2);

// ─── 3 Bileşenli ─────────────────────────────────────────────────────────────
impl_query_mut_refs!(QueryMutRefRef,  s1:mut T1, s2:ref T2, s3:ref T3);
impl_query_refs!   (QueryRefRefRef,   s1:T1,     s2:T2,     s3:T3);

// ─── 4 Bileşenli ─────────────────────────────────────────────────────────────
impl_query_mut_refs!(QueryMutRefRefRef,  s1:mut T1, s2:ref T2, s3:ref T3, s4:ref T4);
impl_query_refs!   (QueryRefRefRefRef,   s1:T1,     s2:T2,     s3:T3,     s4:T4);

// ─── 5 Bileşenli ─────────────────────────────────────────────────────────────
impl_query_mut_refs!(QueryMutRefRefRefRef,  s1:mut T1, s2:ref T2, s3:ref T3, s4:ref T4, s5:ref T5);
impl_query_refs!   (QueryRefRefRefRefRef,   s1:T1,     s2:T2,     s3:T3,     s4:T4,     s5:T5);

// ─── 6 Bileşenli — Makroyla eklendi, önceden el yazımıyla imkânsızdı ─────────
impl_query_mut_refs!(QueryMutRefRefRefRefRef,  s1:mut T1, s2:ref T2, s3:ref T3, s4:ref T4, s5:ref T5, s6:ref T6);
impl_query_refs!   (QueryRefRefRefRefRefRef,   s1:T1,     s2:T2,     s3:T3,     s4:T4,     s5:T5,     s6:T6);
