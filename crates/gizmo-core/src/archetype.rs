//! # Archetype Storage
//!
//! Aynı component bileşimine sahip entity'leri, sütun bazlı (SoA) bitişik bellekte
//! depolayan yüksek performanslı ECS depolama katmanı.
//!
//! ## Yapılar
//! - [`BlobVec`]  — Tip-silinmiş, hizalanmış vektör. Tek bir component sütunu için ham bellek.
//! - [`Column`]   — `BlobVec` + `TypeId` sarmalayıcı.
//! - [`Archetype`] — Birden fazla `Column` ve entity listesi barındıran tablo.
//! - [`EntityLocation`] — Entity'nin hangi archetype'ta, hangi satırda olduğu.

use std::alloc::{self, Layout};
use std::any::TypeId;
use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::collections::HashMap;
use std::ptr::{self, NonNull};

// ═══════════════════════════════════════════════════════════════════════════
// ENTITY LOCATION
// ═══════════════════════════════════════════════════════════════════════════

/// Entity'nin World içindeki fiziksel konumu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntityLocation {
    /// Hangi archetype tablosunda
    pub archetype_id: u32,
    /// Archetype içindeki satır indeksi
    pub row: u32,
}

impl EntityLocation {
    pub const INVALID: Self = Self {
        archetype_id: u32::MAX,
        row: u32::MAX,
    };

    #[inline]
    pub fn is_valid(self) -> bool {
        self.archetype_id != u32::MAX
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// BLOB VEC — Tip-silinmiş vektör
// ═══════════════════════════════════════════════════════════════════════════

/// Tip bilgisi olmadan ham bayt dizisi olarak component verisi saklayan vektör.
///
/// `Layout` ile hizalanmış bellek bloğu kullanır. Her eleman `item_layout.size()` bayttır.
/// Destructor, `drop_fn` fonksiyon pointer'ı ile çağrılır.
pub struct BlobVec {
    /// Her elemanın bellek yerleşimi (boyut + hizalama)
    item_layout: Layout,
    /// Destructor fonksiyonu — None ise drop gerekmez (Copy tipler)
    drop_fn: Option<unsafe fn(*mut u8)>,
    /// Tahsis edilmiş bellek bloğunun başlangıcı
    data: NonNull<u8>,
    /// Mevcut eleman sayısı
    len: usize,
    /// Tahsis edilmiş kapasite (eleman cinsinden)
    capacity: usize,
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

    /// Yeni bir elemanı sona ekler (ham bayt olarak).
    ///
    /// # Safety
    /// `value` pointer'ı `item_layout.size()` bayt okunabilir belleğe işaret etmelidir.
    /// Çağıran, `value`'nun sahipliğini BlobVec'e devreder — value'dan sonra okuma yapmamalıdır.
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
// ═══════════════════════════════════════════════════════════════════════════

/// Archetype içindeki tek bir component tipinin sütunu.
pub struct Column {
    pub(crate) data: BlobVec,
    type_id: TypeId,
}

impl Column {
    /// Yeni boş sütun oluşturur.
    pub fn new(type_id: TypeId, item_layout: Layout, drop_fn: Option<unsafe fn(*mut u8)>) -> Self {
        Self {
            data: BlobVec::new(item_layout, drop_fn),
            type_id,
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

    /// - Dönen pointer geçerli bir `T` tipindeki veriye işaret eder
    #[inline]
    pub unsafe fn get_mut_ptr(&self, row: usize) -> *mut u8 {
        self.data.get_unchecked_mut(row)
    }

    /// Ham bayt olarak yeni bir değer ekler.
    ///
    /// # Safety
    /// `value` pointer'ı bu sütunun tip boyutu kadar okunabilir belleğe işaret etmelidir.
    #[inline]
    pub unsafe fn push_raw(&mut self, value: *const u8) {
        self.data.push(value);
    }

    /// Belirtilen satırı swap-remove ile çıkarır ve düşürür.
    ///
    /// # Safety
    /// `row < self.len()` olmalıdır.
    #[inline]
    pub unsafe fn swap_remove_and_drop(&mut self, row: usize) {
        self.data.swap_remove_and_drop(row);
    }

    /// Belirtilen satırı swap-remove ile çıkarır, değeri `out`'a taşır.
    ///
    /// # Safety
    /// - `row < self.len()` olmalıdır
    /// - `out` pointer'ı yeterli boyutta yazılabilir belleğe işaret etmelidir
    #[inline]
    pub unsafe fn swap_remove_move(&mut self, row: usize, out: *mut u8) {
        self.data.swap_remove_unchecked(row, out);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// ARCHETYPE EDGE — Component ekleme/çıkarma geçiş cache'i
// ═══════════════════════════════════════════════════════════════════════════

/// Bir component tipinin eklenmesi veya çıkarılması sonucu hangi archetype'a
/// geçileceğini cache'leyen kenar yapısı.
#[derive(Debug, Clone, Copy)]
pub struct ArchetypeEdge {
    /// Bu component tipi eklenince hedef archetype
    pub add: Option<u32>,
    /// Bu component tipi çıkarılınca hedef archetype
    pub remove: Option<u32>,
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
}

impl ComponentInfo {
    /// Belirtilen Rust tipi için ComponentInfo oluşturur.
    pub fn of<T: 'static>() -> Self {
        Self {
            type_id: TypeId::of::<T>(),
            layout: Layout::new::<T>(),
            drop_fn: if std::mem::needs_drop::<T>() {
                Some(|ptr: *mut u8| unsafe { ptr::drop_in_place(ptr as *mut T) })
            } else {
                None
            },
        }
    }

    /// Sadece TypeId biliniyorsa (registry durumları), kısıtlı bir ComponentInfo oluşturur.
    pub fn of_type_id(type_id: TypeId) -> Self {
        Self {
            type_id,
            layout: Layout::from_size_align(0, 1).unwrap(), // Geçici, gerçek layout registry'den gelmeli
            drop_fn: None,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// ARCHETYPE — Sütun tablosu
// ═══════════════════════════════════════════════════════════════════════════

/// Aynı component bileşimine sahip entity'lerin sütun bazlı depolama tablosu.
pub struct Archetype {
    /// Bu archetype'ın global indeks numarası
    pub id: u32,
    /// Component tipi → sütun indeksi (columns vektöründeki)
    column_indices: HashMap<TypeId, usize>,
    /// Sütunların vektörü — her biri bir component tipinin verisi.
    /// RwLock ile sarmalandı çünkü aynı archetype içindeki farklı sütunlara 
    /// eşzamanlı ve multi-thread erişim (örn: &Transform ve &mut Velocity) gerekebilir.
    columns: Vec<RwLock<Column>>,
    /// Bu archetype'taki entity ID'leri (sıra = satır indeksi)
    entities: Vec<u32>,
    /// Component ekleme/çıkarma geçiş cache'i
    /// TypeId → ArchetypeEdge
    edges: HashMap<TypeId, ArchetypeEdge>,
}

impl Archetype {
    /// Belirtilen component tipleri için yeni boş archetype oluşturur.
    pub fn new(id: u32, component_infos: &[ComponentInfo]) -> Self {
        let mut column_indices = HashMap::with_capacity(component_infos.len());
        let mut columns = Vec::with_capacity(component_infos.len());

        for (idx, info) in component_infos.iter().enumerate() {
            column_indices.insert(info.type_id, idx);
            columns.push(RwLock::new(Column::new(info.type_id, info.layout, info.drop_fn)));
        }

        Self {
            id,
            column_indices,
            columns,
            entities: Vec::new(),
            edges: HashMap::new(),
        }
    }

    /// Bu archetype'taki entity sayısı
    #[inline]
    pub fn len(&self) -> usize {
        self.entities.len()
    }

    /// Boş mu?
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    /// Entity ID listesine referans
    #[inline]
    pub fn entities(&self) -> &[u32] {
        &self.entities
    }

    /// Bu archetype belirtilen component tipini içeriyor mu?
    #[inline]
    pub fn has_component(&self, type_id: TypeId) -> bool {
        self.column_indices.contains_key(&type_id)
    }

    /// Bu archetype'taki component tiplerinin listesi
    pub fn component_types(&self) -> Vec<TypeId> {
        self.column_indices.keys().cloned().collect()
    }

    /// Sıralanmış component tipleri (archetype kimliği olarak kullanılır)
    pub fn sorted_component_types(&self) -> Vec<TypeId> {
        let mut types = self.component_types();
        types.sort();
        types
    }

    /// Belirtilen component tipinin sütununa RwLock üzerinden immutable erişim
    #[inline]
    pub fn get_column(&self, type_id: TypeId) -> Option<RwLockReadGuard<'_, Column>> {
        self.column_indices.get(&type_id).map(|&idx| self.columns[idx].read().unwrap())
    }

    /// Belirtilen component tipinin sütununa RwLock üzerinden mutable erişim
    #[inline]
    pub fn get_column_mut(&self, type_id: TypeId) -> Option<RwLockWriteGuard<'_, Column>> {
        self.column_indices.get(&type_id).map(|&idx| self.columns[idx].write().unwrap())
    }

    /// Yeni bir entity satırı ekler. Tüm sütunlara veri zaten push edilmiş olmalıdır.
    /// Eklenen satır indeksini döndürür.
    #[inline]
    pub(crate) fn push_entity(&mut self, entity_id: u32) -> u32 {
        let row = self.entities.len() as u32;
        self.entities.push(entity_id);
        row
    }

    /// Belirtilen satırdaki entity'yi swap-remove ile çıkarır.
    /// Taşınan entity'nin (eski son satırdaki) ID'sini döndürür.
    /// Eğer çıkarılan zaten son sıradaysa None döndürür.
    pub(crate) fn swap_remove_entity(&mut self, row: usize) -> Option<u32> {
        let last = self.entities.len() - 1;

        // Tüm sütunlarda swap_remove_and_drop
        for col_cell in &mut self.columns {
            unsafe {
                col_cell.get_mut().unwrap().swap_remove_and_drop(row);
            }
        }

        if row != last {
            let moved_entity = self.entities[last];
            self.entities.swap(row, last);
            self.entities.pop();
            Some(moved_entity)
        } else {
            self.entities.pop();
            None
        }
    }

    /// Bir entity'nin verilerini bir archetype'tan diğerine taşır (Migration).
    /// `source_row`: Kaynak archetype'taki satır.
    /// `target`: Hedef archetype.
    /// Dönen değer: Hedef archetype'taki yeni satır indeksi.
    pub(crate) unsafe fn move_entity_to(&mut self, source_row: usize, target: &mut Archetype) -> (u32, Option<u32>) {
        let entity_id = self.entities[source_row];

        // 1. Hedef archetype'ın TÜM sütunlarını genişlet (ortak olanları taşı, olmayanları boş bırak)
        for (type_id, &dst_col_idx) in &target.column_indices {
            let mut dst_col = target.columns[dst_col_idx].write().unwrap();
            
            // Hedefte her zaman yer açmalıyız ki sütun boyu entity listesiyle uyuşsun
            dst_col.data.reserve(1);
            let row_to_write = dst_col.data.len;
            dst_col.data.len += 1; // Önce boyutu artır ki get_unchecked_mut geçsin
            
            let dst_ptr = dst_col.data.get_unchecked_mut(row_to_write);

            if let Some(&src_col_idx) = self.column_indices.get(type_id) {
                let mut src_col = self.columns[src_col_idx].write().unwrap();
                // Veriyi kopyala ve kaynak sütunda swap-remove yap
                src_col.data.swap_remove_unchecked(source_row, dst_ptr);
            } else {
                // Bu sütun kaynakta yok (yeni ekleniyor), yer ayırt ama veri yazma (caller yapacak)
            }
        }

        // 2. Hedefte olmayan ama kaynakta olan component'ları temizle
        for (type_id, &src_col_idx) in &self.column_indices {
            if !target.column_indices.contains_key(type_id) {
                let mut src_col = self.columns[src_col_idx].write().unwrap();
                src_col.swap_remove_and_drop(source_row);
            }
        }

        // 2. Kaynak archetype'tan entity listesini güncelle (sütunlar zaten swap_remove edildi)
        let last = self.entities.len() - 1;
        let moved_entity = if source_row != last {
            let moved = self.entities[last];
            self.entities.swap(source_row, last);
            self.entities.pop();
            Some(moved)
        } else {
            self.entities.pop();
            None
        };

        // 3. Hedef archetype'a entity ID'sini kaydet
        let new_row = target.push_entity(entity_id);
        (new_row, moved_entity)
    }

    /// Edge cache'den belirtilen tipten geçiş hedefini al
    #[inline]
    pub fn get_edge(&self, type_id: TypeId) -> Option<&ArchetypeEdge> {
        self.edges.get(&type_id)
    }

    /// Edge cache'e yeni geçiş hedefi ekle
    #[inline]
    pub fn set_add_edge(&mut self, type_id: TypeId, target: u32) {
        let edge = self.edges.entry(type_id).or_insert(ArchetypeEdge { add: None, remove: None });
        edge.add = Some(target);
    }

    /// Edge cache'e çıkarma geçiş hedefi ekle
    #[inline]
    pub fn set_remove_edge(&mut self, type_id: TypeId, target: u32) {
        let edge = self.edges.entry(type_id).or_insert(ArchetypeEdge { add: None, remove: None });
        edge.remove = Some(target);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// TESTS
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blob_vec_push_and_read() {
        let layout = Layout::new::<u32>();
        let mut blob = BlobVec::new(layout, None);

        let values: Vec<u32> = vec![10, 20, 30, 40, 50];
        for v in &values {
            unsafe {
                blob.push(v as *const u32 as *const u8);
            }
        }

        assert_eq!(blob.len(), 5);

        for (i, expected) in values.iter().enumerate() {
            unsafe {
                let ptr = blob.get_unchecked(i) as *const u32;
                assert_eq!(*ptr, *expected);
            }
        }
    }

    #[test]
    fn blob_vec_swap_remove() {
        let layout = Layout::new::<u64>();
        let mut blob = BlobVec::new(layout, None);

        let values: Vec<u64> = vec![100, 200, 300, 400];
        for v in &values {
            unsafe {
                blob.push(v as *const u64 as *const u8);
            }
        }

        // index 1'i (200) çıkar → son(400) onun yerine gelir
        unsafe {
            blob.swap_remove_and_drop(1);
        }
        assert_eq!(blob.len(), 3);

        unsafe {
            assert_eq!(*(blob.get_unchecked(0) as *const u64), 100);
            assert_eq!(*(blob.get_unchecked(1) as *const u64), 400); // swap
            assert_eq!(*(blob.get_unchecked(2) as *const u64), 300);
        }
    }

    #[test]
    fn blob_vec_drop_called() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static DROP_COUNT: AtomicUsize = AtomicUsize::new(0);

        #[repr(C)]
        struct Droppable(u32);
        impl Drop for Droppable {
            fn drop(&mut self) {
                DROP_COUNT.fetch_add(1, Ordering::Relaxed);
            }
        }

        DROP_COUNT.store(0, Ordering::Relaxed);

        let layout = Layout::new::<Droppable>();
        let drop_fn: unsafe fn(*mut u8) = |ptr| unsafe {
            ptr::drop_in_place(ptr as *mut Droppable);
        };

        {
            let mut blob = BlobVec::new(layout, Some(drop_fn));
            for i in 0..5 {
                let val = Droppable(i);
                unsafe {
                    blob.push(&val as *const Droppable as *const u8);
                }
                std::mem::forget(val); // BlobVec sahiplik alır
            }
            assert_eq!(blob.len(), 5);
            // blob drop olunca 5 adet Droppable düşürülmeli
        }

        assert_eq!(DROP_COUNT.load(Ordering::Relaxed), 5);
    }

    #[test]
    fn column_basic_ops() {
        let info = ComponentInfo::of::<f32>();
        let mut col = Column::new(info.type_id, info.layout, info.drop_fn);

        let vals: Vec<f32> = vec![1.0, 2.0, 3.0];
        for v in &vals {
            unsafe {
                col.push_raw(v as *const f32 as *const u8);
            }
        }

        assert_eq!(col.len(), 3);
        assert_eq!(col.type_id(), TypeId::of::<f32>());

        unsafe {
            let v = *(col.get_ptr(1) as *const f32);
            assert_eq!(v, 2.0);
        }
    }

    #[test]
    fn archetype_entity_management() {
        let infos = vec![
            ComponentInfo::of::<f32>(), // "Position X"
            ComponentInfo::of::<u32>(), // "Health"
        ];

        let mut arch = Archetype::new(0, &infos);
        assert!(arch.has_component(TypeId::of::<f32>()));
        assert!(arch.has_component(TypeId::of::<u32>()));
        assert!(!arch.has_component(TypeId::of::<u64>()));
        assert_eq!(arch.len(), 0);

        // Entity 42 ekle
        let pos: f32 = 10.0;
        let hp: u32 = 100;
        unsafe {
            arch.get_column_mut(TypeId::of::<f32>()).unwrap().push_raw(&pos as *const f32 as *const u8);
            arch.get_column_mut(TypeId::of::<u32>()).unwrap().push_raw(&hp as *const u32 as *const u8);
        }
        let row = arch.push_entity(42);
        assert_eq!(row, 0);
        assert_eq!(arch.len(), 1);
        assert_eq!(arch.entities()[0], 42);

        // Entity 99 ekle
        let pos2: f32 = 20.0;
        let hp2: u32 = 50;
        unsafe {
            arch.get_column_mut(TypeId::of::<f32>()).unwrap().push_raw(&pos2 as *const f32 as *const u8);
            arch.get_column_mut(TypeId::of::<u32>()).unwrap().push_raw(&hp2 as *const u32 as *const u8);
        }
        arch.push_entity(99);

        // row 0'ı çıkar (entity 42) → entity 99 row 0'a taşınmalı
        let moved = arch.swap_remove_entity(0);
        assert_eq!(moved, Some(99));
        assert_eq!(arch.len(), 1);
        assert_eq!(arch.entities()[0], 99);
    }

    #[test]
    fn archetype_edge_cache() {
        let infos = vec![ComponentInfo::of::<f32>()];
        let mut arch = Archetype::new(0, &infos);

        arch.set_add_edge(TypeId::of::<u32>(), 1);
        arch.set_remove_edge(TypeId::of::<f32>(), 2);

        let edge = arch.get_edge(TypeId::of::<u32>()).unwrap();
        assert_eq!(edge.add, Some(1));
        assert_eq!(edge.remove, None);

        let edge2 = arch.get_edge(TypeId::of::<f32>()).unwrap();
        assert_eq!(edge2.remove, Some(2));
    }

    #[test]
    fn entity_location_invalid() {
        let loc = EntityLocation::INVALID;
        assert!(!loc.is_valid());

        let loc2 = EntityLocation { archetype_id: 0, row: 5 };
        assert!(loc2.is_valid());
    }

    #[test]
    fn component_info_drop_detection() {
        // Copy type — drop_fn = None
        let info_u32 = ComponentInfo::of::<u32>();
        assert!(info_u32.drop_fn.is_none());

        // Drop type — drop_fn = Some
        let info_string = ComponentInfo::of::<String>();
        assert!(info_string.drop_fn.is_some());
    }

    #[test]
    fn blob_vec_swap_remove_move() {
        let layout = Layout::new::<u32>();
        let mut blob = BlobVec::new(layout, None);

        let values: Vec<u32> = vec![10, 20, 30, 40];
        for v in &values {
            unsafe { blob.push(v as *const u32 as *const u8); }
        }

        // index 1'i (20) çıkar ve out'a taşı
        let mut out: u32 = 0;
        unsafe {
            blob.swap_remove_unchecked(1, &mut out as *mut u32 as *mut u8);
        }
        assert_eq!(out, 20);
        assert_eq!(blob.len(), 3);

        // Sıra: [10, 40, 30]
        unsafe {
            assert_eq!(*(blob.get_unchecked(0) as *const u32), 10);
            assert_eq!(*(blob.get_unchecked(1) as *const u32), 40);
            assert_eq!(*(blob.get_unchecked(2) as *const u32), 30);
        }
    }
}
