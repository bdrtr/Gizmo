//! Type-erased raw storage: [`BlobVec`], the untyped growable array that backs a single
//! component column and the dense half of a sparse set.
//!
//! Nothing in this module knows what type it is holding. Every element operation is
//! `unsafe` and carries the same contract: pointers must refer to a value laid out exactly
//! like the `Layout` the vec was constructed with, and indices must be in range. Bounds are
//! `debug_assert`ed only — a release build computes an out-of-bounds address instead of
//! panicking.

use std::alloc::{self, Layout};
use std::ptr::{self, NonNull};

/// An untyped `Vec`: a manually allocated, contiguous array of elements whose size and
/// alignment are known only at runtime, with no type parameter and no per-element
/// bookkeeping.
///
/// **Ownership.** [`push`](BlobVec::push) *memcpy*s the bytes and takes ownership of them;
/// the caller must not drop the source afterwards (`std::mem::forget` it) or the value is
/// dropped twice. Conversely [`swap_remove_unchecked`](BlobVec::swap_remove_unchecked)
/// moves a value *out* without dropping it — the receiver of `out` becomes the owner.
///
/// The clone-batch pushes are the exception: [`push_cloned_batch`](BlobVec::push_cloned_batch)
/// and [`push_cloned_batch_from_row`](BlobVec::push_cloned_batch_from_row) *read* their
/// source (through `clone_fn`, or as a repeated byte copy when there is none) and never
/// consume it. For `push_cloned_batch` that source is the caller's pointer, so the caller
/// still owns the value afterwards and is the one who has to drop it;
/// `push_cloned_batch_from_row` reads a row this vec already owns, so there is nothing on
/// the caller's side to release.
///
/// **Ordering.** New elements are appended at the end, but an index is not a stable handle
/// on the value stored through it. Removal is swap-remove — removing anything but the last
/// element moves the last element into the hole — and
/// [`swap_rows`](BlobVec::swap_rows) permutes two live elements with no removal involved.
/// Treat an index as valid only until the next mutating call, and keep any parallel arrays
/// (ticks, entity ids) in step yourself.
///
/// **Zero-sized elements** are supported and never allocate — not on construction, not on
/// `reserve`, and nothing is freed on drop. The data pointer stays at
/// `NonNull::<u8>::dangling()` (it is never dereferenced, and is not adjusted for an
/// over-aligned element type). `capacity` carries no meaning in this case: it starts at
/// `usize::MAX` and is not maintained afterwards, so read `len` and ignore it.
///
/// **Growth** is amortised — reserving at least doubles the capacity, with a floor of four
/// elements — and reallocates in place where the allocator can, so any pointer obtained
/// before an insertion may dangle after it. If `item_size * capacity` would overflow
/// `usize` or exceed `isize::MAX`, allocation panics rather than wrapping.
///
/// Dropping the vec runs `drop_fn` over the live elements and then frees the block; when
/// `drop_fn` is `None` (a component that needs no drop glue) the bytes are simply released.
pub struct BlobVec {
    /// The memory layout of each element (size + alignment)
    item_layout: Layout,
    /// Destructor function — if None, no drop is needed (Copy types)
    drop_fn: Option<unsafe fn(*mut u8)>,
    /// The start of the allocated memory block
    data: NonNull<u8>,
    /// The current element count
    pub(crate) len: usize,
    /// The allocated capacity (in elements)
    pub(crate) capacity: usize,
}

// SAFETY: a `BlobVec` only ever holds COMPONENT values (columns and sparse sets are its only
// users), and `Component: Send + Sync` — so every byte it owns is already safe to move to
// another thread. That bound, not the access pattern, is what makes this sound.
unsafe impl Send for BlobVec {}
// SAFETY: same bound gives `Sync` for the contents. The type itself synchronises nothing and
// promises nothing: `get_unchecked_mut(&self)` hands out a `*mut` through a shared reference,
// which is exactly why it is an `unsafe fn` — the no-aliasing half of the contract is the
// caller's (module docs), and the world's borrow tracking is what upholds it in this crate.
unsafe impl Sync for BlobVec {}

/// Converts the `item_size * count` product into a `Layout` in an overflow-safe way.
///
/// If the product overflows `usize` or exceeds `isize::MAX` (which is the maximum
/// allocation size Rust permits), it returns `None` instead of wrapping/panicking.
#[inline]
fn checked_array_layout(item_size: usize, count: usize, align: usize) -> Option<Layout> {
    let total = item_size.checked_mul(count)?;
    Layout::from_size_align(total, align).ok()
}

impl BlobVec {
    /// Creates a new empty BlobVec.
    ///
    /// # Arguments
    /// * `item_layout` — The Layout of each element (size + alignment)
    /// * `drop_fn` — The element-dropping function. If `None`, drop is not called.
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

    /// The current element count
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Is it empty?
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Returns the raw pointer of the element at the specified index.
    ///
    /// # Safety
    /// `index < self.len` must hold.
    #[inline]
    pub unsafe fn get_unchecked(&self, index: usize) -> *const u8 {
        debug_assert!(
            index < self.len,
            "BlobVec::get_unchecked: index {} >= len {}",
            index,
            self.len
        );
        self.data.as_ptr().add(index * self.item_layout.size())
    }

    /// Returns the mutable raw pointer of the element at the specified index.
    ///
    /// # Safety
    /// `index < self.len` must hold.
    #[inline]
    pub unsafe fn get_unchecked_mut(&self, index: usize) -> *mut u8 {
        debug_assert!(
            index < self.len,
            "BlobVec::get_unchecked_mut: index {} >= len {}",
            index,
            self.len
        );
        self.data.as_ptr().add(index * self.item_layout.size())
    }

    /// Adds a new element.
    ///
    /// # Safety
    /// The `value` pointer must point to memory readable for `item_layout.size()` bytes.
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

    /// Duplicates a component N times using the clone function and appends them at the end.
    ///
    /// # Safety
    /// The `src` pointer must point to memory readable for `item_layout.size()` bytes.
    pub unsafe fn push_cloned_batch(
        &mut self,
        src: *const u8,
        count: usize,
        clone_fn: Option<unsafe fn(*const u8, *mut u8, usize)>,
    ) {
        if count == 0 {
            return;
        }
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

    /// Takes a component from the index it sits at, duplicates it N times and appends at the end.
    /// Prevents the src pointer from being dangling during realloc.
    ///
    /// # Safety
    /// `row < self.len` must hold.
    pub unsafe fn push_cloned_batch_from_row(
        &mut self,
        row: usize,
        count: usize,
        clone_fn: Option<unsafe fn(*const u8, *mut u8, usize)>,
    ) {
        if count == 0 {
            return;
        }
        if self.item_layout.size() == 0 {
            self.len += count;
            return;
        }
        self.reserve(count);
        // Reserve sonrasi pointer guncellenir:
        let src = self.get_unchecked(row);
        let dst_start = self.data.as_ptr().add(self.len * self.item_layout.size());

        if let Some(c_fn) = clone_fn {
            c_fn(src, dst_start, count);
        } else {
            // fallback
            let size = self.item_layout.size();
            let mut current_dst = dst_start;
            for _ in 0..count {
                ptr::copy_nonoverlapping(src, current_dst, size);
                current_dst = current_dst.add(size);
            }
        }
        self.len += count;
    }

    /// Swaps the raw memory contents of two rows (Swap).
    /// It is quite effective for cache-friendly memory shifts such as those in the hierarchy.
    ///
    /// # Safety
    /// `a < self.len` and `b < self.len` must hold.
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

    /// Returns the raw pointer of the data area.
    pub fn as_ptr(&self) -> *const u8 {
        self.data.as_ptr()
    }

    /// Returns the mutable raw pointer of the data area.
    pub fn as_mut_ptr(&mut self) -> *mut u8 {
        self.data.as_ptr()
    }

    /// Removes the element at the specified index by swap-and-pop (the last element is moved
    /// into its place) and drops the old value.
    ///
    /// # Safety
    /// `index < self.len` must hold.
    /// **The bookkeeping happens before the drop**, which is what makes this safe when the
    /// element's own `Drop` panics. It used to drop first and decrement afterwards, so a panic
    /// left a destroyed value still counted inside `len` — and `Drop for BlobVec` then dropped it
    /// a second time, which is undefined behaviour. The victim is swapped to the end and put out
    /// of range first now, so a panic leaks it instead. `Vec::swap_remove` has the same shape.
    /// Fixed 2026-08-31.
    ///
    /// The swap is a byte swap rather than the old copy-over: copying the last element onto the
    /// victim before dropping it would overwrite a live value, and dropping the victim before
    /// copying is the ordering this is fixing.
    pub unsafe fn swap_remove_and_drop(&mut self, index: usize) {
        debug_assert!(index < self.len);
        let last = self.len - 1;
        let item_size = self.item_layout.size();

        if index != last {
            // Both pointers are into this vec's own allocation at distinct indices, so the
            // regions are non-overlapping. `get_unchecked_mut` hands out a raw pointer, so no
            // two `&mut` exist at once.
            let a = self.get_unchecked_mut(index);
            let b = self.get_unchecked_mut(last);
            ptr::swap_nonoverlapping(a, b, item_size);
        }

        // The victim now sits at `last`, and this puts it out of range. Everything still inside
        // `len` is live and untouched, so the drop below cannot leave the vec inconsistent
        // however it ends.
        self.len -= 1;

        if let Some(drop_fn) = self.drop_fn {
            // SAFETY: `last` was a live index on entry and holds the victim after the swap; it
            // is now beyond `len`, so nothing else will ever look at it again.
            let ptr = self.data.as_ptr().add(last * item_size);
            drop_fn(ptr);
        }
    }

    /// Appends a **bytewise duplicate** of the element at `row`, without cloning it.
    ///
    /// After this call the same value exists twice: once at `row` and once at the new last
    /// index. That is deliberately not a state this type can be left in — exactly one of the two
    /// must eventually be dropped and the other abandoned (see [`BlobVec::forget_above`]) — and
    /// it is the whole point. It lets a caller that is about to overwrite a live slot with a
    /// write it does not control put the old value somewhere *outside* the visible range first,
    /// so that whatever the write does, no slot is ever both counted and destroyed.
    ///
    /// Added 2026-08-31 for [`World::add_bundle`](crate::world::World::add_bundle)'s
    /// same-archetype branch: it duplicates the row it is about to overwrite, lets
    /// `Bundle::write_to_archetype` write over the live copy exactly as before, and then drops
    /// the duplicate — which is a `swap_remove_and_drop` of the last index, and therefore does
    /// its bookkeeping before it runs any user code. A panic anywhere in between leaves every
    /// live slot holding either its original value or the new one, and the duplicates above
    /// `len` are abandoned.
    ///
    /// The `reserve` happens **before** the source pointer is taken. Taking it first and pushing
    /// afterwards would read through a pointer that `grow`'s `realloc` had already moved.
    ///
    /// # Safety
    /// - `row < self.len` must hold, and the slot must be live.
    /// - The caller must ensure exactly one of the two copies is dropped, and that the other is
    ///   forgotten rather than dropped.
    pub(crate) unsafe fn push_copy_of_row(&mut self, row: usize) {
        debug_assert!(row < self.len);
        let item_size = self.item_layout.size();
        if item_size == 0 {
            self.len += 1;
            return;
        }
        self.reserve(1);
        // Taken AFTER `reserve`, which is the only step that can move the allocation.
        let src = self.get_unchecked(row);
        let dst = self.data.as_ptr().add(self.len * item_size);
        ptr::copy_nonoverlapping(src, dst, item_size);
        self.len += 1;
    }

    /// Runs the element's own drop glue on the value at `index`, leaving the slot
    /// **uninitialised** and the length unchanged.
    ///
    /// The odd one out among the removal methods: it does not shorten the vec, so on return
    /// there is a hole below `len`, and reading it, dropping it again, or letting the vec's own
    /// `Drop` run over it is undefined. The caller must write a fresh value into that slot
    /// before anything else touches the column.
    ///
    /// Added 2026-08-25 for [`World::add_bundle`](crate::world::World::add_bundle), whose bundle
    /// write copies raw bytes and so cannot drop what it replaces. That same write also lands on
    /// the freshly allocated holes `Archetype::move_entity_to` leaves behind, where dropping
    /// would be undefined — so "is this slot live?" is a question only the caller can answer, and
    /// this is how it acts on the answer.
    ///
    /// # Safety
    /// - `index < self.len` must hold.
    /// - The slot must hold a **live** value. Calling this twice on one slot without an
    ///   intervening write is a double free.
    /// - The caller must initialise the slot before it is read, dropped or moved.
    pub unsafe fn drop_in_place_at(&mut self, index: usize) {
        debug_assert!(index < self.len);
        if let Some(drop_fn) = self.drop_fn {
            drop_fn(self.get_unchecked_mut(index));
        }
    }

    /// Removes the element at the specified index by swap-and-pop (the last element is moved
    /// into its place), moving the old value out to the `out` pointer (does not drop it).
    ///
    /// # Safety
    /// - `index < self.len` must hold
    /// - The `out` pointer must point to memory writable for `item_layout.size()` bytes
    pub unsafe fn swap_remove_unchecked(&mut self, index: usize, out: *mut u8) {
        debug_assert!(index < self.len);
        let last = self.len - 1;

        // Çıkarılan elemanı out'a kopyala
        let src = self.get_unchecked(index);
        ptr::copy_nonoverlapping(src, out, self.item_layout.size());

        if index != last {
            // Son elemanı çıkarılan yere taşı
            let last_src = self.get_unchecked(last);
            let dst = self.get_unchecked_mut(index);
            ptr::copy_nonoverlapping(last_src, dst, self.item_layout.size());
        }

        self.len -= 1;
    }

    /// Grow if there is not enough capacity.
    pub(crate) fn reserve(&mut self, additional: usize) {
        let required = self.len + additional;
        if required <= self.capacity {
            return;
        }

        let new_capacity = required.max(self.capacity * 2).max(4);
        self.grow(new_capacity);
    }

    /// Grow the capacity to the specified value.
    fn grow(&mut self, new_capacity: usize) {
        assert!(new_capacity > self.capacity);
        let item_size = self.item_layout.size();
        if item_size == 0 {
            return;
        }

        let new_layout =
            checked_array_layout(item_size, new_capacity, self.item_layout.align())
                .expect("BlobVec::grow: Layout overflow (allocation too large)");

        let new_data = if self.capacity == 0 {
            // İlk tahsis
            // SAFETY: `new_layout` came from `checked_array_layout`, and the ZST case returned
            // above — so the layout has non-zero size, which is `alloc`'s one requirement. The
            // null it may return is handled by the `NonNull::new(..).expect(..)` below.
            unsafe { alloc::alloc(new_layout) }
        } else {
            // Yeniden tahsis
            let old_layout =
                checked_array_layout(item_size, self.capacity, self.item_layout.align())
                    .expect("BlobVec::grow: Old layout overflow");
            // SAFETY: `capacity > 0` on this arm, so `self.data` is a live allocation made by
            // this same allocator with exactly `old_layout` (recomputed here from the fields that
            // produced it), and `new_layout.size()` is non-zero. Null is handled below.
            unsafe { alloc::realloc(self.data.as_ptr(), old_layout, new_layout.size()) }
        };

        self.data = NonNull::new(new_data).expect("BlobVec::grow: Allocation failed (OOM)");
        self.capacity = new_capacity;
    }

    /// Shrink (Defragmentation) operation. Makes the BlobVec's capacity value equal to its len value.
    pub fn shrink_to_fit(&mut self) {
        if self.capacity == self.len {
            return;
        }
        let item_size = self.item_layout.size();
        if item_size == 0 {
            // A ZST vec has no allocation to shrink, and its capacity is the `usize::MAX`
            // sentinel `new` sets to mean "never needs to grow" — which `grow` relies on by
            // returning early for ZSTs without updating it. Writing `capacity = len` here, as
            // this used to, contradicted that sentinel for the rest of the vec's life. Harmless
            // in practice, because `push` short-circuits on ZSTs before consulting capacity at
            // all, but `compact` runs this over every ZST marker column on every GC tick, so
            // the invariant was false almost everywhere. Leave it alone. Fixed 2026-08-31.
            return;
        }

        if self.len == 0 {
            // Tamamen boşalt, belleği dealloc yap.
            let old_layout =
                checked_array_layout(item_size, self.capacity, self.item_layout.align())
                    .expect("BlobVec::shrink_to_fit: Layout overflow");
            // SAFETY: `len == 0` and the ZST case returned above, so this is the live
            // allocation this vec made with `old_layout` and there is no element left to drop
            // (`clear`/`swap_remove` handled ownership). The pointer is not used again before
            // being replaced with a dangling one on the next line.
            unsafe { alloc::dealloc(self.data.as_ptr(), old_layout) };
            self.data = NonNull::dangling();
            self.capacity = 0;
            return;
        }

        let new_layout = checked_array_layout(item_size, self.len, self.item_layout.align())
            .expect("BlobVec::shrink_to_fit: Layout overflow");
        let old_layout = checked_array_layout(item_size, self.capacity, self.item_layout.align())
            .expect("BlobVec::shrink_to_fit: Layout overflow");

        // SAFETY: same as `grow`'s realloc — a live allocation of `old_layout` from this
        // allocator, and `new_layout.size()` is non-zero because `len > 0` and the element is not
        // a ZST (both returned earlier). Shrinking keeps the first `len` elements in place.
        let new_data = unsafe { alloc::realloc(self.data.as_ptr(), old_layout, new_layout.size()) };
        self.data =
            NonNull::new(new_data).expect("BlobVec::shrink_to_fit: Allocation failed (OOM)");
        self.capacity = self.len;
    }

    /// Abandons every element at or above `len` **without running their drop glue**, and
    /// without touching the allocation.
    ///
    /// This LEAKS whatever those elements owned, deliberately. It exists for one caller:
    /// `Archetype::forget_rows_above`, which runs when a migration has been abandoned part-way
    /// and some of the rows above `len` are initialised while others are not — and nothing can
    /// say which. Dropping them would be drop glue over uninitialised bytes; leaving them
    /// counted would be the same thing later, at teardown. Forgetting them is the only sound
    /// answer, and a leak on a panic path is an acceptable one.
    ///
    /// It cannot fail: one integer store, no allocation, no user code. That is the whole reason
    /// the abandon protocol is built on it.
    ///
    /// # Safety
    /// `len <= self.len`. Growing the length this way would count uninitialised memory as live.
    #[inline]
    pub(crate) unsafe fn forget_above(&mut self, len: usize) {
        debug_assert!(len <= self.len);
        self.len = len;
    }

    /// Drops all elements (without releasing the memory).
    ///
    /// **The length is zeroed BEFORE the drops run**, which is what makes this safe when one of
    /// them panics. It used to be set afterwards, and the SAFETY comment below used to say — of
    /// that arrangement — "`len` is set to 0 right after the loop, so nothing is dropped twice".
    /// True on the normal path and false on the unwind path: a panic at element `i` left `len`
    /// untouched, so `Drop for BlobVec` called `clear` again and re-dropped `0..i`. Dropping an
    /// already-dropped value is undefined behaviour, and this is the primitive
    /// `Column::clear`, `Archetype::clear` and `ComponentSparseSet::clear` are all built on.
    ///
    /// Zeroing first turns that into a LEAK of the elements after the panicking one, which is
    /// safe. It is what `Vec::clear` does for the same reason. Fixed 2026-08-31.
    #[inline]
    pub fn clear(&mut self) {
        // Take the length first: from here on this vec owns nothing, whatever happens below.
        let len = std::mem::replace(&mut self.len, 0);
        if let Some(drop_fn) = self.drop_fn {
            let item_size = self.item_layout.size();
            for i in 0..len {
                // SAFETY: `i < len`, the length this vec had on entry, so `i * item_size` stays
                // inside the allocation and the element there was live and owned by this vec —
                // `drop_fn` is the type's own dropper, recorded at construction for exactly this
                // layout. `self.len` is already 0, so no path can reach any of these again.
                unsafe {
                    let ptr = self.data.as_ptr().add(i * item_size);
                    drop_fn(ptr);
                }
            }
        }
    }
}

impl Drop for BlobVec {
    fn drop(&mut self) {
        self.clear();
        let item_size = self.item_layout.size();
        if item_size > 0 && self.capacity > 0 {
            let layout =
                checked_array_layout(item_size, self.capacity, self.item_layout.align())
                    .expect("BlobVec::drop: Layout error");
            // SAFETY: `clear()` above dropped every live element, and the guard on this branch
            // says the allocation exists (non-ZST, capacity > 0). `layout` is recomputed from the
            // same fields that allocated it, and the vec is being dropped, so the pointer is
            // never read again.
            unsafe {
                alloc::dealloc(self.data.as_ptr(), layout);
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// COLUMN — Tip-silinmiş sütun

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A value whose `Drop` panics on demand and counts every drop it is asked for.
    ///
    /// The counter is what the two tests below actually assert on: a value dropped twice is
    /// undefined behaviour, and the observable shadow of it is a count that is one too high.
    struct Bomb {
        armed: bool,
        counter: &'static AtomicU32,
    }
    impl Drop for Bomb {
        fn drop(&mut self) {
            self.counter.fetch_add(1, Ordering::SeqCst);
            if self.armed {
                self.armed = false;
                panic!("Bomb");
            }
        }
    }

    /// `clear` must not re-drop what it already dropped when one of the drops panics.
    ///
    /// It used to set `len = 0` AFTER the loop, so an unwind left the length untouched and
    /// `Drop for BlobVec` — which calls `clear` — ran the whole prefix again. Three values, the
    /// middle one armed: the correct behaviour is four drop calls in total (three from the
    /// clear, the last of which panics, then nothing at teardown), and the old ordering gave
    /// six, re-dropping the first two.
    #[test]
    fn a_panicking_drop_during_clear_does_not_drop_anything_twice() {
        static DROPS: AtomicU32 = AtomicU32::new(0);
        DROPS.store(0, Ordering::SeqCst);

        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut v = BlobVec::new(
                Layout::new::<Bomb>(),
                Some(|ptr: *mut u8| {
                    // SAFETY: this thunk is only ever called by `BlobVec` on a slot of the
                    // layout it was constructed with, which is `Bomb`'s, holding a live value.
                    unsafe { std::ptr::drop_in_place(ptr as *mut Bomb) }
                }),
            );
            for armed in [false, true, false] {
                let mut b = Bomb { armed, counter: &DROPS };
                // SAFETY: `b` is a live `Bomb` of this vec's layout; `push` takes ownership by
                // memcpy, which is why it is forgotten rather than dropped.
                unsafe { v.push(&mut b as *mut Bomb as *const u8) };
                std::mem::forget(b);
            }
            v.clear();
            // Unreachable: the middle drop panics.
            drop(v);
        }));
        assert!(unwound.is_err(), "the armed drop was supposed to panic");

        assert_eq!(
            DROPS.load(Ordering::SeqCst),
            2,
            "the first value and the armed one were dropped once each; the third is leaked by \
             the unwind, which is safe — anything above 2 means something was dropped twice"
        );
    }

    /// `swap_remove_and_drop` must not leave a destroyed value inside `len`.
    ///
    /// It used to drop first and decrement afterwards, so a panicking drop left the victim
    /// counted and `Drop for BlobVec` dropped it again.
    #[test]
    fn a_panicking_drop_during_swap_remove_does_not_drop_the_victim_twice() {
        static DROPS: AtomicU32 = AtomicU32::new(0);
        DROPS.store(0, Ordering::SeqCst);

        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut v = BlobVec::new(
                Layout::new::<Bomb>(),
                Some(|ptr: *mut u8| {
                    // SAFETY: this thunk is only ever called by `BlobVec` on a slot of the
                    // layout it was constructed with, which is `Bomb`'s, holding a live value.
                    unsafe { std::ptr::drop_in_place(ptr as *mut Bomb) }
                }),
            );
            // The armed value is at index 0, so it is NOT the last row — the branch that swaps,
            // which is the one whose ordering was wrong.
            for armed in [true, false, false] {
                let mut b = Bomb { armed, counter: &DROPS };
                // SAFETY: as above.
                unsafe { v.push(&mut b as *mut Bomb as *const u8) };
                std::mem::forget(b);
            }
            // SAFETY: index 0 is live and the vec holds three rows.
            unsafe { v.swap_remove_and_drop(0) };
            drop(v);
        }));
        assert!(unwound.is_err(), "the armed drop was supposed to panic");

        // Three: the victim once, and the two survivors once each when `v` is dropped during
        // the unwind — `clear` still owns them and is right to. FOUR is the old behaviour and
        // the whole point: the victim stayed inside `len`, so teardown dropped it a second time.
        assert_eq!(
            DROPS.load(Ordering::SeqCst),
            3,
            "victim + two survivors, once each; 4 means the destroyed victim was still counted"
        );
    }

    /// Shrinking a ZST vec must leave its capacity sentinel alone.
    ///
    /// `new` sets a zero-sized element type's capacity to `usize::MAX` to mean "never needs to
    /// grow", and `grow` relies on that by returning early for ZSTs without updating it.
    /// `shrink_to_fit` used to write `capacity = len` instead, contradicting the sentinel — and
    /// `World::compact` runs it over every ZST marker column on every GC tick, so the invariant
    /// was false almost everywhere. Nothing broke, because `push` short-circuits on ZSTs before
    /// looking at capacity; a type whose own documented invariant is routinely false is still
    /// how the next reader gets misled.
    #[test]
    fn shrinking_a_zst_column_keeps_its_capacity_sentinel() {
        struct Marker;
        let mut v = BlobVec::new(Layout::new::<Marker>(), None);
        assert_eq!(v.capacity, usize::MAX, "a ZST vec starts at the sentinel");

        for _ in 0..3 {
            let mut m = Marker;
            // SAFETY: `m` is a live `Marker` of this vec's layout; `push` takes ownership by
            // memcpy, which for a ZST copies nothing. No `mem::forget` after it, unlike the
            // `Bomb` pushes elsewhere in this module: `Marker` has no `Drop`, so there is
            // nothing for the caller to hand over and nothing to leak.
            unsafe { v.push(&mut m as *mut Marker as *const u8) };
        }
        assert_eq!(v.len(), 3);

        v.shrink_to_fit();
        assert_eq!(
            v.capacity,
            usize::MAX,
            "there is no allocation to give back, and `grow` reads this to decide it never has \
             to run for a ZST"
        );
        // …and the vec still works afterwards, which is what the sentinel is protecting.
        let mut m = Marker;
        // SAFETY: as above.
        unsafe { v.push(&mut m as *mut Marker as *const u8) };
        assert_eq!(v.len(), 4);
    }

    #[test]
    fn blobvec_drop_called() {
        static DROP_COUNT: AtomicU32 = AtomicU32::new(0);
        
        struct Dropper;
        impl Drop for Dropper {
            fn drop(&mut self) { DROP_COUNT.fetch_add(1, Ordering::SeqCst); }
        }

        // SAFETY: test-local — the values were built here with the layout this storage was created
        // with, the rows used are the ones just pushed, and the test owns the storage outright.
        let mut vec = BlobVec::new(Layout::new::<Dropper>(), Some(|ptr| unsafe {
            std::ptr::drop_in_place(ptr as *mut Dropper)
        }));

        // SAFETY: test-local — the values were built here with the layout this storage was created
        // with, the rows used are the ones just pushed, and the test owns the storage outright.
        unsafe {
            let d = Dropper;
            vec.push(&d as *const Dropper as *const u8);
            std::mem::forget(d);
            let d2 = Dropper;
            vec.push(&d2 as *const Dropper as *const u8);
            std::mem::forget(d2);
        }

        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 0);
        drop(vec); // BlobVec drop edilince 2 kez çağrılmalı
        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn blobvec_swap_remove_drops_correctly() {
        static DROP_COUNT: AtomicU32 = AtomicU32::new(0);
        struct Dropper;
        impl Drop for Dropper {
            fn drop(&mut self) { DROP_COUNT.fetch_add(1, Ordering::SeqCst); }
        }

        // SAFETY: test-local — the values were built here with the layout this storage was created
        // with, the rows used are the ones just pushed, and the test owns the storage outright.
        let mut vec = BlobVec::new(Layout::new::<Dropper>(), Some(|ptr| unsafe {
            std::ptr::drop_in_place(ptr as *mut Dropper)
        }));

        // SAFETY: test-local — the values were built here with the layout this storage was created
        // with, the rows used are the ones just pushed, and the test owns the storage outright.
        unsafe {
            let d = Dropper;
            vec.push(&d as *const Dropper as *const u8);
            std::mem::forget(d);
            let d2 = Dropper;
            vec.push(&d2 as *const Dropper as *const u8);
            std::mem::forget(d2);
        }

        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 0);
        // SAFETY: test-local — the values were built here with the layout this storage was created
        // with, the rows used are the ones just pushed, and the test owns the storage outright.
        unsafe { vec.swap_remove_and_drop(0); }
        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 1);
        drop(vec);
        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn checked_array_layout_detects_overflow() {
        // item_size * count taşarsa None dönmeli (panik yerine).
        let item_size = 1000usize;
        let count = usize::MAX / 500 + 1; // 1000 * count sarar
        assert!(checked_array_layout(item_size, count, 8).is_none());

        // isize::MAX üstü de reddedilmeli (Layout::from_size_align kuralı).
        assert!(checked_array_layout(2, (isize::MAX as usize / 2) + 1, 1).is_none());

        // Makul boyutlar geçerli Layout üretmeli.
        assert!(checked_array_layout(16, 8, 8).is_some());
    }

    #[test]
    fn blobvec_no_drop_for_copy_types() {
        let vec = BlobVec::new(Layout::new::<u32>(), None);
        // Bu testin amacı sadece Copy tipler için drop fn'in None olduğunu
        // derleme zamanında doğrulamak; gerçek davranış Miri ile çalıştırılmalı.
        let _ = &vec;
    }
}
