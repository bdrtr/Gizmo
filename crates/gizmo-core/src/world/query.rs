use super::World;
use crate::component::Component;

impl World {
    // ==========================================================
    // ERGONOMİK SORGULAR (QUERY API)
    // ==========================================================

    /// Bir [`Query`](crate::query::Query) oluşturur (ergonomik component erişimi).
    ///
    /// # Aliasing sözleşmesi — `Mut<T>` kullanmadan önce OKUYUN
    /// Bu metod `&self` alır, yani borrow checker aynı anda iki query tutmanı
    /// ENGELLEMEZ. Paylaşımlı (`&T`) erişim için bu daima sağlamdır. Ama mutable
    /// (`Mut<T>` / [`World::borrow_mut`]) erişim için **DEĞİL**: aynı component'e
    /// mutable dokunan iki *canlı* query, aynı depolamaya iki `&mut T` verir — bu
    /// tanımsız davranıştır (UB), ve %100 güvenli koddan ulaşılabildiği için ne panik
    /// ne derleme hatası olur. Query-içi
    /// [`check_aliasing`](crate::query::WorldQuery::check_aliasing) guard'ı yalnız
    /// **tek bir** query içinde aynı component'i iki kez yakalar; query'ler ARASI takip
    /// yapılmaz.
    ///
    /// **Çağıran invariant'ı:** aynı component'e mutable erişen iki query aynı anda canlı
    /// olmasın. Somut olarak:
    /// - ✅ `world.query::<(&A, Mut<B>)>()` — tek birleşik query (sağlam; B'ye özel erişim).
    /// - ✅ bir `Mut<A>` query'si TAMAMEN düşürülür, sonra başka bir `Mut<A>` query'si.
    /// - ✅ aynı anda istediğin kadar `&A` (paylaşımlı) query.
    /// - ❌ bir `Mut<A>` query'si canlıyken başka bir `Mut<A>` (veya `&A`) açmak → UB.
    ///
    /// Garantili-özel erişim gerekiyorsa `&mut self` alan giriş noktalarını tercih et
    /// ([`World::query_cached`], [`World::query_entity_mut`]) — tip sistemi onları senin
    /// için aliasing-suz yapar.
    ///
    /// **Yapısal eliminasyon** (query *oluşturma*yı *iterasyon*dan ayırıp mutable
    /// iterasyonu `&mut World` gerektirmek — Bevy'nin `iter(&world)`/`iter_mut(&mut world)`
    /// modeli — UB'yi tip düzeyinde kaldırır) planlı post-0.x işidir: her çağrı yerini
    /// etkileyen breaking bir değişiklik olduğundan İZLENİYOR, henüz YAPILMADI.
    pub fn query<'w, Q: crate::query::WorldQuery>(&'w self) -> Option<crate::query::Query<'w, Q>> {
        crate::query::Query::new(self)
    }

    /// Geriye uyumluluk için StorageView alternatifi (`&T` paylaşımlı erişim — daima sağlam).
    #[inline]
    pub fn borrow<'w, T: Component>(&'w self) -> crate::query::Query<'w, &'w T> {
        self.query::<&T>().expect("Failed to create borrow Query")
    }

    /// Geriye uyumluluk için StorageViewMut alternatifi.
    ///
    /// **DİKKAT — aliasing:** aynı `T` için iki `borrow_mut` (veya bir `borrow_mut` +
    /// bir `borrow`) aynı anda canlıyken UB doğar (bkz. [`World::query`] aliasing
    /// sözleşmesi). Birini açmadan önce diğerinin düştüğünden emin ol, ya da birleşik
    /// query / `&mut self` giriş noktalarını kullan.
    #[inline]
    pub fn borrow_mut<'w, T: Component>(&'w self) -> crate::query::Query<'w, crate::query::Mut<'w, T>> {
        self.query::<crate::query::Mut<T>>().expect("Failed to create borrow_mut Query")
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
    /// **Ham `u32` id ile — generation kontrolü yapmaz** (bkz. [`World::query_entity_mut`]).
    pub fn query_entity<'w, Q: crate::query::WorldQuery>(
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
