use crate::archetype::blob::BlobVec;
use crate::archetype::column::ComponentInfo;
use crate::archetype::ComponentTicks;
use std::cell::UnsafeCell;

/// Bir SparseSet, bileşen verilerini Entity ID'lerine göre hızlıca dışarıdan yönetmeyi sağlar.
/// Archetype tablosuna girmeden doğrudan ekleme/silme yapılabilmesi için tasarlanmıştır.
pub struct ComponentSparseSet {
    pub info: ComponentInfo,
    pub dense: BlobVec,
    /// Değişiklik-tespiti tick'leri. `UnsafeCell` ile içsel-değişebilir — tıpkı
    /// `dense: BlobVec`'in ham-pointer'lı iç değişebilirliği gibi: `Mut<T>` sorgu
    /// yolu (`query::fetch::get_item`) set'e PAYLAŞIMLI `&self` ile erişip ayrık
    /// satırları yazar (paralel `par_for_each_mut` için gerekli). Düz `Vec<_>`
    /// olsaydı `as_ptr(&self)` yalnız-okuma provenance verir → `&mut *ticks_ptr`
    /// aliasing UB olurdu (güvenli koddan ulaşılabilir). Cell üzerinden yazma sağlam.
    pub ticks: Vec<UnsafeCell<ComponentTicks>>,
    pub entities: Vec<u32>, // dense row index -> Entity ID
    pub sparse: Vec<u32>,   // Entity ID -> dense row index (Yoksa u32::MAX)
}

// SAFETY: `BlobVec`/`Archetype` üzerindeki aynı impl'lerle aynı gerekçe. İçsel-
// değişebilir alanlara (`dense` ham-pointer, `ticks` `UnsafeCell`) yalnız sorgu
// zamanlayıcısının ayrık-erişim garantisi altında yazılır → iki thread aynı satırı
// eşzamanlı yazmaz. `UnsafeCell<ComponentTicks>` eklenince otomatik `Sync` düştü.
unsafe impl Send for ComponentSparseSet {}
unsafe impl Sync for ComponentSparseSet {}

impl ComponentSparseSet {
    pub fn new(info: ComponentInfo) -> Self {
        Self {
            info,
            dense: BlobVec::new(info.layout, info.drop_fn),
            ticks: Vec::new(),
            entities: Vec::new(),
            sparse: Vec::new(),
        }
    }

    /// Bir entity için veriyi SparseSet'e yazar. (Ekler veya üzerine yazar).
    ///
    /// # Safety
    /// `data_ptr`, bu set'in `info.layout` değeriyle uyumlu, geçerli ve hizalanmış
    /// bir bileşen örneğini göstermelidir. İşaretçinin sahipliği SparseSet'e devredilir
    /// (çağıran taraf, kopyalanan değeri ayrıca `drop` etmemeli — bkz. `std::mem::forget`).
    pub unsafe fn insert(&mut self, entity: u32, data_ptr: *const u8, tick: u32) {
        let e = entity as usize;
        if e >= self.sparse.len() {
            self.sparse.resize(e + 1, u32::MAX);
        }

        let existing_row = self.sparse[e];
        if existing_row != u32::MAX {
            // Zaten var, üzerine yaz
            let row = existing_row as usize;
            unsafe {
                let slot = self.dense.get_unchecked_mut(row);
                // Üzerine yazmadan ÖNCE eski değeri düşür; aksi halde heap sahibi
                // bir bileşen (String/Vec) yeniden eklenince eski tahsis sızardı
                // (çağıran taraf yeni değeri mem::forget ediyor).
                if let Some(drop_fn) = self.info.drop_fn {
                    drop_fn(slot);
                }
                std::ptr::copy_nonoverlapping(data_ptr, slot, self.info.layout.size());
            }
            self.ticks[row].get_mut().changed = tick;
        } else {
            // Yeni satır oluştur
            let row = self.dense.len() as u32;
            unsafe {
                self.dense.push(data_ptr);
            }
            self.ticks.push(UnsafeCell::new(ComponentTicks::new(tick)));
            self.entities.push(entity);
            self.sparse[e] = row;
        }
    }

    /// Bir entity'nin verisini O(1) hızında siler.
    pub fn remove(&mut self, entity: u32) -> bool {
        let e = entity as usize;
        if e >= self.sparse.len() || self.sparse[e] == u32::MAX {
            return false; // Bulunamadı
        }

        let row = self.sparse[e] as usize;
        let last_row = self.dense.len() - 1;

        unsafe {
            self.dense.swap_remove_and_drop(row);
        }
        self.ticks.swap_remove(row);
        let last_entity = self.entities[last_row];
        self.entities.swap_remove(row);

        self.sparse[e] = u32::MAX;

        // Eğer silinen eleman dizinin sonundaki eleman değilse,
        // son sıradan alıp silinen yere taşıdığımız (swap) objenin sparse indexini güncelliyoruz.
        if row != last_row {
            self.sparse[last_entity as usize] = row as u32;
        }

        true
    }

    #[inline]
    pub fn contains(&self, entity: u32) -> bool {
        let e = entity as usize;
        e < self.sparse.len() && self.sparse[e] != u32::MAX
    }

    /// Bir entity'nin değişiklik-tespiti tick'lerini döndürür (yoksa `None`).
    /// `Changed<T>`/`Added<T>` filtreleri SparseSet bileşenleri için bunu kullanır.
    #[inline]
    pub fn ticks_for(&self, entity: u32) -> Option<&ComponentTicks> {
        let e = entity as usize;
        if e >= self.sparse.len() || self.sparse[e] == u32::MAX {
            return None;
        }
        // SAFETY: paylaşımlı okuma. Bu tick hücresine yazma yalnız `&mut self`
        // metotlarından ya da `get_item`'in ayrık-satır içsel-değişebilir yolundan
        // gelir; bu erişim `&self` ödünçlediğinden aynı hücreye canlı bir
        // `&mut ComponentTicks` yoktur.
        self.ticks
            .get(self.sparse[e] as usize)
            .map(|c| unsafe { &*c.get() })
    }

    #[inline]
    pub fn get_ptr(&self, entity: u32) -> Option<*const u8> {
        let e = entity as usize;
        if e >= self.sparse.len() || self.sparse[e] == u32::MAX {
            return None;
        }
        unsafe { Some(self.dense.get_unchecked(self.sparse[e] as usize)) }
    }

    #[inline]
    pub fn get_ptr_mut(&mut self, entity: u32) -> Option<*mut u8> {
        let e = entity as usize;
        if e >= self.sparse.len() || self.sparse[e] == u32::MAX {
            return None;
        }
        unsafe { Some(self.dense.get_unchecked_mut(self.sparse[e] as usize)) }
    }

    /// Deep-clone the component stored for `src` into `dst` (both entity ids),
    /// using the component's `clone_fn`. Returns `false` if `src` has no entry or
    /// the component is not `Clone`. Used by `World::clone_entity` (prefab splice),
    /// which otherwise only clones archetype (table) columns.
    pub fn clone_entry(&mut self, src: u32, dst: u32, tick: u32) -> bool {
        let Some(clone_fn) = self.info.clone_fn else {
            return false;
        };
        let Some(src_ptr) = self.get_ptr(src) else {
            return false;
        };
        let layout = self.info.layout;
        // Clone src into a temp buffer, then hand it to `insert`, which memcpys
        // the bytes and takes ownership — so the buffer is freed WITHOUT dropping
        // the moved-out value (mirrors the mem::forget-after-raw-insert pattern).
        // src_ptr points into `dense`; it is consumed by clone_fn BEFORE insert
        // may reallocate `dense`, so it never dangles.
        unsafe {
            if layout.size() == 0 {
                let z = std::ptr::NonNull::<u8>::dangling().as_ptr();
                clone_fn(src_ptr, z, 1);
                self.insert(dst, z, tick);
            } else {
                let tmp = std::alloc::alloc(layout);
                if tmp.is_null() {
                    std::alloc::handle_alloc_error(layout);
                }
                clone_fn(src_ptr, tmp, 1);
                self.insert(dst, tmp, tick);
                std::alloc::dealloc(tmp, layout);
            }
        }
        true
    }
}
