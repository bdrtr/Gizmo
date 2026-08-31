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
    /// memory cost tracks the largest entity id inserted since the last
    /// [`clear`](ComponentSparseSet::clear) rather than the number of entries. It grows on
    /// insert; `remove` never shortens it, only writing the `u32::MAX` sentinel, and `clear` —
    /// which empties it outright — is the only thing that gives the memory back. An id past
    /// the end of this vector is simply absent, so every read must bounds-check before
    /// indexing.
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
    /// - `data_ptr` must point to a valid and aligned component instance compatible with this
    ///   set's `info.layout` value.
    /// - **Ownership transfers on ENTRY, on every path including an unwind out of this
    ///   function.** The caller must have relinquished the value *before* calling — a
    ///   `ManuallyDrop` around it, or a raw buffer it frees without dropping. Forgetting it
    ///   afterwards is not enough and has not been since 2026-08-31: the overwrite branch copies
    ///   the incoming bytes in before it drops what they replace, so a panic out of that `Drop`
    ///   leaves the set owning the new value while a `mem::forget` on the next line never runs —
    ///   a double free of the value that was just inserted.
    /// - `data_ptr` must **not** point into this set's own `dense`. The overwrite branch pushes,
    ///   and `BlobVec::push` may reallocate before it reads the source.
    pub unsafe fn insert(&mut self, entity: u32, data_ptr: *const u8, tick: u32) {
        let e = entity as usize;
        if e >= self.sparse.len() {
            self.sparse.resize(e + 1, u32::MAX);
        }

        let existing_row = self.sparse[e];
        if existing_row != u32::MAX {
            // ÜZERİNE YAZMA. The old value still has to be dropped — a component owning heap
            // memory would leak its allocation on re-insert otherwise — but it is dropped LAST,
            // out of a slot nothing counts any more.
            //
            // Dropping it in place first and copying over it afterwards is what this used to do,
            // and a component whose `Drop` panics made that undefined: the slot was left
            // destroyed while `dense.len`, `ticks`, `entities` and `sparse[e]` all still counted
            // it, so every later read handed out a dangling value and teardown dropped it again.
            // Nothing here is inconsistent enough for an assertion to notice, which is why the
            // shape survived the sweep that closed ten of its siblings.
            //
            // The fix needs no temporary buffer and no allocation, contrary to the note this
            // replaces: `dense`'s own tail is the temporary. Push the incoming value at the end,
            // then swap-remove the row — which puts the new value in place, moves the old one
            // past `len`, and only then runs its drop glue.
            let row = existing_row as usize;
            // Stamped before the drop: everything the set will show has to be final before user
            // code gets control.
            self.ticks[row].get_mut().changed = tick;
            // SAFETY: `data_ptr` points at a live value of this set's layout and ownership of it
            // transferred to this set on entry (see the contract above); it does not point into
            // `dense`, so the `reserve` inside `push` cannot invalidate it. No pointer is taken
            // across the push for the same reason.
            unsafe {
                self.dense.push(data_ptr);
            }
            // SAFETY: `row` came from `sparse`, which only ever holds live `dense` indices, and
            // the push above only made the vector longer. `swap_remove_and_drop` swaps the new
            // value at the end down into `row`, decrements `len` — restoring
            // `dense.len() == ticks.len() == entities.len()` — and drops the old value only
            // then. A panic out of that `Drop` therefore abandons bytes that are already out of
            // range: a leak, which is safe, and the set is left holding the new value.
            unsafe {
                self.dense.swap_remove_and_drop(row);
            }
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

        #[cfg(debug_assertions)]
        self.debug_assert_consistent();
    }

    /// Drops every entity's data and empties the set, keeping the four arrays in step.
    ///
    /// The values are **dropped**, not forgotten: `BlobVec::clear` runs the type's own
    /// `drop_fn` per element, so a component owning a heap allocation releases it. That is the
    /// whole reason this is a method here rather than four `.clear()` calls at the call site —
    /// the invariant that `dense`, `ticks`, `entities` and `sparse` agree is this type's, and
    /// `sparse` is the one that has to be emptied rather than truncated, since it is indexed by
    /// entity id and a stale `u32::MAX`-free slot would still claim a row.
    ///
    /// Added 2026-08-24 for [`World::clear_entities`](crate::world::World::clear_entities),
    /// which reset the archetypes and the id allocator and left these sets populated — so the
    /// first entity spawned afterwards took id 0 back and inherited whatever id 0 used to hold.
    ///
    /// **The three lookup arrays are emptied before `dense` is**, for the reason
    /// [`ComponentSparseSet::remove`] gives: `dense.clear()` runs every element's destructor and
    /// one of those can panic. `sparse` is this type's visibility switch — `contains`,
    /// `ticks_for`, `get_ptr`, `get_ptr_mut`, `remove`, `clone_entry` and `query::fetch`'s sparse
    /// branch all reach `dense` through it and nothing else — so emptying it first means a panic
    /// partway through the drops leaks what it did not reach and leaves all four arrays empty
    /// and in step. The old order left an empty `dense` under a fully populated `sparse`, where
    /// every entity in the set still claimed a row of a zero-length vector.
    pub fn clear(&mut self) {
        // `u32` and `ComponentTicks` have no drop glue: none of these three lines runs user code
        // or can unwind, and once they are done the set shows nothing.
        self.ticks.clear();
        self.entities.clear();
        self.sparse.clear();
        // The only user code in the function, and now the last thing in it. `BlobVec::clear`
        // zeroes its own length before it runs any destructor, so a panic here leaks rather than
        // double-drops — and the three arrays above are already empty, so the invariant holds
        // whether it completes or not. That is also why there is no `debug_assert_consistent`
        // call here: on the success path it could only compare `0 == 0`, and on the failure path
        // the unwind is out of the last statement, so nothing after it runs either way.
        self.dense.clear();
    }

    /// Gives back every byte this set is holding beyond what its live entries need.
    ///
    /// Two different reclamations, and the second is the one that matters:
    ///
    /// 1. `shrink_to_fit` on the four arrays, which returns the slack a `Vec`'s doubling growth
    ///    leaves behind — bounded by the number of entries, so at most a constant factor.
    /// 2. **Truncating [`sparse`](Self::sparse)**, which is bounded by the largest entity id ever
    ///    inserted instead. `insert` resizes it to `id + 1` and `remove` only writes the
    ///    `u32::MAX` sentinel, so a world that spawned a million entities carrying this component
    ///    and despawned all but the first holds a million-entry reverse index describing one
    ///    entry — 4 MB to say where entity 0 lives. Every trailing sentinel is pure absence, and
    ///    absence is what an id past the end already means, so they can go.
    ///
    /// Truncating is sound because every reader treats a short `sparse` as "not present":
    /// `contains`, `ticks_for`, `get_ptr`, `get_ptr_mut`, `remove` and `clone_entry` all
    /// bounds-check before indexing, and `insert` grows it back. Exactly one place indexes
    /// without a check — the sparse branch of `query::fetch`'s `get_item` — and it is reached
    /// only for an entity the query already matched through `contains`, whose entry is therefore
    /// non-sentinel and below the new length by construction.
    ///
    /// **That leaves one standing obligation on this crate rather than on this function.**
    /// `World::hierarchy_sort` also calls `get_item` directly, for a row it took from an
    /// archetype rather than from a `contains` check. It is safe today only because it fetches
    /// [`Children`](crate::component::Children), which is `StorageType::Table`, so the sparse
    /// branch is never entered. Giving `Children` — or anything else that walk fetches — SparseSet
    /// storage would index this vector blind for an entity that need not be in the set.
    ///
    /// The set itself is kept even when empty, and it is nearly free to keep: after this call an
    /// empty set holds **no heap allocation at all** (`BlobVec::shrink_to_fit` deallocates at
    /// `len == 0`, and an emptied `Vec` shrinks to capacity 0), so dropping the map entry would
    /// buy one `HashMap` slot. It would also change `WorldStats::sparse_set_components`, which
    /// reports how many sparse component types the world has. Registration itself is *not* the
    /// reason — that lives in `World::component_infos`; these entries are created lazily on the
    /// first insert, and every reader already handles a missing one with `?`.
    pub fn shrink_to_fit(&mut self) {
        // Everything past the last live entry is the `u32::MAX` sentinel, which says exactly
        // what running off the end of the vector says.
        let live_end = self
            .sparse
            .iter()
            .rposition(|&row| row != u32::MAX)
            .map_or(0, |i| i + 1);
        self.sparse.truncate(live_end);

        self.sparse.shrink_to_fit();
        self.entities.shrink_to_fit();
        self.ticks.shrink_to_fit();
        self.dense.shrink_to_fit();
    }

    /// Deletes an entity's data at O(1) speed.
    ///
    /// **The four arrays are repaired BEFORE the value's `Drop` runs**, and that ordering is the
    /// safety property rather than a style choice. It used to drop first —
    /// `BlobVec::swap_remove_and_drop` decrements its own length before it calls the drop glue,
    /// so `dense` repaired itself and `ticks`, `entities` and `sparse` did not. A panicking
    /// destructor then left `sparse[entity]` naming a row that now holds the *survivor's* value,
    /// and `sparse[last_entity]` naming `dense.len()` — one past the end. Both are indexed with
    /// no bounds check in release, by `get_ptr` and by `query::fetch`'s sparse branch. Fixed
    /// 2026-08-31, the same reordering as `Column::swap_remove_and_drop`.
    pub fn remove(&mut self, entity: u32) -> bool {
        let e = entity as usize;
        if e >= self.sparse.len() || self.sparse[e] == u32::MAX {
            return false; // Bulunamadı
        }

        let row = self.sparse[e] as usize;
        let last_row = self.dense.len() - 1;
        // Read before anything is permuted — `Vec::swap_remove` returns the VICTIM, not the
        // survivor, so this cannot be folded into the call below. It also turns an already
        // corrupt set into a clean index panic here rather than a wild offset inside `dense`.
        let last_entity = self.entities[last_row];

        // ── Everything from here to the drop is a `Copy` store or a `Vec` op on plain data:
        //    no drop glue, no user code, nothing that can unwind. ──
        self.ticks.swap_remove(row);
        self.entities.swap_remove(row);

        self.sparse[e] = u32::MAX;

        // Eğer silinen eleman dizinin sonundaki eleman değilse,
        // son sıradan alıp silinen yere taşıdığımız (swap) objenin sparse indexini güncelliyoruz.
        // When `row == last_row` the victim IS the survivor, so writing this would undo the
        // sentinel on the line above.
        if row != last_row {
            self.sparse[last_entity as usize] = row as u32;
        }

        // The set is final: three arrays of length `n - 1` that agree, and no `sparse` entry
        // names the victim's row. ONLY NOW the destructor.
        //
        // SAFETY: `row` is a live `dense` index (it came from `sparse`, checked above) and
        // nothing between the read and this line touched `dense`, so its length is still `n`.
        // `swap_remove_and_drop` puts the victim out of range before running its drop glue, so a
        // panic there abandons bytes nothing counts — a leak, which is safe.
        unsafe {
            self.dense.swap_remove_and_drop(row);
        }

        #[cfg(debug_assertions)]
        self.debug_assert_consistent();
        true
    }

    /// Debug-only invariant check: the three packed arrays must be the same length.
    ///
    /// `dense`, `ticks` and `entities` are one entry per stored component, and every desync in
    /// this family is a disagreement between them — a destructor that panicked between two of
    /// the updates. `get_ptr` and `query::fetch`'s sparse branch validate against `sparse` alone
    /// and then index `dense` unchecked, so nothing outside this type can see a violation coming.
    ///
    /// **`sparse` itself is deliberately NOT walked.** The reverse index is as long as the
    /// highest entity id ever inserted, so checking it on every `insert` and `remove` is `O(n)`
    /// per operation and quadratic over a batch — measured, not guessed: the CI Miri job's
    /// `sparse` filter went from 190 s to over six minutes with the walk in place. The length
    /// check is `O(1)` and catches the shape this exists for; the reverse index is checked by
    /// the regression test that needs it.
    #[cfg(debug_assertions)]
    pub(crate) fn debug_assert_consistent(&self) {
        debug_assert_eq!(self.dense.len(), self.ticks.len(), "sparse set: dense/ticks desync");
        debug_assert_eq!(
            self.dense.len(),
            self.entities.len(),
            "sparse set: dense/entities desync"
        );
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
        //
        // This is the second caller of `insert` and it already meets the stricter contract that
        // function documents. Ownership is relinquished on entry — nothing here would ever drop
        // `tmp`'s contents — and `tmp` is a fresh allocation rather than a pointer into `dense`,
        // which the push in the overwrite branch requires. If `insert` unwinds, the set owns the
        // clone and only the raw `tmp` block leaks; a leak on a panic path is safe.
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
