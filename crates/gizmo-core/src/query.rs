use crate::component::{Component, SparseSet};
use crate::world::World;
use std::cell::{Ref, RefMut};

// ==============================================================
// 1 Bileşenli Sorgular
// ==============================================================

pub struct QueryMut<'a, T1: Component> {
    pub s1: RefMut<'a, SparseSet<T1>>,
}

impl<'a, T1: Component> QueryMut<'a, T1> {
    pub fn new(world: &'a World) -> Option<Self> {
        Some(Self {
            s1: world.borrow_mut::<T1>()?,
        })
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (u32, &mut T1)> {
        let s1 = &mut *self.s1;
        s1.dense
            .iter_mut()
            .map(|entry| (entry.entity, &mut entry.data))
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
        Some(Self {
            s1: world.borrow::<T1>()?,
        })
    }

    pub fn iter(&self) -> impl Iterator<Item = (u32, &T1)> {
        self.s1
            .dense
            .iter()
            .map(|entry| (entry.entity, &entry.data))
    }
}

impl<T1: Component> crate::system::SystemParam for QueryRef<'static, T1> {
    type Item<'a> = QueryRef<'a, T1>;
    fn fetch<'a>(world: &'a World, _dt: f32) -> Option<Self::Item<'a>> {
        QueryRef::new(world)
    }
}

// ==============================================================
// 2 Bileşenli Sorgular
// ==============================================================

pub struct QueryMutRef<'a, T1: Component, T2: Component> {
    pub s1: RefMut<'a, SparseSet<T1>>,
    pub s2: Ref<'a, SparseSet<T2>>,
}

impl<'a, T1: Component, T2: Component> QueryMutRef<'a, T1, T2> {
    pub fn new(world: &'a World) -> Option<Self> {
        Some(Self {
            s1: world.borrow_mut::<T1>()?,
            s2: world.borrow::<T2>()?,
        })
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (u32, &mut T1, &T2)> {
        let s2 = &self.s2; // lifetime capture
        let s1 = &mut *self.s1;
        s1.dense.iter_mut().filter_map(move |entry| {
            let e = entry.entity;
            let t1 = &mut entry.data;
            s2.get(e).map(|t2| (e, t1, t2))
        })
    }
}

impl<T1: Component, T2: Component> crate::system::SystemParam for QueryMutRef<'static, T1, T2> {
    type Item<'a> = QueryMutRef<'a, T1, T2>;
    fn fetch<'a>(world: &'a World, _dt: f32) -> Option<Self::Item<'a>> {
        QueryMutRef::new(world)
    }
}

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
        // Bunu safe Rust ile iter üzerinden yapmak zor zira s2'den mut ödünç almak filter_map içinde sıkıntı.
        // Ama Unsafe block ile çok kolayca çözülebilir. Burada Raw Pointer kullanıyoruz çünkü
        // ECS kurallarına göre bir entity sadece 1 s2 elemanına sahip olabilir, aliasing MÜMKÜN DEĞİLDİR.

        let s2_ptr = self.s2.dense.as_mut_ptr();
        let s2_sparse = &self.s2; // Sparse hashmap lookup için immutable

        let s1 = &mut *self.s1;
        s1.dense.iter_mut().filter_map(move |entry| {
            let e = entry.entity;
            let t1 = &mut entry.data;
            // Sparse üzerinden mut borrow almadan index bul, pointer aritmatiği ile &mut çevir
            // Bu kesinlikle Safe'dir çünkü her T1 için T2'ye olan e (Entity ID) UNIQUE'dır.
            if let Some(&index) = s2_sparse.sparse.get(&e) {
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

pub struct QueryRefRef<'a, T1: Component, T2: Component> {
    pub s1: Ref<'a, SparseSet<T1>>,
    pub s2: Ref<'a, SparseSet<T2>>,
}

impl<'a, T1: Component, T2: Component> QueryRefRef<'a, T1, T2> {
    pub fn new(world: &'a World) -> Option<Self> {
        Some(Self {
            s1: world.borrow::<T1>()?,
            s2: world.borrow::<T2>()?,
        })
    }

    pub fn iter(&self) -> impl Iterator<Item = (u32, &T1, &T2)> {
        let s2 = &self.s2;
        self.s1.dense.iter().filter_map(move |entry| {
            let e = entry.entity;
            let t1 = &entry.data;
            s2.get(e).map(|t2| (e, t1, t2))
        })
    }
}

// ==============================================================
// 3 Bileşenli Sorgular
// ==============================================================

pub struct QueryMutRefRef<'a, T1: Component, T2: Component, T3: Component> {
    pub s1: RefMut<'a, SparseSet<T1>>,
    pub s2: Ref<'a, SparseSet<T2>>,
    pub s3: Ref<'a, SparseSet<T3>>,
}

impl<'a, T1: Component, T2: Component, T3: Component> QueryMutRefRef<'a, T1, T2, T3> {
    pub fn new(world: &'a World) -> Option<Self> {
        Some(Self {
            s1: world.borrow_mut::<T1>()?,
            s2: world.borrow::<T2>()?,
            s3: world.borrow::<T3>()?,
        })
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (u32, &mut T1, &T2, &T3)> {
        let s2 = &self.s2;
        let s3 = &self.s3;
        let s1 = &mut *self.s1;
        s1.dense.iter_mut().filter_map(move |entry| {
            let e = entry.entity;
            let t1 = &mut entry.data;
            if let (Some(t2), Some(t3)) = (s2.get(e), s3.get(e)) {
                Some((e, t1, t2, t3))
            } else {
                None
            }
        })
    }
}

pub struct QueryRefRefRef<'a, T1: Component, T2: Component, T3: Component> {
    pub s1: Ref<'a, SparseSet<T1>>,
    pub s2: Ref<'a, SparseSet<T2>>,
    pub s3: Ref<'a, SparseSet<T3>>,
}

impl<'a, T1: Component, T2: Component, T3: Component> QueryRefRefRef<'a, T1, T2, T3> {
    pub fn new(world: &'a World) -> Option<Self> {
        Some(Self {
            s1: world.borrow::<T1>()?,
            s2: world.borrow::<T2>()?,
            s3: world.borrow::<T3>()?,
        })
    }

    pub fn iter(&self) -> impl Iterator<Item = (u32, &T1, &T2, &T3)> {
        let s2 = &self.s2;
        let s3 = &self.s3;
        self.s1.dense.iter().filter_map(move |entry| {
            let e = entry.entity;
            let t1 = &entry.data;
            if let (Some(t2), Some(t3)) = (s2.get(e), s3.get(e)) {
                Some((e, t1, t2, t3))
            } else {
                None
            }
        })
    }
}

// ==============================================================
// 4 Bileşenli Sorgular
// ==============================================================

pub struct QueryMutRefRefRef<'a, T1: Component, T2: Component, T3: Component, T4: Component> {
    pub s1: RefMut<'a, SparseSet<T1>>,
    pub s2: Ref<'a, SparseSet<T2>>,
    pub s3: Ref<'a, SparseSet<T3>>,
    pub s4: Ref<'a, SparseSet<T4>>,
}

impl<'a, T1: Component, T2: Component, T3: Component, T4: Component>
    QueryMutRefRefRef<'a, T1, T2, T3, T4>
{
    pub fn new(world: &'a World) -> Option<Self> {
        Some(Self {
            s1: world.borrow_mut::<T1>()?,
            s2: world.borrow::<T2>()?,
            s3: world.borrow::<T3>()?,
            s4: world.borrow::<T4>()?,
        })
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (u32, &mut T1, &T2, &T3, &T4)> {
        let s2 = &self.s2;
        let s3 = &self.s3;
        let s4 = &self.s4;
        let s1 = &mut *self.s1;
        s1.dense.iter_mut().filter_map(move |entry| {
            let e = entry.entity;
            let t1 = &mut entry.data;
            if let (Some(t2), Some(t3), Some(t4)) = (s2.get(e), s3.get(e), s4.get(e)) {
                Some((e, t1, t2, t3, t4))
            } else {
                None
            }
        })
    }
}

pub struct QueryRefRefRefRef<'a, T1: Component, T2: Component, T3: Component, T4: Component> {
    pub s1: Ref<'a, SparseSet<T1>>,
    pub s2: Ref<'a, SparseSet<T2>>,
    pub s3: Ref<'a, SparseSet<T3>>,
    pub s4: Ref<'a, SparseSet<T4>>,
}

impl<'a, T1: Component, T2: Component, T3: Component, T4: Component>
    QueryRefRefRefRef<'a, T1, T2, T3, T4>
{
    pub fn new(world: &'a World) -> Option<Self> {
        Some(Self {
            s1: world.borrow::<T1>()?,
            s2: world.borrow::<T2>()?,
            s3: world.borrow::<T3>()?,
            s4: world.borrow::<T4>()?,
        })
    }

    pub fn iter(&self) -> impl Iterator<Item = (u32, &T1, &T2, &T3, &T4)> {
        let s2 = &self.s2;
        let s3 = &self.s3;
        let s4 = &self.s4;
        self.s1.dense.iter().filter_map(move |entry| {
            let e = entry.entity;
            let t1 = &entry.data;
            if let (Some(t2), Some(t3), Some(t4)) = (s2.get(e), s3.get(e), s4.get(e)) {
                Some((e, t1, t2, t3, t4))
            } else {
                None
            }
        })
    }
}

// ==============================================================
// 5 Bileşenli Sorgular
// ==============================================================

pub struct QueryMutRefRefRefRef<
    'a,
    T1: Component,
    T2: Component,
    T3: Component,
    T4: Component,
    T5: Component,
> {
    pub s1: RefMut<'a, SparseSet<T1>>,
    pub s2: Ref<'a, SparseSet<T2>>,
    pub s3: Ref<'a, SparseSet<T3>>,
    pub s4: Ref<'a, SparseSet<T4>>,
    pub s5: Ref<'a, SparseSet<T5>>,
}

impl<'a, T1: Component, T2: Component, T3: Component, T4: Component, T5: Component>
    QueryMutRefRefRefRef<'a, T1, T2, T3, T4, T5>
{
    pub fn new(world: &'a World) -> Option<Self> {
        Some(Self {
            s1: world.borrow_mut::<T1>()?,
            s2: world.borrow::<T2>()?,
            s3: world.borrow::<T3>()?,
            s4: world.borrow::<T4>()?,
            s5: world.borrow::<T5>()?,
        })
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (u32, &mut T1, &T2, &T3, &T4, &T5)> {
        let s2 = &self.s2;
        let s3 = &self.s3;
        let s4 = &self.s4;
        let s5 = &self.s5;
        let s1 = &mut *self.s1;
        s1.dense.iter_mut().filter_map(move |entry| {
            let e = entry.entity;
            let t1 = &mut entry.data;
            if let (Some(t2), Some(t3), Some(t4), Some(t5)) =
                (s2.get(e), s3.get(e), s4.get(e), s5.get(e))
            {
                Some((e, t1, t2, t3, t4, t5))
            } else {
                None
            }
        })
    }
}

pub struct QueryRefRefRefRefRef<
    'a,
    T1: Component,
    T2: Component,
    T3: Component,
    T4: Component,
    T5: Component,
> {
    pub s1: Ref<'a, SparseSet<T1>>,
    pub s2: Ref<'a, SparseSet<T2>>,
    pub s3: Ref<'a, SparseSet<T3>>,
    pub s4: Ref<'a, SparseSet<T4>>,
    pub s5: Ref<'a, SparseSet<T5>>,
}

impl<'a, T1: Component, T2: Component, T3: Component, T4: Component, T5: Component>
    QueryRefRefRefRefRef<'a, T1, T2, T3, T4, T5>
{
    pub fn new(world: &'a World) -> Option<Self> {
        Some(Self {
            s1: world.borrow::<T1>()?,
            s2: world.borrow::<T2>()?,
            s3: world.borrow::<T3>()?,
            s4: world.borrow::<T4>()?,
            s5: world.borrow::<T5>()?,
        })
    }

    pub fn iter(&self) -> impl Iterator<Item = (u32, &T1, &T2, &T3, &T4, &T5)> {
        let s2 = &self.s2;
        let s3 = &self.s3;
        let s4 = &self.s4;
        let s5 = &self.s5;
        self.s1.dense.iter().filter_map(move |entry| {
            let e = entry.entity;
            let t1 = &entry.data;
            if let (Some(t2), Some(t3), Some(t4), Some(t5)) =
                (s2.get(e), s3.get(e), s4.get(e), s5.get(e))
            {
                Some((e, t1, t2, t3, t4, t5))
            } else {
                None
            }
        })
    }
}
