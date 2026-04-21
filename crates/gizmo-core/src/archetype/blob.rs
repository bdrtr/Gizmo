use std::alloc::{self, Layout};
use std::ptr::{self, NonNull};

pub struct BlobVec {
    /// Her elemanın bellek yerleşimi (boyut + hizalama)
    item_layout: Layout,
    /// Destructor fonksiyonu — None ise drop gerekmez (Copy tipler)
    drop_fn: Option<unsafe fn(*mut u8)>,
    /// Tahsis edilmiş bellek bloğunun başlangıcı
    data: NonNull<u8>,
    /// Mevcut eleman sayısı
    pub(crate) len: usize,
    /// Tahsis edilmiş kapasite (eleman cinsinden)
    pub(crate) capacity: usize,
}

// BlobVec Send + Sync güvenlidir çünkü:
// - Tüm erişim &self veya &mut self üzerinden yapılır
// - İç pointer'a eşzamanlı erişim RefCell guard'ları ile korunur
unsafe impl Send for BlobVec {}
unsafe impl Sync for BlobVec {}

impl BlobVec {
    /// Yeni boş BlobVec oluşturur.
    ///
    /// # Arguments
    /// * `item_layout` — Her elemanın Layout'u (boyut + hizalama)
    /// * `drop_fn` — Eleman düşürme fonksiyonu. `None` ise drop çağrılmaz.
    pub fn new(item_layout: Layout, drop_fn: Option<unsafe fn(*mut u8)>) -> Self {
        // ZST (zero-sized type) kontrolü
        let (data, capacity) = if item_layout.size() == 0 {
            (NonNull::dangling(), usize::MAX)
        } else {
            (NonNull::dangling(), 0)
        };

        Self {
            item_layout,
            drop_fn,
            data,
            len: 0,
            capacity,
        }
    }

    /// Mevcut eleman sayısı
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Boş mu?
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Belirtilen indeksteki elemanın ham pointer'ını döndürür.
    ///
    /// # Safety
    /// `index < self.len` olmalıdır.
    #[inline]
    pub unsafe fn get_unchecked(&self, index: usize) -> *const u8 {
        debug_assert!(index < self.len, "BlobVec::get_unchecked: index {} >= len {}", index, self.len);
        self.data.as_ptr().add(index * self.item_layout.size())
    }

    /// Belirtilen indeksteki elemanın mutable ham pointer'ını döndürür.
    ///
    /// # Safety
    /// `index < self.len` olmalıdır.
    #[inline]
    pub unsafe fn get_unchecked_mut(&self, index: usize) -> *mut u8 {
        debug_assert!(index < self.len, "BlobVec::get_unchecked_mut: index {} >= len {}", index, self.len);
        self.data.as_ptr().add(index * self.item_layout.size())
    }

    pub unsafe fn push(&mut self, value: *const u8) {
        if self.item_layout.size() == 0 {
            self.len += 1;
            return;
        }
        self.reserve(1);
        let dst = self.data.as_ptr().add(self.len * self.item_layout.size());
        ptr::copy_nonoverlapping(value, dst, self.item_layout.size());
        self.len += 1;
    }

    /// Bir component'ı clone fonksiyonunu kullanarak N kere çoğaltır ve arkaya ekler.
    ///
    /// # Safety
    /// `src` pointer'ı `item_layout.size()` bayt okunabilir belleğe işaret etmelidir.
    pub unsafe fn push_cloned_batch(&mut self, src: *const u8, count: usize, clone_fn: Option<unsafe fn(*const u8, *mut u8, usize)>) {
        if count == 0 { return; }
        if self.item_layout.size() == 0 {
            self.len += count;
            return;
        }
        self.reserve(count);
        let dst_start = self.data.as_ptr().add(self.len * self.item_layout.size());
        
        if let Some(c_fn) = clone_fn {
            c_fn(src, dst_start, count);
        } else {
            // fallback (copy türler için vb.)
            let size = self.item_layout.size();
            let mut current_dst = dst_start;
            for _ in 0..count {
                ptr::copy_nonoverlapping(src, current_dst, size);
                current_dst = current_dst.add(size);
            }
        }
        self.len += count;
    }

    /// İki satırın ham bellek içeriğini takas eder (Swap).
    /// Hiyerarşi gibi önbellek-dostu (cache-friendly) bellek kaydırmaları için oldukça etkilidir.
    ///
    /// # Safety
    /// `a < self.len` ve `b < self.len` olmalıdır.
    pub unsafe fn swap_rows(&mut self, a: usize, b: usize) {
        if a == b || self.item_layout.size() == 0 {
            return;
        }
        debug_assert!(a < self.len && b < self.len);
        let ptr_a = self.get_unchecked_mut(a);
        let ptr_b = self.get_unchecked_mut(b);
        let size = self.item_layout.size();
        ptr::swap_nonoverlapping(ptr_a, ptr_b, size);
    }

    /// Veri alanının ham pointer'ını döndürür.
    pub fn as_ptr(&self) -> *const u8 {
        self.data.as_ptr()
    }

    /// Son elemanı swap-and-pop ile belirtilen indeksten çıkarır ve eski değeri düşürür.
    ///
    /// # Safety
    /// `index < self.len` olmalıdır.
    pub unsafe fn swap_remove_and_drop(&mut self, index: usize) {
        debug_assert!(index < self.len);
        let last = self.len - 1;

        if index != last {
            let src = self.get_unchecked(last) as *const u8;
            let dst = self.get_unchecked_mut(index);
            // Önce eski değeri düşür
            if let Some(drop_fn) = self.drop_fn {
                drop_fn(dst);
            }
            // Sonra son elemanı kopyala
            ptr::copy_nonoverlapping(src, dst, self.item_layout.size());
        } else {
            // Son eleman zaten silinecek olan — sadece düşür
            if let Some(drop_fn) = self.drop_fn {
                let ptr = self.get_unchecked_mut(index);
                drop_fn(ptr);
            }
        }

        self.len -= 1;
    }

    /// Son elemanı swap-and-pop ile çıkarır, eski değeri `out` pointer'ına taşır (düşürmez).
    ///
    /// # Safety
    /// - `index < self.len` olmalıdır
    /// - `out` pointer'ı `item_layout.size()` bayt yazılabilir belleğe işaret etmelidir
    pub unsafe fn swap_remove_unchecked(&mut self, index: usize, out: *mut u8) {
        debug_assert!(index < self.len);
        let last = self.len - 1;

        // Çıkarılan elemanı out'a kopyala
        let src = self.get_unchecked(index) as *const u8;
        ptr::copy_nonoverlapping(src, out, self.item_layout.size());

        if index != last {
            // Son elemanı çıkarılan yere taşı
            let last_src = self.get_unchecked(last) as *const u8;
            let dst = self.get_unchecked_mut(index);
            ptr::copy_nonoverlapping(last_src, dst, self.item_layout.size());
        }

        self.len -= 1;
    }

    /// Yeterli kapasite yoksa büyüt.
    pub(crate) fn reserve(&mut self, additional: usize) {
        let required = self.len + additional;
        if required <= self.capacity {
            return;
        }

        let new_capacity = required.max(self.capacity * 2).max(4);
        self.grow(new_capacity);
    }

    /// Kapasiteyi belirtilen değere büyüt.
    fn grow(&mut self, new_capacity: usize) {
        assert!(new_capacity > self.capacity);
        let item_size = self.item_layout.size();
        if item_size == 0 {
            return;
        }

        let new_layout = Layout::from_size_align(item_size * new_capacity, self.item_layout.align())
            .expect("BlobVec::grow: Layout overflow");

        let new_data = if self.capacity == 0 {
            // İlk tahsis
            unsafe { alloc::alloc(new_layout) }
        } else {
            // Yeniden tahsis
            let old_layout = Layout::from_size_align(item_size * self.capacity, self.item_layout.align())
                .expect("BlobVec::grow: Old layout overflow");
            unsafe { alloc::realloc(self.data.as_ptr(), old_layout, new_layout.size()) }
        };

        self.data = NonNull::new(new_data).expect("BlobVec::grow: Allocation failed (OOM)");
        self.capacity = new_capacity;
    }

    /// Küçültme (Defragmentation) operasyonu. BlobVec'in capacity değerini len değerine eşitler.
    pub fn shrink_to_fit(&mut self) {
        if self.capacity == self.len {
            return;
        }
        let item_size = self.item_layout.size();
        if item_size == 0 {
            self.capacity = self.len;
            return;
        }

        if self.len == 0 {
            // Tamamen boşalt, belleği dealloc yap.
            let old_layout = Layout::from_size_align(item_size * self.capacity, self.item_layout.align()).unwrap();
            unsafe { alloc::dealloc(self.data.as_ptr(), old_layout) };
            self.data = NonNull::dangling();
            self.capacity = 0;
            return;
        }

        let new_layout = Layout::from_size_align(item_size * self.len, self.item_layout.align()).unwrap();
        let old_layout = Layout::from_size_align(item_size * self.capacity, self.item_layout.align()).unwrap();

        let new_data = unsafe { alloc::realloc(self.data.as_ptr(), old_layout, new_layout.size()) };
        self.data = NonNull::new(new_data).expect("BlobVec::shrink_to_fit: Allocation failed (OOM)");
        self.capacity = self.len;
    }

    /// Tüm elemanları düşürür (belleği serbest bırakmadan).
    fn clear(&mut self) {
        if let Some(drop_fn) = self.drop_fn {
            let item_size = self.item_layout.size();
            for i in 0..self.len {
                unsafe {
                    let ptr = self.data.as_ptr().add(i * item_size);
                    drop_fn(ptr);
                }
            }
        }
        self.len = 0;
    }
}

impl Drop for BlobVec {
    fn drop(&mut self) {
        self.clear();
        let item_size = self.item_layout.size();
        if item_size > 0 && self.capacity > 0 {
            let layout = Layout::from_size_align(item_size * self.capacity, self.item_layout.align())
                .expect("BlobVec::drop: Layout error");
            unsafe {
                alloc::dealloc(self.data.as_ptr(), layout);
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// COLUMN — Tip-silinmiş sütun

