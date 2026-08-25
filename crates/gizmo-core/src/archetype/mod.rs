//! # Archetype Storage
//!
//! A high-performance ECS storage layer that stores entities having the same component
//! composition in column-based (SoA) contiguous memory.
//!
//! ## Structures
//! - [`BlobVec`]  — Type-erased, aligned vector. Raw memory for a single component column.
//! - [`Column`]   — `BlobVec` + `TypeId` wrapper.
//! - [`Archetype`] — A table hosting more than one `Column` plus the entity list.
pub mod blob;
pub mod column;
/// Internal archetype registry: the sorted-component-set → archetype map, the vector that
/// owns the archetype tables, the entity → archetype lookup and the query match cache.
///
/// Archetype 0 is created up front for the empty component set and is never removed; every
/// entity starts there on spawn. Every other archetype id is a position in that vector, and
/// garbage-collecting an emptied archetype swap-removes it — renumbering the archetype that
/// was last, and patching the entity locations, the set map and the cached
/// [`ArchetypeEdge`](crate::archetype::ArchetypeEdge)s to match. An archetype id is therefore
/// an addressing slot, valid only until the next collection, never a durable identity.
///
/// Nothing here is part of the public API: every item inside is `pub(crate)` or private.
pub mod index;
pub mod sparse_set;

pub use self::blob::*;
pub use self::column::*;
pub use self::sparse_set::*;

use std::any::TypeId;
use std::collections::HashMap;
use std::cell::UnsafeCell;

/// The entity's physical location within the World.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntityLocation {
    /// Index of the archetype table holding this entity, as a position in the world's
    /// archetype vector — an addressing slot, not a durable identity. Compacting the
    /// archetype list renumbers it, so a location is only meaningful for as long as the
    /// world's structure is unchanged. `u32::MAX` is the invalid marker (see
    /// [`EntityLocation::INVALID`]).
    pub archetype_id: u32,
    /// Row index within the archetype
    pub row: u32,
}

impl EntityLocation {
    /// Sentinel meaning "this entity occupies no storage slot" — never spawned, or
    /// despawned. Both fields are `u32::MAX`; write this value when clearing a slot rather
    /// than invalidating one field, since [`is_valid`](Self::is_valid) only inspects
    /// `archetype_id`.
    pub const INVALID: Self = Self {
        archetype_id: u32::MAX,
        row: u32::MAX,
    };

    /// True unless `archetype_id` is `u32::MAX`.
    ///
    /// Only `archetype_id` is examined: a half-built location with a real archetype but
    /// `row == u32::MAX` reports valid and will then index out of bounds. Construct
    /// invalid locations from [`INVALID`](Self::INVALID), never field by field.
    #[inline]
    pub fn is_valid(self) -> bool {
        self.archetype_id != u32::MAX
    }
}

/// One cached archetype transition, stored in the owning archetype under the `TypeId` of
/// the component being added or removed.
///
/// It memoises the answer to "if I add (or remove) this component type, which archetype do
/// I become?", letting a repeated `add_component`/`remove_component` skip rebuilding and
/// re-hashing the sorted component set. Both directions are cached at once: taking an add
/// edge also records the reverse remove edge on the target archetype.
///
/// The two sides are independent — an edge may exist with only one of them populated, so a
/// hit on the map still has to check the direction actually wanted. `None` means "not
/// computed yet", never "no such archetype"; `Default` is the empty edge. The values are
/// indices into the world's archetype vector and are patched (or dropped) when that vector
/// is compacted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ArchetypeEdge {
    /// The target archetype when this component type is added
    pub add: Option<u32>,
    /// The target archetype when this component type is removed
    pub remove: Option<u32>,
}

// ═══════════════════════════════════════════════════════════════════════════
// ARCHETYPE — Sütun tablosu
// ═══════════════════════════════════════════════════════════════════════════

/// Column-based storage table for entities having the same component composition.
pub struct Archetype {
    /// This archetype's global index number
    pub id: u32,
    /// Component type → column index (in the columns vector)
    column_indices: HashMap<TypeId, usize>,
    /// The vector of columns — each one the data of a single component type.
    /// Wrapped in UnsafeCell because the engine's Scheduler (DAG) guarantees parallel access
    /// safety at compile/planning time, not at run time (RwLock).
    columns: Vec<UnsafeCell<Column>>,
    /// The entity IDs in this archetype (order = row index)
    entities: Vec<u32>,
    /// Component add/remove transition cache
    /// TypeId → ArchetypeEdge
    pub(crate) edges: HashMap<TypeId, ArchetypeEdge>,
}

// SAFETY: an archetype's columns hold component values only, and `Component: Send + Sync`, so
// its contents may cross threads. The `UnsafeCell<Column>` wrapper is what makes the type
// `!Sync` by default; it is there for INTERIOR MUTABILITY (a `&self` query handing out `&mut` to
// a disjoint column), and the no-aliasing half of that is the caller's contract on
// `get_column_mut` — upheld in this crate by the world's borrow tracking.
unsafe impl Send for Archetype {}
// SAFETY: as above — contents are `Sync` by the `Component` bound, and every `&self` route to a
// `&mut Column` goes through an `unsafe fn` whose contract forbids aliasing.
unsafe impl Sync for Archetype {}

impl Archetype {
    /// Creates a new empty archetype for the specified component types.
    pub fn new(id: u32, component_infos: &[ComponentInfo]) -> Self {
        let mut column_indices = HashMap::with_capacity(component_infos.len());
        let mut columns = Vec::with_capacity(component_infos.len());

        for (idx, info) in component_infos.iter().enumerate() {
            column_indices.insert(info.type_id, idx);
            columns.push(UnsafeCell::new(Column::new(
                info.type_id,
                info.layout,
                info.drop_fn,
                info.clone_fn,
            )));
        }

        Self {
            id,
            column_indices,
            columns,
            entities: Vec::new(),
            edges: HashMap::new(),
        }
    }

    /// The number of entities in this archetype
    #[inline]
    pub fn len(&self) -> usize {
        self.entities.len()
    }

    /// Debug-only invariant check: every column length must equal the entity count.
    /// The `write_to_archetype` contract: a bundle must write ALL of the archetype's columns;
    /// otherwise an `entities.len() != column.len()` desync occurs and query iteration
    /// reads out-of-bounds/uninitialized memory. This helper locks that invariant down.
    #[cfg(debug_assertions)]
    pub(crate) fn debug_assert_consistent(&self) {
        let n = self.entities.len();
        for cell in &self.columns {
            // SAFETY: a shared read of the column behind the cell, taken and dropped inside this
            // expression. `&mut self` is not held anywhere in this loop, so no `&mut Column`
            // handed out by `get_column_mut` can be alive at the same time.
            let col_len = unsafe { (*cell.get()).len() };
            debug_assert_eq!(
                col_len, n,
                "archetype column/entities desync: bir bundle tüm sütunları kapsamıyor olabilir"
            );
        }
    }

    /// Is it empty?
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    /// Physically swaps the data and the entity identity of the two specified rows.
    pub(crate) unsafe fn swap_rows(&mut self, a: usize, b: usize) {
        if a == b {
            return;
        }
        // Tüm sütunlarda takas işlemini gerçekleştir
        for col_cell in &self.columns {
            (&mut *col_cell.get()).swap_rows(a, b);
        }
        // Entity ID'lerini takasla
        self.entities.swap(a, b);
    }

    /// A reference to the entity ID list
    #[inline]
    pub fn entities(&self) -> &[u32] {
        &self.entities
    }

    /// Does this archetype contain the specified component type?
    #[inline]
    pub fn has_component(&self, type_id: TypeId) -> bool {
        self.column_indices.contains_key(&type_id)
    }

    /// The list of component types in this archetype
    pub fn component_types(&self) -> Vec<TypeId> {
        self.column_indices.keys().cloned().collect()
    }

    /// Sorted component types (used as the archetype identity)
    pub fn sorted_component_types(&self) -> Vec<TypeId> {
        let mut types = self.component_types();
        types.sort();
        types
    }

    /// Raw pointer access to the column of the specified component type
    #[inline]
    pub fn get_column(&self, type_id: TypeId) -> Option<&Column> {
        self.column_indices
            .get(&type_id)
            // SAFETY: `column_indices` only ever holds indices this archetype created for its own
            // `columns`, so the index is in range and the cell is live. The reference handed back
            // is shared and borrows `self`; a conflicting `&mut` can only come from the `unsafe`
            // `get_column_mut`, whose contract is exactly that it must not overlap with this.
            .map(|&idx| unsafe { &*self.columns[idx].get() })
    }

    /// Mutable access to the column of the specified component type (interior mutability via `UnsafeCell`).
    ///
    /// # Safety
    /// Returns a `&mut Column` through `&self`: this is a deliberate ECS pattern, and safety
    /// depends on more than one `&mut` never being produced for the same column at the same
    /// time. This invariant (disjoint access) must be upheld by the caller — in practice by
    /// the query scheduler's archetype-based disjoint access guarantee. Obtaining two live
    /// `&mut` for the same `type_id` is undefined behavior.
    // `mut_from_ref`: imza şekli (&self -> &mut) kasıtlı iç-değişebilirlik; güvenlik
    // `unsafe` kontratıyla çağırana devredildiği için lint bastırılıyor.
    #[allow(clippy::mut_from_ref)]
    #[inline]
    pub unsafe fn get_column_mut(&self, type_id: TypeId) -> Option<&mut Column> {
        self.column_indices
            .get(&type_id)
            // SAFETY: index validity as in `get_column`. The aliasing rule — no second live
            // reference to the SAME column — is this function's own documented contract and is
            // the caller's to keep; that is why it is an `unsafe fn`.
            .map(|&idx| unsafe { &mut *self.columns[idx].get() })
    }

    /// Adds a new entity row. Data must already have been pushed into all columns.
    /// Returns the index of the added row.
    #[inline]
    pub(crate) fn push_entity(&mut self, entity_id: u32) -> u32 {
        let row = self.entities.len() as u32;
        self.entities.push(entity_id);
        row
    }

    /// Removes the entity at the specified row via swap-remove.
    /// Returns the ID of the moved entity (the one previously in the last row).
    /// If the removed one was already in the last position, returns None.
    pub(crate) fn swap_remove_entity(&mut self, row: usize) -> Option<u32> {
        let last = self.entities.len() - 1;

        // Tüm sütunlarda swap_remove_and_drop
        for col_cell in &self.columns {
            // SAFETY: `&mut self` here, so no other reference into this archetype exists; each
            // cell is visited once, so the `&mut Column`s never overlap. `row` is in range —
            // `last` was computed from a non-empty `entities`, and every column is kept the same
            // length as `entities` (the invariant `debug_assert_consistent` checks).
            unsafe {
                (&mut *col_cell.get()).swap_remove_and_drop(row);
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

    /// Moves an entity's data from one archetype to another (Migration).
    ///
    /// `source_row` is the row in the source archetype, `target` the destination, and the
    /// return value is the new row in the target, paired with the source entity that got
    /// swapped into `source_row` (if any).
    ///
    /// **It returns with the target archetype in an INVALID state, and the caller must finish
    /// the job.** Every one of the target's columns is extended by one row, but only those the
    /// source also had are filled — the rest are left **uninitialised below `len`**, for the
    /// caller to write. Until it does, that archetype cannot be read, iterated, dropped or
    /// migrated out of: each of those would touch a hole.
    ///
    /// The alternative — leaving the new columns short and having the caller push — was
    /// rejected because it makes column length disagree with `entities` for the same window,
    /// and a short column is just as unusable as one with a hole. What matters is that the
    /// window exists at all, and this is the paragraph that says so.
    ///
    /// **The hole is invisible from inside the column**, which is the trap. `len` counts it, so
    /// `row < col.len()` does NOT mean "row holds a live value" — it means "row is inside the
    /// column", and after this call the two are different claims. A caller that reads the first
    /// as the second and drops the slot before writing it frees a garbage pointer; that is
    /// exactly what a fix to `add_bundle` did on 2026-08-24, and it turned an ordinary
    /// `spawn()` + `add_bundle(e, (T,))` into `free(): invalid pointer` for any `T` owning a
    /// heap allocation. The distinction is recoverable only *before* the move, from whether the
    /// SOURCE archetype had that column — see `live_after_move` in `World::add_bundle`.
    pub(crate) unsafe fn move_entity_to(
        &mut self,
        source_row: usize,
        target: &mut Archetype,
    ) -> (u32, Option<u32>) {
        let entity_id = self.entities[source_row];

        // 1. Hedef archetype'ın TÜM sütunlarını genişlet (ortak olanları taşı, olmayanları boş bırak)
        for (type_id, &dst_col_idx) in &target.column_indices {
            let dst_col = &mut *target.columns[dst_col_idx].get();

            // Hedefte her zaman yer açmalıyız ki sütun boyu entity listesiyle uyuşsun
            dst_col.data.reserve(1);
            let row_to_write = dst_col.data.len;
            dst_col.data.len += 1; // Önce boyutu artır ki get_unchecked_mut geçsin

            let dst_ptr = dst_col.data.get_unchecked_mut(row_to_write);

            if let Some(&src_col_idx) = self.column_indices.get(type_id) {
                let src_col = &mut *self.columns[src_col_idx].get();
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
                let src_col = &mut *self.columns[src_col_idx].get();
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

    /// Get the transition target for the specified type from the edge cache
    #[inline]
    pub fn get_edge(&self, type_id: TypeId) -> Option<&ArchetypeEdge> {
        self.edges.get(&type_id)
    }

    /// Add a new transition target to the edge cache
    #[inline]
    pub fn set_add_edge(&mut self, type_id: TypeId, target: u32) {
        let edge = self.edges.entry(type_id).or_insert(ArchetypeEdge {
            add: None,
            remove: None,
        });
        edge.add = Some(target);
    }

    /// Add a removal transition target to the edge cache
    #[inline]
    pub fn set_remove_edge(&mut self, type_id: TypeId, target: u32) {
        let edge = self.edges.entry(type_id).or_insert(ArchetypeEdge {
            add: None,
            remove: None,
        });
        edge.remove = Some(target);
    }

    /// Copies the row an entity occupies N times and associates the new entity identities.
    pub(crate) unsafe fn batch_clone_row(
        &mut self,
        row: usize,
        count: usize,
        new_eids: &[u32],
        tick: u32,
    ) -> Vec<u32> {
        if count == 0 {
            return Vec::new();
        }

        for col_cell in &self.columns {
            let col = &mut *col_cell.get();
            col.push_cloned_batch_from_row(row, count, tick);
        }

        let mut new_rows = Vec::with_capacity(count);
        for &id in new_eids {
            new_rows.push(self.push_entity(id));
        }
        new_rows
    }

    /// Shrinks the memory sizes (capacity) down to the number of active entities.
    pub fn shrink_to_fit(&mut self) {
        self.entities.shrink_to_fit();
        for col_cell in &self.columns {
            // SAFETY: `&mut self`, one visit per cell — no overlapping references. Shrinking
            // moves the allocation but keeps every live element, so rows stay valid.
            unsafe {
                (&mut *col_cell.get()).shrink_to_fit();
            }
        }
        self.edges.shrink_to_fit();
        self.column_indices.shrink_to_fit();
    }

    /// Quickly clears all row data in this archetype table.
    pub fn clear(&mut self) {
        for col_cell in &mut self.columns {
            // SAFETY: `&mut self`, one visit per cell. `clear` drops every live element and
            // leaves the column empty; `self.entities.clear()` below keeps the two in step.
            unsafe {
                (&mut *col_cell.get()).clear();
            }
        }
        self.entities.clear();
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// TESTS
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use crate::component::Component;
    #[derive(Clone, Copy)] #[allow(dead_code)] struct F32Comp(f32); impl Component for F32Comp {}
    #[derive(Clone, Copy)] #[allow(dead_code)] struct U32Comp(u32); impl Component for U32Comp {}
    #[derive(Clone)] #[allow(dead_code)] struct StringComp(String); impl Component for StringComp {}
    use super::*;
    use std::alloc::Layout;
    use std::ptr;

    #[test]
    fn blob_vec_push_and_read() {
        let layout = Layout::new::<u32>();
        let mut blob = BlobVec::new(layout, None);

        let values: Vec<u32> = vec![10, 20, 30, 40, 50];
        for v in &values {
            // SAFETY: test-local — the values were built here with the layout this storage was created
            // with, the rows used are the ones just pushed, and the test owns the storage outright.
            unsafe {
                blob.push(v as *const u32 as *const u8);
            }
        }

        assert_eq!(blob.len(), 5);

        for (i, expected) in values.iter().enumerate() {
            // SAFETY: test-local — the values were built here with the layout this storage was created
            // with, the rows used are the ones just pushed, and the test owns the storage outright.
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
            // SAFETY: test-local — the values were built here with the layout this storage was created
            // with, the rows used are the ones just pushed, and the test owns the storage outright.
            unsafe {
                blob.push(v as *const u64 as *const u8);
            }
        }

        // index 1'i (200) çıkar → son(400) onun yerine gelir
        // SAFETY: test-local — the values were built here with the layout this storage was created
        // with, the rows used are the ones just pushed, and the test owns the storage outright.
        unsafe {
            blob.swap_remove_and_drop(1);
        }
        assert_eq!(blob.len(), 3);

        // SAFETY: test-local — the values were built here with the layout this storage was created
        // with, the rows used are the ones just pushed, and the test owns the storage outright.
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
        // SAFETY: this dropper is paired with the layout of the type declared beside it, and the test is
        // its only caller.
        let drop_fn: unsafe fn(*mut u8) = |ptr| unsafe {
            ptr::drop_in_place(ptr as *mut Droppable);
        };

        {
            let mut blob = BlobVec::new(layout, Some(drop_fn));
            for i in 0..5 {
                let val = Droppable(i);
                // SAFETY: test-local — the values were built here with the layout this storage was created
                // with, the rows used are the ones just pushed, and the test owns the storage outright.
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
        let info = ComponentInfo::of::<F32Comp>();
        let mut col = Column::new(info.type_id, info.layout, info.drop_fn, info.clone_fn);

        let vals: Vec<f32> = vec![1.0, 2.0, 3.0];
        for v in &vals {
            // SAFETY: test-local — the values were built here with the layout this storage was created
            // with, the rows used are the ones just pushed, and the test owns the storage outright.
            unsafe {
                col.push_raw(v as *const f32 as *const u8, 1);
            }
        }

        assert_eq!(col.len(), 3);
        assert_eq!(col.type_id(), TypeId::of::<F32Comp>());

        // SAFETY: test-local — the values were built here with the layout this storage was created
        // with, the rows used are the ones just pushed, and the test owns the storage outright.
        unsafe {
            let v = *(col.get_ptr(1) as *const f32);
            assert_eq!(v, 2.0);
        }
    }

    #[test]
    fn archetype_entity_management() {
        let infos = vec![
            ComponentInfo::of::<F32Comp>(), // "Position X"
            ComponentInfo::of::<U32Comp>(), // "Health"
        ];

        let mut arch = Archetype::new(0, &infos);
        assert!(arch.has_component(TypeId::of::<F32Comp>()));
        assert!(arch.has_component(TypeId::of::<U32Comp>()));
        assert!(!arch.has_component(TypeId::of::<u64>()));
        assert_eq!(arch.len(), 0);

        // Entity 42 ekle
        let pos: F32Comp = F32Comp(10.0);
        let hp: U32Comp = U32Comp(100);
        // SAFETY: test-local — the values were built here with the layout this storage was created
        // with, the rows used are the ones just pushed, and the test owns the storage outright.
        unsafe {
            arch.get_column_mut(TypeId::of::<F32Comp>())
                .unwrap()
                .push_raw(&pos as *const F32Comp as *const u8, 1);
            arch.get_column_mut(TypeId::of::<U32Comp>())
                .unwrap()
                .push_raw(&hp as *const U32Comp as *const u8, 1);
        }
        let row = arch.push_entity(42);
        assert_eq!(row, 0);
        assert_eq!(arch.len(), 1);
        assert_eq!(arch.entities()[0], 42);

        // Entity 99 ekle
        let pos2: F32Comp = F32Comp(20.0);
        let hp2: U32Comp = U32Comp(50);
        // SAFETY: test-local — the values were built here with the layout this storage was created
        // with, the rows used are the ones just pushed, and the test owns the storage outright.
        unsafe {
            arch.get_column_mut(TypeId::of::<F32Comp>())
                .unwrap()
                .push_raw(&pos2 as *const F32Comp as *const u8, 1);
            arch.get_column_mut(TypeId::of::<U32Comp>())
                .unwrap()
                .push_raw(&hp2 as *const U32Comp as *const u8, 1);
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
        let infos = vec![ComponentInfo::of::<F32Comp>()];
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

        let loc2 = EntityLocation {
            archetype_id: 0,
            row: 5,
        };
        assert!(loc2.is_valid());
    }

    #[test]
    fn component_info_drop_detection() {
        // Copy type — drop_fn = None
        let info_u32 = ComponentInfo::of::<U32Comp>();
        assert!(info_u32.drop_fn.is_none());

        // Drop type — drop_fn = Some
        let info_string = ComponentInfo::of::<StringComp>();
        assert!(info_string.drop_fn.is_some());
    }

    #[test]
    fn blob_vec_swap_remove_move() {
        let layout = Layout::new::<u32>();
        let mut blob = BlobVec::new(layout, None);

        let values: Vec<u32> = vec![10, 20, 30, 40];
        for v in &values {
            // SAFETY: test-local — the values were built here with the layout this storage was created
            // with, the rows used are the ones just pushed, and the test owns the storage outright.
            unsafe {
                blob.push(v as *const u32 as *const u8);
            }
        }

        // index 1'i (20) çıkar ve out'a taşı
        let mut out: u32 = 0;
        // SAFETY: test-local — the values were built here with the layout this storage was created
        // with, the rows used are the ones just pushed, and the test owns the storage outright.
        unsafe {
            blob.swap_remove_unchecked(1, &mut out as *mut u32 as *mut u8);
        }
        assert_eq!(out, 20);
        assert_eq!(blob.len(), 3);

        // Sıra: [10, 40, 30]
        // SAFETY: test-local — the values were built here with the layout this storage was created
        // with, the rows used are the ones just pushed, and the test owns the storage outright.
        unsafe {
            assert_eq!(*(blob.get_unchecked(0) as *const u32), 10);
            assert_eq!(*(blob.get_unchecked(1) as *const u32), 40);
            assert_eq!(*(blob.get_unchecked(2) as *const u32), 30);
        }
    }
}
