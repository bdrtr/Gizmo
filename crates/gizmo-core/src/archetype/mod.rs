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

/// What one [`Archetype::move_entity_to`] actually did.
///
/// `moved` exists because the function does not take an entity — it takes a ROW, and moves
/// whoever is sitting in it. Every caller has an entity in mind and derives the row from that
/// entity's recorded location, so the two agree only while that location is fresh. When it is
/// stale the migration moves a stranger and the caller records the new row under its own id,
/// leaving two entities wrong and nothing to notice: one listed in an archetype its location
/// does not name, the other pointing at a row it does not occupy.
///
/// That is not hypothetical. Until 2026-08-31 a duplicated entity in an `insert_batch` slice
/// produced exactly it, silently, because the first pass moved the location the second pass then
/// read. The duplicate is de-duplicated now — and returning the id makes the assumption checkable
/// instead of assumed, so every call site `debug_assert_eq!`s it against the entity it meant.
/// In release the check is gone and nothing changes; in the test suite, under Miri and in CI, a
/// stale row is a named failure at the line that caused it rather than corruption found later.
pub(crate) struct Moved {
    /// The entity actually taken from `source_row` — whoever was there, not whoever was meant.
    pub(crate) moved: u32,
    /// Its row in the target archetype.
    pub(crate) new_row: u32,
    /// The entity swap-removed into the vacated source row, if the source was not the last row.
    /// Its own recorded row is now `source_row` and the caller has to store that.
    pub(crate) swapped: Option<u32>,
}

/// A row that has been removed from an archetype but whose values have not been dropped yet.
///
/// Returned by [`Archetype::detach_entity_row`] and consumed by
/// [`Archetype::drop_detached_row`]. Between the two the archetype is completely consistent and
/// the values belong to nobody, which is the entire point: the destructors — the only user code
/// in a removal, and the only thing that can unwind — run when there is no length left to
/// corrupt.
///
/// It carries `row` only so the second half can check it against the archetype's current length;
/// a token from another archetype, or one used after something pushed, is caught in debug.
#[must_use = "a detached row still owns its values"]
pub(crate) struct DetachedRow {
    /// The entity swap-moved into the vacated row, if it was not the last one. Its recorded row
    /// is now the vacated one and the caller has to store that.
    pub(crate) moved: Option<u32>,
    /// Where the detached values sit: the archetype's length after the removal.
    row: usize,
}

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

    /// The infallible half of a removal: the row stops existing, and its values are left where
    /// nothing can reach them.
    ///
    /// The whole archetype is FINAL on return — `entities` and every column are exactly one
    /// shorter, every visible row is coherent across every column, and the removed values sit at
    /// index `entities.len()`, above every length. Only [`Archetype::drop_detached_row`] can
    /// still reach them.
    ///
    /// This is a split rather than a reordering because neither half of the obvious answer works
    /// alone. The old body dropped column by column and popped `entities` afterwards, so a
    /// panicking destructor left the columns it had reached one shorter than `entities` — the
    /// direction `query::fetch` turns into an unchecked read, since it bounds itself by
    /// `Archetype::len()` and never looks at a column. Popping `entities` first fixes that and
    /// leaves the columns at *different* lengths from each other, permanently: the next
    /// `push_entity` lands the new value at a different index in each, and every row after it is
    /// off by one somewhere. Doing all the swapping in one infallible pass is what makes the
    /// post-unwind archetype exactly as consistent as a clean removal, for every column.
    ///
    /// # Panics
    /// If `row` is not a live row. That was an unchecked `entities.len() - 1` underflow in
    /// release before, feeding `usize::MAX` into the column operations.
    #[must_use = "the detached row still owns its values; pass the token to drop_detached_row"]
    pub(crate) fn detach_entity_row(&mut self, row: usize) -> DetachedRow {
        assert!(
            row < self.entities.len(),
            "detach_entity_row: row {row} of {} rows",
            self.entities.len()
        );
        let last = self.entities.len() - 1;
        // Byte swaps, `Vec::swap` over `Copy` ticks and one `Vec::swap` of ids. No drop glue, no
        // allocation, no panic point; `Archetype::swap_rows` early-returns when `row == last`.
        // SAFETY: both indices are live rows — `row` by the assertion above, `last` because
        // `entities` is non-empty.
        unsafe { self.swap_rows(row, last) };
        // The survivor now sits at `row`. Read before the truncation, which is what removes it
        // from the end.
        let moved = (row != last).then(|| self.entities[row]);
        // SAFETY: every column is `last + 1` long (the archetype invariant) and so is
        // `entities`, so `last` is a legal new length for all of them.
        unsafe { self.forget_rows_above(last) };
        DetachedRow { moved, row: last }
    }

    /// The fallible half: runs the removed row's drop glue, one column at a time.
    ///
    /// Every call here acts on a slot that is already outside every length in the archetype, so
    /// a panicking destructor leaks the columns it did not reach — safe — and moves no length at
    /// all. After the unwind the archetype is byte-for-byte as consistent as after a clean
    /// removal.
    ///
    /// # Safety
    /// `token` must have come from [`Archetype::detach_entity_row`] on **this** archetype, with
    /// nothing touching it in between, and must be used exactly once.
    pub(crate) unsafe fn drop_detached_row(&mut self, token: DetachedRow) {
        debug_assert_eq!(
            token.row,
            self.entities.len(),
            "drop_detached_row: the token does not describe this archetype's current state"
        );
        for cell in &self.columns {
            // SAFETY: `&mut self`, one visit per cell. The slot at `token.row` is the one
            // `detach_entity_row` put above the length and nothing else can see it.
            let col = unsafe { &mut *cell.get() };
            // SAFETY: `token.row` is the index `detach_entity_row` left the value at, above
            // this column's length, and each token is consumed exactly once.
            unsafe { col.data.drop_abandoned_at(token.row) };
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
    /// Pre-pays every allocation the staged-row protocol will need.
    ///
    /// After this, pushing one row into every column and one id into `entities` is a sequence of
    /// stores — no `realloc`, so no OOM abort and no unwind. That is what lets
    /// [`Archetype::commit_staged_row`] and [`Archetype::forget_rows_above`] be infallible, and
    /// their being infallible is what makes the abandon protocol sound.
    pub(crate) fn reserve_row(&mut self) {
        for cell in &self.columns {
            // SAFETY: `&mut self`, and each cell is visited once, so no two `&mut Column`
            // overlap. Reserving touches only the allocation, not any element.
            let col = unsafe { &mut *cell.get() };
            col.data.reserve(1);
            col.ticks.reserve(1);
        }
        self.entities.reserve(1);
    }

    /// Abandons every row at or above `len` — no drop glue, no allocation, no user code.
    ///
    /// The recovery action for a migration that was given up part-way. It does not need to know
    /// how far the caller got, and that is the point: everything above `len` is about to become
    /// unreachable, so whether a particular slot holds a live value or uninitialised bytes stops
    /// mattering. Live ones are leaked, which is safe; uninitialised ones are never touched,
    /// which is the whole problem being avoided.
    ///
    /// # Safety
    /// `len <= self.entities.len()` and `len <= col.data.len` for every column.
    pub(crate) unsafe fn forget_rows_above(&mut self, len: usize) {
        for cell in &self.columns {
            // SAFETY: as `reserve_row`. `ComponentTicks` is `Copy`, so truncating cannot run
            // user code either.
            let col = &mut *cell.get();
            col.data.forget_above(len);
            col.ticks.truncate(len);
        }
        self.entities.truncate(len);
    }

    /// Makes a staged row visible.
    ///
    /// `Archetype::len` is `entities.len()` and every query bounds itself by it, so a column
    /// that is longer than `entities` is a row no query can reach. Pushing the id is therefore
    /// the single moment the row exists — one store into capacity [`reserve_row`] already
    /// bought, which is why it cannot fail half-way.
    pub(crate) fn commit_staged_row(&mut self, entity_id: u32) {
        debug_assert!(
            self.columns.iter().all(|c| {
                // SAFETY: shared read of a length under `&mut self`.
                let col = unsafe { &*c.get() };
                col.data.len == self.entities.len() + 1
            }),
            "commit_staged_row: every column must hold exactly one row beyond `entities`"
        );
        self.entities.push(entity_id);
    }

    /// Runs the drop glue over the rows [`Archetype::stage_entity_into`] detached — the
    /// components the target archetype does not have, i.e. the ones the migration removed.
    ///
    /// Separate from the stage, and called only after the target's row is committed, because
    /// this is the migration's only user code: a destructor that panics here unwinds out of the
    /// caller, and everything the caller could still be holding half-done has to be finished
    /// first. Each value sits at its column's own `len`, out of range for every reader, so a
    /// panic leaks the ones not yet reached and moves no length at all.
    ///
    /// # Safety
    /// `target` must be the archetype the matching `stage_entity_into` staged into — the set of
    /// detached columns is re-derived from it — nothing may have pushed into this archetype in
    /// between, and this must run exactly once per stage.
    pub(crate) unsafe fn drop_detached_rows(&mut self, target: &Archetype) {
        for (type_id, &src_col_idx) in &self.column_indices {
            if !target.column_indices.contains_key(type_id) {
                // SAFETY: `&mut self`, one visit per column index. The value at `data.len` is the
                // one `detach_row` left above the length and nothing else can reach it.
                let src_col = unsafe { &mut *self.columns[src_col_idx].get() };
                // SAFETY: `data.len` is where `Column::detach_row` left the removed value —
                // above the length, so nothing else can reach it — and this pass runs once.
                unsafe { src_col.data.drop_abandoned_at(src_col.data.len) };
            }
        }
    }

    /// [`Archetype::stage_entity_into`] followed immediately by
    /// [`Archetype::commit_staged_row`] — the shape every caller wants that runs no user code
    /// between the two.
    ///
    /// `World::add_bundle` is the exception and calls the halves itself, because it has to write
    /// the bundle into the staged row before the row may exist. See there.
    pub(crate) unsafe fn move_entity_to(
        &mut self,
        source_row: usize,
        target: &mut Archetype,
    ) -> Moved {
        let moved = self.stage_entity_into(source_row, target);
        target.commit_staged_row(moved.moved);
        // The removed components' destructors, LAST — after the target's row exists. Run before
        // the commit they could unwind past it, leaving the target holding a staged row that
        // nothing commits and nothing abandons.
        // SAFETY: `target` is the archetype the stage above staged into, nothing has touched
        // either archetype in between, and this is the one disposal of those rows.
        unsafe { self.drop_detached_rows(target) };
        moved
    }

    /// Everything `move_entity_to` does EXCEPT making the target row visible.
    ///
    /// On return the target's columns each hold one row more than its `entities`, at index
    /// `Moved::new_row`. The row is addressable — `write_to_archetype` writes into it — and
    /// invisible to every query, because queries bound by `entities.len()`. Until
    /// [`Archetype::commit_staged_row`] runs it can be abandoned with
    /// [`Archetype::forget_rows_above`] at no cost and with no knowledge of what was written.
    ///
    /// The source is left fully consistent before this returns: its row is gone and its
    /// `entities` popped, so a caller that abandons the target does not have to undo the source.
    pub(crate) unsafe fn stage_entity_into(
        &mut self,
        source_row: usize,
        target: &mut Archetype,
    ) -> Moved {
        let entity_id = self.entities[source_row];

        // Buy every allocation up front so nothing below this line can fail on one.
        target.reserve_row();

        // 1. Hedef archetype'ın TÜM sütunlarını genişlet (ortak olanları taşı, olmayanları boş bırak)
        for (type_id, &dst_col_idx) in &target.column_indices {
            let dst_col = &mut *target.columns[dst_col_idx].get();

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

        // 2. DETACH the components the target does not have — the ones this migration is
        //    REMOVING. Their destructors are the only user code in the whole function, and they
        //    do not run here.
        //
        //    They used to, through `swap_remove_and_drop`, and at the worst possible moment:
        //    loop 1 has already shortened every SHARED source column to `n - 1` while
        //    `self.entities` is still `n`, so a panicking destructor escaped with the source
        //    archetype claiming one more row than most of its columns hold — the direction
        //    `query::fetch` turns into an unchecked read. Detaching is memcpy and two integer
        //    stores, so the source reaches its final shape below with nothing having been able
        //    to unwind.
        for (type_id, &src_col_idx) in &self.column_indices {
            if !target.column_indices.contains_key(type_id) {
                let src_col = &mut *self.columns[src_col_idx].get();
                src_col.detach_row(source_row);
            }
        }

        // 3. Kaynak archetype'tan entity listesini güncelle (sütunlar zaten swap_remove edildi)
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

        // 4. The SOURCE is now final: `entities` and every column are `n - 1`, and the removed
        //    values sit above that length where nothing can reach them. THE DROP GLUE IS THE
        //    CALLER'S, through `drop_detached_rows`, and it must not run until the target's row
        //    is committed — a destructor panicking here would otherwise unwind past
        //    `commit_staged_row` and leave the target holding a staged row nothing commits and
        //    nothing abandons.
        //
        // 5. The row is staged, not committed: the columns hold it, `target.entities` does not
        //    yet, so no query can see it. `commit_staged_row` is what makes it exist.
        Moved {
            moved: entity_id,
            new_row: target.entities.len() as u32,
            swapped: moved_entity,
        }
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

        // THE ROWS ARE ABANDONED IF A CLONE PANICS, and no reordering can replace that. The ids
        // are already pushed last, so the addition rule is satisfied; the problem is that
        // growing M columns by `count` user `Clone` calls is M separate fallible operations and
        // a panic in the k-th always leaves k-1 of them grown. The columns then disagree with
        // each other permanently — every later row lands at a different index in each, and no
        // query can see it because `entities` never moved. So the recovery is the same abandon
        // protocol the migration paths use.
        let base_len = self.entities.len();
        // Only `entities` needs pre-bought capacity: `push_cloned_batch_from_row` reserves its
        // own, and a failure there is inside the guard below.
        self.entities.reserve(count);
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            for col_cell in &self.columns {
                // SAFETY: forwarded from this function's contract — `row` is a live row and each
                // cell is visited once, so no two `&mut Column` overlap.
                let col = unsafe { &mut *col_cell.get() };
                // SAFETY: `row` is a live row of this column — forwarded from this function's
                // own contract — and the clones land above the current length.
                unsafe { col.push_cloned_batch_from_row(row, count, tick) };
            }
        }));
        if let Err(payload) = outcome {
            // SAFETY: `entities` was never touched, so `base_len` is still its length, and every
            // column is either `base_len` or `base_len + count` long — the panicking one repairs
            // itself, because `BlobVec` raises its length only after the whole clone loop.
            unsafe { self.forget_rows_above(base_len) };
            std::panic::resume_unwind(payload);
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
        let mut counts = Vec::with_capacity(self.columns.len());
        self.forget_all_rows(&mut counts);
        // SAFETY: `counts` is what `forget_all_rows` just produced for this archetype and
        // nothing has touched it since.
        unsafe { self.drop_forgotten_rows(&counts) };
    }

    /// The infallible half of [`Archetype::clear`]: `entities` and every column length go to
    /// zero, and the values are left uncounted. One count per column is pushed onto `out`, in
    /// `self.columns` order.
    ///
    /// **The count is each column's OWN length, not `entities.len()`.** Those agree in every
    /// healthy archetype, and this is exactly the function that must not assume it: it is the
    /// recovery a caller reaches for *after* something else left the archetype torn, and one
    /// shared count would then run drop glue over a row a shorter column had already disposed
    /// of. For the same reason there is no `debug_assert_consistent` here.
    pub(crate) fn forget_all_rows(&mut self, out: &mut Vec<usize>) {
        // The visibility switch first: after this line no query can reach any row, whatever
        // happens to the columns.
        self.entities.clear();
        for cell in &self.columns {
            // SAFETY: `&mut self`, one visit per cell, so no two `&mut Column` overlap.
            let col = unsafe { &mut *cell.get() };
            out.push(col.forget_rows());
        }
    }

    /// The fallible half: runs the drop glue over the rows [`Archetype::forget_all_rows`]
    /// abandoned. This is the only part that runs user code, and by the time it does, every
    /// length in the archetype is already final.
    ///
    /// # Safety
    /// `counts` must be what the matching `forget_all_rows` produced, in the same order, with
    /// nothing touching this archetype in between, and this must run exactly once for them.
    pub(crate) unsafe fn drop_forgotten_rows(&mut self, counts: &[usize]) {
        debug_assert_eq!(counts.len(), self.columns.len());
        for (cell, &n) in self.columns.iter().zip(counts) {
            // SAFETY: as above for the borrow; `n` is this column's own abandoned count.
            let col = unsafe { &mut *cell.get() };
            // SAFETY: `n` is what this same column returned from `forget_rows`, and nothing has
            // touched it since — the caller's contract on this function.
            unsafe { col.drop_forgotten_rows(n) };
        }
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
        let detached = arch.detach_entity_row(0);
        assert_eq!(detached.moved, Some(99));
        // The archetype is already final HERE, before any value is dropped — that is the whole
        // point of the split, and it is what a panicking destructor used to be able to break.
        assert_eq!(arch.len(), 1);
        assert_eq!(arch.entities()[0], 99);
        for cell in &arch.columns {
            // SAFETY: `&mut arch` is not held; this is a shared read of a length.
            assert_eq!(unsafe { (*cell.get()).len() }, 1);
        }
        // SAFETY: the token came from this archetype and nothing has touched it since.
        unsafe { arch.drop_detached_row(detached) };
        assert_eq!(arch.len(), 1);
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

    // ── `entities` is the visibility switch, and these three composites had it backwards. ──
    //
    // `Archetype::len()` IS `entities.len()`, and the query path bounds itself by that and then
    // indexes a column with no check in either profile — `query/fetch.rs` never calls
    // `BlobVec::get_unchecked`, so its `debug_assert` is not on the path. A column SHORTER than
    // `entities` is therefore read out of bounds by an ordinary `world.query::<&T>()`.
    //
    // Every one of these operations used to run its component destructors — user code, free to
    // panic — while its arrays still disagreed. Each test below arms exactly one destructor,
    // catches the unwind, and asserts the lengths rather than trying to provoke the read: the
    // assertion is deterministic in both profiles, and the read it prevents is not.
    //
    // A NOTE ON COLUMN ORDER, measured rather than assumed: `self.columns` is in SORTED `TypeId`
    // order, not declaration order — `World` builds every archetype from a `binary_search`ed
    // `Vec<TypeId>`. A test that needs "the bomb is column k" can only get it by constructing the
    // `Archetype` by hand, as these do, or by using a single component type.

    /// A component whose `Drop` panics once, plus a counter for a second type that must not be
    /// dropped twice. Built by hand so the column order is the declaration order.
    #[test]
    fn a_panicking_drop_during_a_removal_leaves_every_column_at_the_entity_count() {
        use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::SeqCst};

        static ARMED: AtomicBool = AtomicBool::new(false);
        static PAYLOAD_DROPS: AtomicU32 = AtomicU32::new(0);

        #[derive(Clone)]
        struct Fuse(&'static str);
        impl Component for Fuse {}
        impl Drop for Fuse {
            fn drop(&mut self) {
                if self.0 == "BOOM" && ARMED.swap(false, SeqCst) {
                    panic!("Fuse::drop");
                }
            }
        }
        #[derive(Clone)]
        struct Payload(#[allow(dead_code)] &'static str);
        impl Component for Payload {}
        impl Drop for Payload {
            fn drop(&mut self) {
                PAYLOAD_DROPS.fetch_add(1, SeqCst);
            }
        }

        // Declaration order IS column order here, so `Fuse` is column 0 and `Payload` — the one
        // that must not be reached — is column 1.
        let infos = vec![ComponentInfo::of::<Fuse>(), ComponentInfo::of::<Payload>()];
        let mut arch = Archetype::new(0, &infos);
        let push = |arch: &mut Archetype, f: Fuse, p: Payload, id: u32| {
            // SAFETY: test-local — the values match the layouts this archetype was built with,
            // and each is pushed exactly once into its own column.
            unsafe {
                arch.get_column_mut(TypeId::of::<Fuse>()).unwrap().push_raw(&f as *const Fuse as *const u8, 1);
                arch.get_column_mut(TypeId::of::<Payload>()).unwrap().push_raw(&p as *const Payload as *const u8, 1);
            }
            std::mem::forget(f);
            std::mem::forget(p);
            arch.push_entity(id)
        };
        push(&mut arch, Fuse("BOOM"), Payload("victim"), 7);
        push(&mut arch, Fuse("quiet"), Payload("survivor"), 9);
        PAYLOAD_DROPS.store(0, SeqCst);
        ARMED.store(true, SeqCst);

        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let detached = arch.detach_entity_row(0);
            // SAFETY: the token came from this archetype and nothing has touched it since.
            unsafe { arch.drop_detached_row(detached) };
        }));
        assert!(unwound.is_err(), "the drop was supposed to panic; the fixture is wrong if not");

        // THE ASSERTION. The removal is complete as far as anything can see: one entity left,
        // and every column exactly as long. Before the split, `entities` was popped after the
        // drop loop, so this was 2 against a `Fuse` column of 1.
        assert_eq!(arch.len(), 1);
        for cell in &arch.columns {
            // SAFETY: a shared read of a length; no `&mut Column` is alive here.
            assert_eq!(unsafe { (*cell.get()).len() }, arch.len(), "column/entities desync");
        }
        assert_eq!(arch.entities()[0], 9, "the survivor moved into the vacated row");
        // The victim's `Payload` was abandoned above the length rather than dropped — the
        // accepted leak on a panic path. A 1 here would mean the drop loop got past the fuse.
        assert_eq!(PAYLOAD_DROPS.load(SeqCst), 0);

        // And the archetype is still usable: nothing is torn, so teardown drops exactly the one
        // surviving row.
        drop(arch);
        assert_eq!(PAYLOAD_DROPS.load(SeqCst), 1);
    }

    /// `Archetype::clear` — the same shape, one operation wider.
    #[test]
    fn a_panicking_drop_during_a_clear_empties_every_column_on_the_unwind() {
        use std::sync::atomic::{AtomicBool, Ordering::SeqCst};

        static ARMED: AtomicBool = AtomicBool::new(false);

        #[derive(Clone)]
        struct Fuse(&'static str);
        impl Component for Fuse {}
        impl Drop for Fuse {
            fn drop(&mut self) {
                if self.0 == "BOOM" && ARMED.swap(false, SeqCst) {
                    panic!("Fuse::drop");
                }
            }
        }
        #[derive(Clone)]
        struct Tail(#[allow(dead_code)] &'static str);
        impl Component for Tail {}

        let infos = vec![ComponentInfo::of::<Fuse>(), ComponentInfo::of::<Tail>()];
        let mut arch = Archetype::new(0, &infos);
        for (i, tag) in ["BOOM", "quiet"].iter().enumerate() {
            // `ManuallyDrop`: the columns take the bytes, so these locals must not run drop
            // glue on any path.
            let f = std::mem::ManuallyDrop::new(Fuse(tag));
            let t = std::mem::ManuallyDrop::new(Tail("t"));
            // SAFETY: test-local, as above.
            unsafe {
                arch.get_column_mut(TypeId::of::<Fuse>()).unwrap().push_raw(&*f as *const Fuse as *const u8, 1);
                arch.get_column_mut(TypeId::of::<Tail>()).unwrap().push_raw(&*t as *const Tail as *const u8, 1);
            }
            arch.push_entity(i as u32);
        }
        ARMED.store(true, SeqCst);

        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| arch.clear()));
        assert!(unwound.is_err(), "the drop was supposed to panic; the fixture is wrong if not");

        // Every length went to zero BEFORE any destructor ran, so the unwind leaks the values it
        // did not reach and leaves nothing that can be indexed. Before the split, the `Tail`
        // column was still 2 long under an `entities` of 2, with the `Fuse` column at 0.
        assert_eq!(arch.len(), 0);
        for cell in &arch.columns {
            // SAFETY: a shared read of a length.
            assert_eq!(unsafe { (*cell.get()).len() }, 0, "a column outlived the clear");
        }
    }

    /// `Archetype::batch_clone_row` — the addition side, where the user code is `Clone`.
    ///
    /// The panic is triggered off PROGRESS rather than off which type it is. This archetype is
    /// hand-built, so column order really is declaration order and the test could have named a
    /// victim — but the property under test is "no column is left grown", which holds whichever
    /// column the panic lands in, and a progress trigger says that instead of assuming it.
    /// Whichever column comes first finishes its three clones; the second panics on its first.
    #[test]
    fn a_panicking_clone_during_a_batch_clone_abandons_every_grown_column() {
        use std::sync::atomic::{AtomicU32, Ordering::SeqCst};

        static CLONES: AtomicU32 = AtomicU32::new(0);

        #[derive(Default)]
        struct Counted(u32);
        impl Component for Counted {}
        impl Clone for Counted {
            fn clone(&self) -> Self {
                if CLONES.fetch_add(1, SeqCst) >= 3 {
                    panic!("Counted::clone");
                }
                Counted(self.0)
            }
        }
        #[derive(Default)]
        struct Other(u32);
        impl Component for Other {}
        impl Clone for Other {
            fn clone(&self) -> Self {
                if CLONES.fetch_add(1, SeqCst) >= 3 {
                    panic!("Other::clone");
                }
                Other(self.0)
            }
        }

        let infos = vec![ComponentInfo::of::<Counted>(), ComponentInfo::of::<Other>()];
        let mut arch = Archetype::new(0, &infos);
        let a = std::mem::ManuallyDrop::new(Counted(1));
        let b = std::mem::ManuallyDrop::new(Other(2));
        // SAFETY: test-local, as above.
        unsafe {
            arch.get_column_mut(TypeId::of::<Counted>()).unwrap().push_raw(&*a as *const Counted as *const u8, 1);
            arch.get_column_mut(TypeId::of::<Other>()).unwrap().push_raw(&*b as *const Other as *const u8, 1);
        }
        arch.push_entity(0);
        CLONES.store(0, SeqCst);

        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            // SAFETY: row 0 is live and `new_eids` has exactly `count` ids.
            unsafe { arch.batch_clone_row(0, 3, &[1, 2, 3], 1) }
        }));
        assert!(unwound.is_err(), "the clone was supposed to panic; the fixture is wrong if not");

        // The finished column is truncated back rather than left three rows longer than its
        // neighbour. Without the abandon protocol one column is 4 and the other 1, permanently,
        // and every later row lands at a different index in each.
        assert_eq!(arch.len(), 1);
        for cell in &arch.columns {
            // SAFETY: a shared read of a length.
            assert_eq!(unsafe { (*cell.get()).len() }, 1, "a half-grown column survived");
        }
    }
}
