use super::World;
use crate::component::Component;

impl World {
    // ==========================================================
    // ERGONOMİK SORGULAR (QUERY API)
    // ==========================================================

    /// Salt-okunur bir [`Query`](crate::query::Query) oluşturur (paylaşımlı component erişimi).
    ///
    /// `Q: ReadOnlyQuery` bağlı olduğundan bu giriş noktası `&mut T` ÜRETEMEZ — `&self`'ten
    /// istenildiği kadar oluşturulabilir, hepsi aynı anda canlı olabilir, hiçbiri UB değildir.
    /// Mutable erişim için [`World::query_mut`] (`&mut World` ister; güvenli) veya — paralel
    /// scheduler içindeki sistemler için — [`World::query_unchecked`] (`unsafe`) kullanın.
    ///
    /// Bu ayrım, denetimin "tek en zayıf noktası" olan dual-`Mut` aliasing UB'sini **güvenli
    /// koddan ULAŞILAMAZ** kılar: `&World`'ten mutable query yalnızca `unsafe` ile alınır.
    ///
    /// # Examples
    /// Shared reads compose freely:
    /// ```
    /// use gizmo_core::prelude::*;
    /// #[derive(Clone)]
    /// struct Position { x: f32 }
    /// gizmo_core::impl_component!(Position);
    ///
    /// let mut world = World::new();
    /// world.register_component_type::<Position>();
    /// let e = world.spawn();
    /// world.add_component(e, Position { x: 1.0 });
    ///
    /// let r1 = world.query::<&Position>().unwrap();
    /// let r2 = world.query::<&Position>().unwrap(); // any number may coexist
    /// assert_eq!(r1.get(e.id()).unwrap().x, 1.0);
    /// assert_eq!(r2.get(e.id()).unwrap().x, 1.0);
    /// ```
    ///
    /// A *mutable* query can NOT be built through `query` — `Mut<T>` is not
    /// [`ReadOnlyQuery`](crate::query::ReadOnlyQuery), so the dual-`Mut` UB is unreachable
    /// from safe code (use [`World::query_mut`] instead):
    /// ```compile_fail
    /// use gizmo_core::prelude::*;
    /// #[derive(Clone)]
    /// struct Position { x: f32 }
    /// gizmo_core::impl_component!(Position);
    ///
    /// let world = World::new();
    /// // error[E0277]: `Mut<Position>: ReadOnlyQuery` is not satisfied
    /// let _q = world.query::<Mut<Position>>();
    /// ```
    pub fn query<'w, Q: crate::query::ReadOnlyQuery>(
        &'w self,
    ) -> Option<crate::query::Query<'w, Q>> {
        crate::query::Query::new(self)
    }

    /// Mutable bir [`Query`](crate::query::Query) oluşturur. `&mut self` aldığından dönen
    /// query World'ü ÖZEL olarak ödünç alır → ikinci bir (mutable VEYA okuma) query aynı anda
    /// derlenemez. Bu, iki canlı `Mut` query'sinin aynı belleği alias'lamasını tip düzeyinde
    /// imkânsız kılan güvenli yoldur.
    ///
    /// World'e özel erişimi olan uygulama kodu (oyun döngüsü, editör, exclusive sistemler)
    /// için tercih edilen mutable giriş noktasıdır.
    ///
    /// # Examples
    /// ```
    /// use gizmo_core::prelude::*;
    /// #[derive(Clone)]
    /// struct Position { x: f32 }
    /// gizmo_core::impl_component!(Position);
    ///
    /// let mut world = World::new();
    /// world.register_component_type::<Position>();
    /// let e = world.spawn();
    /// world.add_component(e, Position { x: 1.0 });
    ///
    /// {
    ///     let mut q = world.query_mut::<Mut<Position>>().unwrap();
    ///     for (_id, mut p) in q.iter_mut() { p.x += 1.0; }
    /// }
    /// assert_eq!(world.query::<&Position>().unwrap().get(e.id()).unwrap().x, 2.0);
    /// ```
    ///
    /// Two simultaneous mutable queries can't exist — each ties up `&mut World`, so the
    /// dual-`Mut` aliasing is rejected at compile time:
    /// ```compile_fail
    /// use gizmo_core::prelude::*;
    /// #[derive(Clone)]
    /// struct Position { x: f32 }
    /// gizmo_core::impl_component!(Position);
    ///
    /// let mut world = World::new();
    /// let q1 = world.query_mut::<Mut<Position>>();
    /// let q2 = world.query_mut::<Mut<Position>>(); // second &mut World — E0499
    /// let _ = (q1, q2);
    /// ```
    ///
    /// Likewise, two live mutable views from ONE query can't coexist (`get_mut` borrows the
    /// query exclusively):
    /// ```compile_fail
    /// use gizmo_core::prelude::*;
    /// #[derive(Clone)]
    /// struct Position { x: f32 }
    /// gizmo_core::impl_component!(Position);
    ///
    /// let mut world = World::new();
    /// world.register_component_type::<Position>();
    /// let mut q = world.query_mut::<Mut<Position>>().unwrap();
    /// let a = q.get_mut(0);
    /// let b = q.get_mut(0); // second &mut borrow of `q` — E0499
    /// let _ = (a, b);
    /// ```
    ///
    /// The shared accessors (`iter`/`get`/…) are gated to read-only queries, so a mutable
    /// query can't hand out an aliasable shared iterator either — use `iter_mut`:
    /// ```compile_fail
    /// use gizmo_core::prelude::*;
    /// #[derive(Clone)]
    /// struct Position { x: f32 }
    /// gizmo_core::impl_component!(Position);
    ///
    /// let mut world = World::new();
    /// let q = world.query_mut::<Mut<Position>>().unwrap();
    /// let _it = q.iter(); // `iter` requires `Q: ReadOnlyQuery`; Mut<Position> isn't — E0599
    /// ```
    pub fn query_mut<'w, Q: crate::query::WorldQuery>(
        &'w mut self,
    ) -> Option<crate::query::Query<'w, Q>> {
        crate::query::Query::new(self)
    }

    /// `&World`'ten mutable bir query oluşturan KAÇIŞ KAPISI. Paralel scheduler içindeki
    /// sistemler (`System::run(&World)`) için — onların `&mut World`'ü yoktur ama disjoint
    /// erişimleri `AccessInfo`/`is_compatible_with` tarafından zamanlama anında doğrulanır.
    ///
    /// # Safety
    /// Çağıran, bu query'nin canlı olduğu süre boyunca, AYNI component'lere mutable dokunan
    /// başka HİÇBİR query'nin (bu World üzerinde, bu veya başka bir thread'de) canlı
    /// olmamasını GARANTİ etmelidir. Motorda bu garanti şuralardan gelir:
    /// - paralel batch'lerde her sistemin `AccessInfo`'su `is_compatible_with` ile
    ///   çakışmayacak şekilde gruplanır (disjoint component erişimi), ve
    /// - `is_exclusive` sistemler tek başına çalışır.
    ///
    /// Bu sözleşme ihlal edilirse iki `&mut T` alias oluşur → tanımsız davranış. Özel erişimin
    /// varsa bunun yerine güvenli [`World::query_mut`]'i kullan.
    pub unsafe fn query_unchecked<'w, Q: crate::query::WorldQuery>(
        &'w self,
    ) -> Option<crate::query::Query<'w, Q>> {
        crate::query::Query::new(self)
    }

    /// Geriye uyumluluk için StorageView alternatifi (`&T` paylaşımlı erişim — daima sağlam).
    #[inline]
    pub fn borrow<'w, T: Component>(&'w self) -> crate::query::Query<'w, &'w T> {
        self.query::<&T>().expect("Failed to create borrow Query")
    }

    /// Tek bir component için mutable query (`Mut<T>`) — güvenli, `&mut self` ister.
    /// [`World::query_mut`]'in ergonomik kısaltması; aynı tip-düzeyi aliasing güvencesini taşır.
    #[inline]
    pub fn borrow_mut<'w, T: Component>(
        &'w mut self,
    ) -> crate::query::Query<'w, crate::query::Mut<'w, T>> {
        self.query_mut::<crate::query::Mut<T>>().expect("Failed to create borrow_mut Query")
    }

    /// [`World::borrow_mut`]'in `unsafe` kaçış-kapısı sürümü — `&World`'ten `Mut<T>` query'si
    /// kuran paralel-scheduler sistemleri için.
    ///
    /// # Safety
    /// [`World::query_unchecked`] ile aynı sözleşme: bu query canlıyken `T`'ye mutable dokunan
    /// başka bir query canlı olmamalı (scheduler disjointness'i garanti eder).
    #[inline]
    pub unsafe fn borrow_mut_unchecked<'w, T: Component>(
        &'w self,
    ) -> crate::query::Query<'w, crate::query::Mut<'w, T>> {
        self.query_unchecked::<crate::query::Mut<T>>()
            .expect("Failed to create borrow_mut_unchecked Query")
    }

    /// Cache'li query — archetype indeks cache'ini kullanır.
    /// &mut self gerektirdiği için sadece World sahibiyken çağrılabilir.
    pub fn query_cached<'w, Q: crate::query::WorldQuery>(
        &'w mut self,
    ) -> Option<crate::query::Query<'w, Q>> {
        crate::query::Query::new_cached(self)
    }

    /// **Ham `u32` id ile — generation kontrolü yapmaz.** Despawn+reuse sonrası yanlış
    /// entity'nin verisi dönebilir; canlılık kritikse önce [`World::is_alive`] çağırın.
    pub fn query_entity_mut<'w, Q: crate::query::WorldQuery>(
        &'w mut self,
        entity_id: u32,
    ) -> Option<Q::Item<'w>> {
        let loc = self.entity_location(entity_id);
        if !loc.is_valid() {
            return None;
        }
        let arch = &self.archetype_index.archetypes[loc.archetype_id as usize];
        if !Q::matches_archetype(arch) {
            return None;
        }
        unsafe {
            let fetch = Q::fetch_raw(self, arch, self.tick)?;
            if !Q::filter_row(fetch, loc.row as usize, entity_id, self.change_ref_tick) {
                return None;
            }
            Some(Q::get_item(fetch, loc.row as usize, entity_id))
        }
    }

    /// Tek bir entity üzerinde read-only `Query` çalıştırıp anında sonuç almanızı sağlar.
    ///
    /// `Q: ReadOnlyQuery` bağlı (paylaşımlı `&self`'ten mutable sonuç dönemez); mutable tekil
    /// erişim için [`World::query_entity_mut`] (`&mut self`).
    ///
    /// **Ham `u32` id ile — generation kontrolü yapmaz** (bkz. [`World::query_entity_mut`]).
    pub fn query_entity<'w, Q: crate::query::ReadOnlyQuery>(
        &'w self,
        entity_id: u32,
    ) -> Option<Q::Item<'w>> {
        let loc = self.entity_location(entity_id);
        if !loc.is_valid() {
            return None;
        }
        let arch = &self.archetype_index.archetypes[loc.archetype_id as usize];
        if !Q::matches_archetype(arch) {
            return None;
        }
        unsafe {
            let fetch = Q::fetch_raw(self, arch, self.tick)?;
            if !Q::filter_row(fetch, loc.row as usize, entity_id, self.change_ref_tick) {
                return None;
            }
            Some(Q::get_item(fetch, loc.row as usize, entity_id))
        }
    }
}
