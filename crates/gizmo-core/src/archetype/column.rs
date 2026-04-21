use std::any::TypeId;
use std::alloc::Layout;
use std::sync::RwLock;
use super::blob::BlobVec;
use std::ptr;

#[derive(Debug, Clone, Copy)]
pub struct ComponentTicks {
    pub added: u32,
    pub changed: u32,
}

impl ComponentTicks {
    pub fn new(tick: u32) -> Self {
        Self { added: tick, changed: tick }
    }
}

/// Archetype içindeki tek bir component tipinin sütunu.
pub struct Column {
    pub(crate) data: BlobVec,
    pub(crate) ticks: Vec<ComponentTicks>,
    type_id: TypeId,
    clone_fn: Option<unsafe fn(*const u8, *mut u8, usize)>,
}

impl Column {
    /// Yeni boş sütun oluşturur.
    pub fn new(type_id: TypeId, item_layout: Layout, drop_fn: Option<unsafe fn(*mut u8)>, clone_fn: Option<unsafe fn(*const u8, *mut u8, usize)>) -> Self {
        Self {
            data: BlobVec::new(item_layout, drop_fn),
            ticks: Vec::new(),
            type_id,
            clone_fn,
        }
    }

    #[inline]
    pub fn type_id(&self) -> TypeId {
        self.type_id
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Belirtilen satırdaki component'a immutable pointer döndürür.
    ///
    /// # Safety
    /// - `row < self.len()` olmalıdır
    /// - Dönen pointer geçerli bir `T` tipindeki veriye işaret eder
    #[inline]
    pub unsafe fn get_ptr(&self, row: usize) -> *const u8 {
        self.data.get_unchecked(row)
    }

    /// Belirtilen satırdaki component'a mutable pointer döndürür.
    ///
    /// # Safety
    /// - `row < self.len()` olmalıdır
    /// Sütun verisinin başlangıç pointer'ını döndürür.
    #[inline]
    pub fn data_ptr(&self) -> *const u8 {
        self.data.as_ptr()
    }

    /// Sütun verisinin başlangıç pointer'ını döndürür (mutable).
    #[inline]
    pub fn data_ptr_mut(&self) -> *mut u8 {
        unsafe { self.data.as_ptr() as *mut u8 }
    }

    /// Sütunun ComponentTick verilerinin başlangıç adresi.
    #[inline]
    pub fn ticks_ptr(&self) -> *const ComponentTicks {
        self.ticks.as_ptr()
    }

    /// Sütunun ComponentTick verilerinin başlangıç adresi (mutable).
    #[inline]
    pub fn ticks_ptr_mut(&self) -> *mut ComponentTicks {
        self.ticks.as_ptr() as *mut ComponentTicks
    }

    /// - Dönen pointer geçerli bir `T` tipindeki veriye işaret eder
    #[inline]
    pub unsafe fn get_mut_ptr(&self, row: usize) -> *mut u8 {
        self.data.get_unchecked_mut(row)
    }

    /// Sütun içindeki iki satırı takas eder.
    #[inline]
    pub unsafe fn swap_rows(&mut self, a: usize, b: usize) {
        self.data.swap_rows(a, b);
        self.ticks.swap(a, b);
    }

    /// Ham bayt olarak yeni bir değer ekler.
    ///
    /// # Safety
    /// `value` pointer'ı bu sütunun tip boyutu kadar okunabilir belleğe işaret etmelidir.
    #[inline]
    pub unsafe fn push_raw(&mut self, value: *const u8, tick: u32) {
        self.data.push(value);
        self.ticks.push(ComponentTicks::new(tick));
    }

    /// Bir component referansını alıp arka arkaya N kez kopyalar (Batch Prefab Cloning).
    #[inline]
    pub unsafe fn push_cloned_batch(&mut self, src: *const u8, count: usize, tick: u32) {
        self.data.push_cloned_batch(src, count, self.clone_fn);
        self.ticks.resize(self.ticks.len() + count, ComponentTicks::new(tick));
    }

    /// Belirtilen satırı swap-remove ile çıkarır ve düşürür.
    ///
    /// # Safety
    /// `row < self.len()` olmalıdır.
    #[inline]
    pub unsafe fn swap_remove_and_drop(&mut self, row: usize) {
        self.data.swap_remove_and_drop(row);
        self.ticks.swap_remove(row);
    }

    /// Belirtilen satırı swap-remove ile çıkarır, değeri `out`'a taşır.
    ///
    /// # Safety
    /// - `row < self.len()` olmalıdır
    /// - `out` pointer'ı yeterli boyutta yazılabilir belleğe işaret etmelidir
    #[inline]
    pub unsafe fn swap_remove_move(&mut self, row: usize, out: *mut u8) {
        self.data.swap_remove_unchecked(row, out);
        self.ticks.swap_remove(row);
    }

    /// Sütun belleğini sıkıştırır.
    pub fn shrink_to_fit(&mut self) {
        self.data.shrink_to_fit();
        self.ticks.shrink_to_fit();
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// COMPONENT INFO — Runtime tip bilgisi
// ═══════════════════════════════════════════════════════════════════════════

/// Bir component tipinin runtime'daki meta bilgileri.
/// Column oluştururken ve archetype migration'da kullanılır.
#[derive(Clone, Copy)]
pub struct ComponentInfo {
    pub type_id: TypeId,
    pub layout: Layout,
    pub drop_fn: Option<unsafe fn(*mut u8)>,
    pub clone_fn: Option<unsafe fn(*const u8, *mut u8, usize)>,
}

impl ComponentInfo {
    /// Belirtilen Rust tipi için ComponentInfo oluşturur.
    pub fn of<T: 'static + Clone>() -> Self {
        Self {
            type_id: TypeId::of::<T>(),
            layout: Layout::new::<T>(),
            drop_fn: if std::mem::needs_drop::<T>() {
                Some(|ptr: *mut u8| unsafe { ptr::drop_in_place(ptr as *mut T) })
            } else {
                None
            },
            clone_fn: Some(|src: *const u8, dst: *mut u8, count: usize| unsafe {
                let src = src as *const T;
                let dst = dst as *mut T;
                for i in 0..count {
                    ptr::write(dst.add(i), (*src).clone());
                }
            }),
        }
    }

    /// Sadece TypeId biliniyorsa (registry durumları), kısıtlı bir ComponentInfo oluşturur.
    pub fn of_type_id(type_id: TypeId) -> Self {
        Self {
            type_id,
            layout: Layout::from_size_align(0, 1).unwrap(), // Geçici, gerçek layout registry'den gelmeli
            drop_fn: None,
            clone_fn: None,
        }
    }
}


