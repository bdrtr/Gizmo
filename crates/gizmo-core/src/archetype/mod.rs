//! # Archetype Storage
//!
//! Aynı component bileşimine sahip entity'leri, sütun bazlı (SoA) bitişik bellekte
//! depolayan yüksek performanslı ECS depolama katmanı.
//!
//! ## Yapılar
//! - [`BlobVec`]  — Tip-silinmiş, hizalanmış vektör. Tek bir component sütunu için ham bellek.
//! - [`Column`]   — `BlobVec` + `TypeId` sarmalayıcı.
//! - [`Archetype`] — Birden fazla `Column` ve entity listesi barındıran tablo.
pub mod blob;
pub mod column;
pub mod index;

pub use self::blob::*;
pub use self::column::*;


use std::collections::HashMap;
use std::any::TypeId;
use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

/// Entity'nin World içindeki fiziksel konumu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntityLocation {
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

pub struct ArchetypeEdge {
    /// Bu component tipi eklenince hedef archetype
    pub add: Option<u32>,
    /// Bu component tipi çıkarılınca hedef archetype
    pub remove: Option<u32>,
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
    pub(crate) edges: HashMap<TypeId, ArchetypeEdge>,
}

impl Archetype {
    /// Belirtilen component tipleri için yeni boş archetype oluşturur.
    pub fn new(id: u32, component_infos: &[ComponentInfo]) -> Self {
        let mut column_indices = HashMap::with_capacity(component_infos.len());
        let mut columns = Vec::with_capacity(component_infos.len());

        for (idx, info) in component_infos.iter().enumerate() {
            column_indices.insert(info.type_id, idx);
            columns.push(RwLock::new(Column::new(info.type_id, info.layout, info.drop_fn, info.clone_fn)));
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

    /// Belirtilen iki satırın (row) verilerini ve entity kimliğini fiziksel olarak takaslar.
    pub(crate) unsafe fn swap_rows(&mut self, a: usize, b: usize) {
        if a == b {
            return;
        }
        // Tüm sütunlarda takas işlemini gerçekleştir
        for col_cell in &mut self.columns {
            col_cell.write().unwrap().swap_rows(a, b);
        }
        // Entity ID'lerini takasla
        self.entities.swap(a, b);
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
                let tick = src_col.ticks.swap_remove(source_row);
                dst_col.ticks.push(tick);
            } else {
                // Bu sütun kaynakta yok (yeni ekleniyor), yer ayırt ama veri yazma (caller yapacak)
                dst_col.ticks.push(ComponentTicks::new(0)); // Caller should update this tick
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

    /// Bir entity'nin bulunduğu satırı N kez kopyalar ve yeni entity kimliklerini ilişkilendirir.
    pub(crate) unsafe fn batch_clone_row(&mut self, row: usize, count: usize, new_eids: &[u32], tick: u32) -> Vec<u32> {
        if count == 0 { return Vec::new(); }
        
        for col_cell in &mut self.columns {
            let mut col = col_cell.write().unwrap();
            let src_ptr = col.get_ptr(row);
            col.push_cloned_batch(src_ptr, count, tick);
        }

        let mut new_rows = Vec::with_capacity(count);
        for &id in new_eids {
            new_rows.push(self.push_entity(id));
        }
        new_rows
    }

    /// Bellek boyutlarını (kapasite) aktif varlık sayısına göre daraltır.
    pub fn shrink_to_fit(&mut self) {
        self.entities.shrink_to_fit();
        for col_cell in &mut self.columns {
            col_cell.write().unwrap().shrink_to_fit();
        }
        self.edges.shrink_to_fit();
        self.column_indices.shrink_to_fit();
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// TESTS
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use std::alloc::Layout;
    use std::ptr;

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
        let mut col = Column::new(info.type_id, info.layout, info.drop_fn, info.clone_fn);

        let vals: Vec<f32> = vec![1.0, 2.0, 3.0];
        for v in &vals {
            unsafe {
                col.push_raw(v as *const f32 as *const u8, 1);
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
            arch.get_column_mut(TypeId::of::<f32>()).unwrap().push_raw(&pos as *const f32 as *const u8, 1);
            arch.get_column_mut(TypeId::of::<u32>()).unwrap().push_raw(&hp as *const u32 as *const u8, 1);
        }
        let row = arch.push_entity(42);
        assert_eq!(row, 0);
        assert_eq!(arch.len(), 1);
        assert_eq!(arch.entities()[0], 42);

        // Entity 99 ekle
        let pos2: f32 = 20.0;
        let hp2: u32 = 50;
        unsafe {
            arch.get_column_mut(TypeId::of::<f32>()).unwrap().push_raw(&pos2 as *const f32 as *const u8, 1);
            arch.get_column_mut(TypeId::of::<u32>()).unwrap().push_raw(&hp2 as *const u32 as *const u8, 1);
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
