use crate::archetype::blob::BlobVec;
use crate::archetype::column::ComponentInfo;
use crate::archetype::ComponentTicks;

/// Bir SparseSet, bileşen verilerini Entity ID'lerine göre hızlıca dışarıdan yönetmeyi sağlar.
/// Archetype tablosuna girmeden doğrudan ekleme/silme yapılabilmesi için tasarlanmıştır.
pub struct ComponentSparseSet {
    pub info: ComponentInfo,
    pub dense: BlobVec,
    pub ticks: Vec<ComponentTicks>,
    pub entities: Vec<u32>, // dense row index -> Entity ID
    pub sparse: Vec<u32>,   // Entity ID -> dense row index (Yoksa u32::MAX)
}

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
            self.ticks[row].changed = tick;
        } else {
            // Yeni satır oluştur
            let row = self.dense.len() as u32;
            unsafe {
                self.dense.push(data_ptr);
            }
            self.ticks.push(ComponentTicks::new(tick));
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
        self.ticks.get(self.sparse[e] as usize)
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
}
