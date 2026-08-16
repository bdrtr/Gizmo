//! [`ComponentSparseSet`] — the alternative to archetype columns for components that are
//! added and removed far more often than they are iterated.
//!
//! One set holds exactly one component type for the whole world. Because the data lives
//! outside the archetype, adding or removing such a component does *not* migrate the entity
//! to another archetype (no row copy, no archetype churn); the cost is that iteration is a
//! per-entity indirection rather than a contiguous scan.

use crate::archetype::blob::BlobVec;
use crate::archetype::column::ComponentInfo;
use crate::archetype::ComponentTicks;
use std::cell::UnsafeCell;

/// A SparseSet makes it possible to manage component data from the outside, quickly, by Entity ID.
/// It is designed so that adding/deleting can be done directly without entering the archetype table.
pub struct ComponentSparseSet {
    /// Runtime type record of the single component type this set stores.
    ///
    /// `info.layout` and `info.drop_fn` are the ones `dense` was built with, so replacing
    /// this field after construction would desynchronise the storage from its own drop and
    /// copy glue. Every raw pointer the set hands out refers to a value of `info.type_id`.
    pub info: ComponentInfo,
    /// Packed component storage — one entry per entity in the set, no holes.
    ///
    /// Row `i` belongs to `entities[i]` and is described by `ticks[i]`; all three are kept
    /// at the same length. Removal is a swap-remove, so rows move: never cache a row index
    /// (or a pointer into `dense`) across an `insert` or `remove`.
    pub dense: BlobVec,
    /// Change-detection ticks. Interior-mutable via `UnsafeCell` — just like
    /// the raw-pointer interior mutability of `dense: BlobVec`: the `Mut<T>` query
    /// path (`query::fetch::get_item`) accesses the set through a SHARED `&self` and
    /// writes disjoint rows (necessary for parallel `par_for_each_mut`). Were it a
    /// plain `Vec<_>`, `as_ptr(&self)` would give read-only provenance → `&mut *ticks_ptr`
    /// would be aliasing UB (reachable from safe code). Writing through the Cell is sound.
    pub ticks: Vec<UnsafeCell<ComponentTicks>>,
    /// Owner of each dense row: `entities[row]` is the entity id stored at that row.
    ///
    /// Same length as `dense` and `ticks`, and permuted with them by every swap-remove.
    /// These are bare `Entity::id()` slot indices — the set records no generation, so it
    /// cannot by itself distinguish a recycled id from the original owner.
    pub entities: Vec<u32>, // dense row index -> Entity ID
    /// Reverse index: `sparse[entity_id]` is that entity's row in `dense`, or `u32::MAX`
    /// when the entity has no such component.
    ///
    /// Indexed *directly* by entity id, so its length is `highest inserted id + 1` and its
    /// memory cost tracks the largest entity id ever inserted rather than the number of
    /// entries. It grows on insert and is never shortened by `remove`, which only writes
    /// the `u32::MAX` sentinel. An id past the end of this vector is simply absent, so
    /// every read must bounds-check before indexing.
    pub sparse: Vec<u32>,   // Entity ID -> dense row index (Yoksa u32::MAX)
}

// SAFETY: `BlobVec`/`Archetype` üzerindeki aynı impl'lerle aynı gerekçe. İçsel-
// değişebilir alanlara (`dense` ham-pointer, `ticks` `UnsafeCell`) yalnız sorgu
// zamanlayıcısının ayrık-erişim garantisi altında yazılır → iki thread aynı satırı
// eşzamanlı yazmaz. `UnsafeCell<ComponentTicks>` eklenince otomatik `Sync` düştü.
// SAFETY: contents are component values, and `Component: Send + Sync`.
unsafe impl Send for ComponentSparseSet {}
// SAFETY: as above for the contents. The interior mutability (`dense`'s raw pointers, the
// `UnsafeCell<ComponentTicks>`) is written only under the query scheduler's disjoint-access
// guarantee, so two threads never write the same row — the same argument `BlobVec` and
// `Archetype` make, and the reason the automatic `Sync` was lost when the ticks became cells.
unsafe impl Sync for ComponentSparseSet {}

impl ComponentSparseSet {
    /// Creates an empty set for `info`'s component type.
    ///
    /// Allocates nothing — `dense` starts with a dangling pointer and `sparse` is empty —
    /// so a registered-but-unused component type costs only the struct itself. `info` is
    /// copied and fixed for the life of the set: one set stores one type, forever.
    pub fn new(info: ComponentInfo) -> Self {
        Self {
            info,
            dense: BlobVec::new(info.layout, info.drop_fn),
            ticks: Vec::new(),
            entities: Vec::new(),
            sparse: Vec::new(),
        }
    }

    /// Writes the data for an entity into the SparseSet. (Adds it or overwrites it).
    ///
    /// # Safety
    /// `data_ptr` must point to a valid and aligned component instance compatible with
    /// this set's `info.layout` value. Ownership of the pointer is transferred to the
    /// SparseSet (the caller must not additionally `drop` the copied value — see
    /// `std::mem::forget`).
    pub unsafe fn insert(&mut self, entity: u32, data_ptr: *const u8, tick: u32) {
        let e = entity as usize;
        if e >= self.sparse.len() {
            self.sparse.resize(e + 1, u32::MAX);
        }

        let existing_row = self.sparse[e];
        if existing_row != u32::MAX {
            // Zaten var, üzerine yaz
            let row = existing_row as usize;
            // SAFETY: `row` came from `sparse`, which only ever holds live `dense` indices, so it
            // is in range. `&mut self` here, so no other reference into `dense` is alive. The old
            // value is dropped before the overwrite a few lines down — without that, a component
            // owning heap memory would leak its allocation on re-insert.
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
            // SAFETY: `data_ptr` points at a value of exactly this set's component type and
            // layout (the caller's contract on `insert`), and `push` memcpys it and takes
            // ownership — the caller must not drop the source afterwards, which is the contract
            // `BlobVec::push` documents and every caller here honours with `mem::forget`.
            unsafe {
                self.dense.push(data_ptr);
            }
            self.ticks.push(UnsafeCell::new(ComponentTicks::new(tick)));
            self.entities.push(entity);
            self.sparse[e] = row;
        }
    }

    /// Deletes an entity's data at O(1) speed.
    pub fn remove(&mut self, entity: u32) -> bool {
        let e = entity as usize;
        if e >= self.sparse.len() || self.sparse[e] == u32::MAX {
            return false; // Bulunamadı
        }

        let row = self.sparse[e] as usize;
        let last_row = self.dense.len() - 1;

        // SAFETY: `row` is a live `dense` index (it came from `sparse`, checked above) and
        // `&mut self` excludes any other reference. `ticks` is swap-removed at the same index on
        // the next line, which is what keeps the two arrays in step.
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

    /// Whether `entity` currently has this component. `O(1)` and total: an id beyond the
    /// end of `sparse` — never inserted here, or belonging to some other component's set —
    /// is `false` rather than a panic.
    #[inline]
    pub fn contains(&self, entity: u32) -> bool {
        let e = entity as usize;
        e < self.sparse.len() && self.sparse[e] != u32::MAX
    }

    /// Returns an entity's change-detection ticks (`None` if absent).
    /// The `Changed<T>`/`Added<T>` filters use this for SparseSet components.
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

    /// Raw pointer to `entity`'s component, or `None` if the entity is not in this set.
    ///
    /// The pointer is untyped: dereferencing it requires casting to `info.type_id`'s type,
    /// which nothing here verifies. It is invalidated by the next mutation of the set —
    /// `insert` may reallocate `dense`, and `remove` swaps a different component into this
    /// address — so it must be consumed before any `&mut self` method runs.
    #[inline]
    pub fn get_ptr(&self, entity: u32) -> Option<*const u8> {
        let e = entity as usize;
        if e >= self.sparse.len() || self.sparse[e] == u32::MAX {
            return None;
        }
        // SAFETY: the index came from `sparse` and was checked against `u32::MAX` above, so it
        // addresses a live row. The returned pointer borrows nothing — the doc contract above
        // says it must be consumed before any `&mut self` method runs, because a `dense` growth
        // moves the allocation.
        unsafe { Some(self.dense.get_unchecked(self.sparse[e] as usize)) }
    }

    /// Mutable raw pointer to `entity`'s component, or `None` if absent. Same lookup and
    /// same invalidation rules as [`ComponentSparseSet::get_ptr`]; the `&mut self` receiver
    /// exists to enforce exclusivity, not because more work is done.
    ///
    /// Writing through this pointer does **not** touch the row's [`ComponentTicks`], so the
    /// mutation is invisible to `Changed<T>` unless the caller stamps `changed` itself.
    #[inline]
    pub fn get_ptr_mut(&mut self, entity: u32) -> Option<*mut u8> {
        let e = entity as usize;
        if e >= self.sparse.len() || self.sparse[e] == u32::MAX {
            return None;
        }
        // SAFETY: index validity as in `get_ptr`; `&mut self` here, so this is the only live
        // reference into `dense` while the pointer is produced.
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
        // SAFETY: `src_ptr` addresses the live row of `src` and `clone_fn` is this component
        // type's own cloner, so the read is well typed. The clone lands in a temporary buffer of
        // exactly `layout`, and `insert` then memcpys it and takes ownership — so the buffer is
        // freed WITHOUT dropping the moved-out value. `src_ptr` is consumed before `insert` can
        // reallocate `dense`, so it cannot dangle.
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
