//! [`Column`] — one component type's slice of an archetype — together with
//! [`ComponentTicks`] (change detection) and [`ComponentInfo`] (the runtime type record
//! used to build columns and to drive archetype migration).
//!
//! A column is a [`BlobVec`] of component bytes plus a parallel `Vec<ComponentTicks>`.
//! The two are always the same length and row `i` of one always describes row `i` of the
//! other; every mutating method here updates both together.

use super::blob::BlobVec;
use std::alloc::Layout;
use std::any::TypeId;
use std::ptr;

/// Change-detection timestamps for a single component instance, stored beside the
/// component itself — one entry per row, in the same order as the column data.
///
/// Both fields hold a *world tick*: a per-frame counter, not a wall-clock time. A tick
/// filter matches when its field is **strictly greater** than the query's reference tick,
/// so a component stamped with the reference tick itself does not match. The world counter
/// starts at 1 and skips 0 on wraparound, which makes 0 usable as a "before everything"
/// reference — but it *does* wrap, so ticks are only comparable within a window far shorter
/// than `u32::MAX` frames.
#[derive(Debug, Clone, Copy)]
pub struct ComponentTicks {
    /// Tick at which this component was written into its row.
    ///
    /// Nothing in this type advances `added` on its own; it changes only when the whole
    /// `ComponentTicks` value is replaced, i.e. when the row is (re)initialised. This is
    /// the field the `Added<T>` filter reads.
    pub added: u32,
    /// Tick of the most recent write to this component.
    ///
    /// Starts equal to [`added`](Self::added) (see [`ComponentTicks::new`]) and is bumped
    /// on its own afterwards, so `changed >= added` holds for any row that has not been
    /// reinitialised. This is the field the `Changed<T>` filter reads. Writes made through
    /// a raw pointer do *not* bump it — only the change-tracking access paths do.
    pub changed: u32,
}

impl ComponentTicks {
    /// Ticks for a freshly written component: both fields are set to `tick`, so the row is
    /// reported by `Added<T>` and `Changed<T>` over exactly the same window.
    ///
    /// Pass the current world tick. Passing 0 produces a row that no tick filter will ever
    /// match, since filters compare strictly greater than a reference tick of at least 0.
    pub fn new(tick: u32) -> Self {
        Self {
            added: tick,
            changed: tick,
        }
    }
}

/// The column of a single component type within the archetype.
pub struct Column {
    pub(crate) data: BlobVec,
    pub(crate) ticks: Vec<ComponentTicks>,
    type_id: TypeId,
    clone_fn: Option<unsafe fn(*const u8, *mut u8, usize)>,
}

impl Column {
    /// Creates a new empty column.
    pub fn new(
        type_id: TypeId,
        item_layout: Layout,
        drop_fn: Option<unsafe fn(*mut u8)>,
        clone_fn: Option<unsafe fn(*const u8, *mut u8, usize)>,
    ) -> Self {
        Self {
            data: BlobVec::new(item_layout, drop_fn),
            ticks: Vec::new(),
            type_id,
            clone_fn,
        }
    }

    /// `TypeId` of the component this column stores.
    ///
    /// Fixed at construction — a column never changes type — and it is the *only* type
    /// information the column carries. Every pointer accessor below is type-erased and
    /// trusts the caller to cast to this type; nothing here checks.
    #[inline]
    pub fn type_id(&self) -> TypeId {
        self.type_id
    }

    /// Number of rows stored, which is also the number of change-detection tick entries.
    ///
    /// Within an archetype this must equal the row count of every *other* column and the
    /// length of the entity list. That invariant is what makes a row index usable across
    /// columns; a bundle that writes only some of an archetype's columns breaks it and
    /// makes query iteration read uninitialised or out-of-bounds memory. It is an
    /// obligation on the writer, not something the column checks — nothing here validates
    /// it, in debug builds or otherwise.
    #[inline]
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Returns true if the column contains no elements.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Returns an immutable pointer to the component at the specified row.
    ///
    /// # Safety
    /// - `row < self.len()` must hold
    /// - The returned pointer points to valid data of type `T`
    #[inline]
    pub unsafe fn get_ptr(&self, row: usize) -> *const u8 {
        self.data.get_unchecked(row)
    }

    /// Returns the start pointer of the column data.
    #[inline]
    pub fn data_ptr(&self) -> *const u8 {
        self.data.as_ptr()
    }

    /// Returns the start pointer of the column data (mutable).
    #[inline]
    pub fn data_ptr_mut(&mut self) -> *mut u8 {
        self.data.as_mut_ptr()
    }

    /// The start address of the column's ComponentTick data.
    #[inline]
    pub fn ticks_ptr(&self) -> *const ComponentTicks {
        self.ticks.as_ptr()
    }

    /// The start address of the column's ComponentTick data (mutable).
    #[inline]
    pub fn ticks_ptr_mut(&mut self) -> *mut ComponentTicks {
        self.ticks.as_mut_ptr()
    }

    /// Returns a mutable pointer to the component at the specified row.
    ///
    /// # Safety
    /// - `row < self.len()` must hold
    /// - The returned pointer points to valid data of type `T`
    #[inline]
    pub unsafe fn get_mut_ptr(&self, row: usize) -> *mut u8 {
        self.data.get_unchecked_mut(row)
    }

    /// Swaps two rows within the column.
    ///
    /// # Safety
    /// - `a < self.len()` and `b < self.len()` must hold
    #[inline]
    pub unsafe fn swap_rows(&mut self, a: usize, b: usize) {
        self.data.swap_rows(a, b);
        self.ticks.swap(a, b);
    }

    /// Adds a new value as raw bytes.
    ///
    /// # Safety
    /// The `value` pointer must point to memory readable for this column's type size.
    #[inline]
    pub unsafe fn push_raw(&mut self, value: *const u8, tick: u32) {
        self.data.push(value);
        self.ticks.push(ComponentTicks::new(tick));
    }

    /// Takes a component reference and copies it N times back to back (Batch Prefab Cloning).
    ///
    /// # Safety
    /// - The `src` pointer must point to memory readable for this column's type size
    #[inline]
    pub unsafe fn push_cloned_batch(&mut self, src: *const u8, count: usize, tick: u32) {
        self.data.push_cloned_batch(src, count, self.clone_fn);
        self.ticks
            .resize(self.ticks.len() + count, ComponentTicks::new(tick));
    }

    /// Copies a component from the row it sits at (realloc safety).
    ///
    /// # Safety
    /// - `row < self.len()` must hold
    #[inline]
    pub unsafe fn push_cloned_batch_from_row(&mut self, row: usize, count: usize, tick: u32) {
        self.data
            .push_cloned_batch_from_row(row, count, self.clone_fn);
        self.ticks
            .resize(self.ticks.len() + count, ComponentTicks::new(tick));
    }

    /// Removes the specified row via swap-remove and drops it.
    ///
    /// # Safety
    /// `row < self.len()` must hold.
    #[inline]
    pub unsafe fn swap_remove_and_drop(&mut self, row: usize) {
        self.data.swap_remove_and_drop(row);
        self.ticks.swap_remove(row);
    }

    /// Removes the specified row via swap-remove, moves the value into `out`.
    ///
    /// # Safety
    /// - `row < self.len()` must hold
    /// - The `out` pointer must point to writable memory of sufficient size
    #[inline]
    pub unsafe fn swap_remove_move(&mut self, row: usize, out: *mut u8) {
        self.data.swap_remove_unchecked(row, out);
        self.ticks.swap_remove(row);
    }

    /// Compacts the column memory.
    pub fn shrink_to_fit(&mut self) {
        self.data.shrink_to_fit();
        self.ticks.shrink_to_fit();
    }

    /// Clears all the data in the column (without releasing the memory).
    pub fn clear(&mut self) {
        self.data.clear();
        self.ticks.clear();
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// COMPONENT INFO — Runtime tip bilgisi
// ═══════════════════════════════════════════════════════════════════════════

use crate::component::StorageType;

/// The runtime metadata of a component type.
/// Used when creating a Column and during archetype migration.
#[derive(Clone, Copy)]
pub struct ComponentInfo {
    /// Identity of the component type, and the key everything else is looked up by:
    /// archetype column indices, sparse-set registration and the world's component
    /// registry all hash on it.
    ///
    /// It is the one field [`ComponentInfo::of_type_id`] can fill in honestly — an info
    /// built that way has a real `type_id` and placeholders everywhere else.
    pub type_id: TypeId,
    /// Size and alignment of one instance.
    ///
    /// Every raw copy in the storage layer moves exactly `layout.size()` bytes, so an info
    /// whose layout disagrees with the real type silently corrupts memory. Zero-sized
    /// components are legal (`size() == 0`) and are stored without ever allocating.
    pub layout: Layout,
    /// Drop glue for one instance, called with a pointer to it.
    ///
    /// `None` means the type needs no drop glue (`std::mem::needs_drop` was false) and the
    /// storage may simply release its bytes. Since pushing into a column transfers
    /// ownership of the bytes, this is the only thing that will ever free a component's
    /// heap allocations — an info with a wrongly-`None` `drop_fn` leaks instead of crashing.
    pub drop_fn: Option<unsafe fn(*mut u8)>,
    /// Clone glue: `(src, dst, count)` writes `count` freshly cloned instances into
    /// consecutive slots starting at `dst`, which must be uninitialised and large enough
    /// for `count * layout.size()` bytes.
    ///
    /// Always `Some` for infos built by [`ComponentInfo::of`], because `Component` requires
    /// `Clone`; only [`ComponentInfo::of_type_id`] leaves it `None`. When it is `None`,
    /// `BlobVec::push_cloned_batch` degrades to a raw byte copy (wrong for any type owning
    /// a heap allocation) and `ComponentSparseSet::clone_entry` refuses and returns `false`.
    pub clone_fn: Option<unsafe fn(*const u8, *mut u8, usize)>,
    /// Which backing store holds this component: `Table` (an archetype [`Column`], laid out
    /// contiguously and iterated per archetype) or `SparseSet` (a `ComponentSparseSet` that
    /// lives outside the archetype and is keyed by entity id).
    ///
    /// Taken from `T::storage_type()` and therefore a property of the type, not of the
    /// entity. The two stores are disjoint: a component is in one or the other, never both,
    /// and the code paths that read them are separate.
    pub storage_type: StorageType,
    /// Human-readable type name (`std::any::type_name`). Captured at registration time so
    /// that the analysis/introspection layer (gizmo-analysis) can report archetype tables
    /// with component names. It CANNOT be recovered from a TypeId afterwards, hence it is here.
    pub type_name: &'static str,
}

impl ComponentInfo {
    /// Creates a ComponentInfo for the specified Rust type.
    pub fn of<T: crate::component::Component>() -> Self {
        Self {
            type_id: TypeId::of::<T>(),
            type_name: std::any::type_name::<T>(),
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
            storage_type: T::storage_type(),
        }
    }

    /// If only the TypeId is known (registry situations), creates a restricted ComponentInfo.
    pub fn of_type_id(type_id: TypeId) -> Self {
        Self {
            type_id,
            type_name: "<unknown>",
            layout: Layout::from_size_align(0, 1).unwrap(), // Geçici, gerçek layout registry'den gelmeli
            drop_fn: None,
            clone_fn: None,
            storage_type: StorageType::Table, // Varsayılan olarak Table (Registry tam bilgi bilmediğinde riskli olabilir ama şimdilik Table)
        }
    }
}
