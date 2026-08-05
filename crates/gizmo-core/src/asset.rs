//! Handle-based asset storage: `Assets<T>` plus the typed [`Handle`] that indexes it.
//!
//! Handles are cheap `Copy` indices, not smart pointers — nothing is reference-counted and
//! nothing is freed when the last handle goes away. A handle outliving its asset dangles in
//! the sense that lookups return `None`, never in the sense of unsafety.
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use crossbeam_queue::SegQueue;

static NEXT_HANDLE_ID: AtomicUsize = AtomicUsize::new(1);

/// Process-global identity of one asset stored in an [`Assets`] map.
///
/// The wrapped `usize` comes from a single process-wide counter that starts at `1` and
/// only ever increases, so [`HandleId::new`] never yields `HandleId(0)` and never reuses
/// a value within a run — not even after the asset it named has been removed. The counter
/// is shared by *every* asset type, so ids minted for an `Assets<Mesh>` can never collide
/// with those of an `Assets<Texture>`; the flip side is that the id itself carries no type
/// information (only [`Handle`] does), so looking a mesh id up in a texture map is not a
/// type error — it simply misses.
///
/// The concrete value depends on how many handles happened to be created before it, on any
/// thread. Do not persist it in a save file and do not feed it into a determinism hash: two
/// runs that load the same content in a different order will produce different ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HandleId(pub usize);

impl Default for HandleId {
    fn default() -> Self {
        Self::new()
    }
}

impl HandleId {
    /// Takes the next unused value from the process-global counter.
    ///
    /// Thread-safe and collision-free: two concurrent calls always get different ids. The
    /// bump is `Relaxed`, so the id orders nothing but itself — it is an identity, not a
    /// synchronisation point.
    ///
    /// This only mints a name. No entry appears in any [`Assets`] map, so a handle built
    /// from a fresh id resolves to `None` until something inserts under it.
    pub fn new() -> Self {
        HandleId(NEXT_HANDLE_ID.fetch_add(1, Ordering::Relaxed))
    }
}

/// Sentinel that reports one asset id as unreachable once the last strong [`Handle`]
/// sharing it goes away.
///
/// It lives behind an `Arc` inside [`Handle::tracker`]; cloning a handle clones that `Arc`,
/// so `drop` runs exactly once — when the final clone of that group dies. Dropping it only
/// pushes the id onto a queue; it does not touch the asset map, so nothing is actually
/// freed until someone calls [`Assets::process_drops`].
pub struct HandleIdTracker {
    /// The [`HandleId`] payload to report. Must equal `Handle::id.0` of every handle
    /// holding this tracker — a mismatch collects a different, innocent asset.
    pub id: usize,
    /// Queue owned by the [`Assets`] map that stores `id`. Pushing into a queue belonging
    /// to some other map, or into one nobody drains, leaks the asset instead of collecting
    /// it: the notice is delivered to a collector that has nothing to remove.
    pub drop_queue: Arc<SegQueue<usize>>,
}

impl Drop for HandleIdTracker {
    fn drop(&mut self) {
        self.drop_queue.push(self.id);
    }
}

/// Typed name for one entry of an [`Assets`] map, optionally reference-counted.
///
/// A handle never owns or borrows the asset — it is an id plus, for strong handles, a shared
/// liveness sentinel. Cloning is therefore cheap, and a handle stays *usable* after the entry
/// it names is gone; it simply starts resolving to `None`. Comparison and hashing look at
/// [`Handle::id`] alone, so a weak and a strong handle to the same id are equal and hash
/// alike. `T` is a compile-time tag only ([`PhantomData`]) — it stops a `Handle<Mesh>` from
/// being passed to an `Assets<Texture>` at the type level, and costs nothing at runtime.
///
/// Two flavours:
///
/// * **strong** — [`Handle::new`], and what [`Assets::add`] hands back. When the last clone
///   of that handle dies, its id is queued for removal from the map it was minted against.
/// * **weak** — [`Handle::weak`]. A pure name: it can resolve an entry but never keeps one
///   alive and never causes one to be dropped.
///
/// `Handle::default()` is *not* a null handle: it mints a brand-new [`HandleId`] and wraps it
/// weakly, so a defaulted handle names an entry that does not exist and resolves to `None`
/// until something inserts under exactly that id.
///
/// `Handle<T>` implements `Component` for every `T: 'static + Send + Sync`, so a handle can be
/// attached to an entity directly rather than being wrapped in a component of your own.
pub struct Handle<T> {
    /// Which entry of the map this handle names.
    ///
    /// Public and `Copy`, but treat it as read-only once a tracker exists: on a strong handle
    /// it must stay equal to `tracker.id`. Overwriting it desyncs the two, and the tracker
    /// will collect the *old* id when it drops while the new one is never collected.
    pub id: HandleId,
    /// The shared liveness sentinel — `None` on a weak handle, `Some` on a strong one.
    ///
    /// Clones of one handle share this `Arc`, so the drop notice fires once per clone group,
    /// not once per handle. Groups are per-construction, not per-id: building a second strong
    /// handle for the same id ([`Handle::new`] again, or [`Handle::make_strong`]) creates an
    /// *independent* group, and whichever group empties first queues the id — collecting the
    /// asset out from under the handles that are still alive in the other group.
    pub tracker: Option<Arc<HandleIdTracker>>,
    _marker: PhantomData<T>,
}

impl<T> Default for Handle<T> {
    fn default() -> Self {
        Self::weak(HandleId::new())
    }
}

impl<T> Handle<T> {
    /// Builds a strong handle for an id that already exists, bound to `drop_queue`.
    ///
    /// This only creates a name: nothing is inserted anywhere, so the handle resolves to
    /// `None` unless `id` is (or becomes) a key of the map that owns `drop_queue`. Passing a
    /// queue belonging to some other map means the drop notice is delivered to a collector
    /// that has nothing to remove, and the asset leaks.
    ///
    /// Prefer [`Assets::add`], which mints the id, stores the asset and returns the one
    /// authoritative strong handle. Calling this a second time for an id that already has a
    /// strong handle starts a second, independent refcount group — see [`Handle::tracker`]
    /// for why that collects early.
    pub fn new(id: HandleId, drop_queue: Arc<SegQueue<usize>>) -> Self {
        Self {
            id,
            tracker: Some(Arc::new(HandleIdTracker {
                id: id.0,
                drop_queue,
            })),
            _marker: PhantomData,
        }
    }

    /// Builds a handle that names `id` without taking part in its lifetime.
    ///
    /// Dropping it — or all of its clones — queues nothing, so a weak handle can never cause
    /// an asset to be removed, and it cannot keep one alive either. This is the right shape
    /// for an id whose ownership lives elsewhere: an id promised by a loader before the asset
    /// exists, or an id read back from a scene file.
    ///
    /// An entry that only ever had weak handles is never garbage-collected: nothing will push
    /// it onto the queue, so it stays in the map until [`Assets::remove`] takes it out.
    pub fn weak(id: HandleId) -> Self {
        Self {
            id,
            tracker: None,
            _marker: PhantomData,
        }
    }
    
    /// Whether this handle carries no tracker, i.e. it does not contribute to the refcount
    /// that decides when its asset is collected.
    ///
    /// This is per-handle, not per-id: a weak handle says nothing about whether some *other*
    /// handle is keeping the same id alive. It is also what `Debug` reports, printing
    /// `WeakHandle(..)` or `StrongHandle(..)`.
    pub fn is_weak(&self) -> bool {
        self.tracker.is_none()
    }

    /// Upgrades a weak handle in place by attaching a tracker bound to `drop_queue`.
    ///
    /// **No-op when the handle is already strong** — an existing tracker is never re-bound, so
    /// this cannot be used to move a handle from one map's drop queue to another's; the only
    /// way to do that is to rebuild the handle.
    ///
    /// The upgrade is not shared with clones that were taken while the handle was weak: they
    /// keep `tracker: None`, and the group created here empties as soon as this handle and any
    /// clone made *after* this call are gone — even if those older weak clones are still in
    /// use. Upgrading an id that another strong group already owns has the early-collection
    /// hazard described on [`Handle::tracker`].
    pub fn make_strong(&mut self, drop_queue: Arc<SegQueue<usize>>) {
        if self.tracker.is_none() {
            self.tracker = Some(Arc::new(HandleIdTracker {
                id: self.id.0,
                drop_queue,
            }));
        }
    }
}

impl<T> Clone for Handle<T> {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            tracker: self.tracker.clone(),
            _marker: PhantomData,
        }
    }
}

impl<T> PartialEq for Handle<T> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl<T> Eq for Handle<T> {}

impl<T> Hash for Handle<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

impl<T: 'static + Send + Sync> crate::component::Component for Handle<T> {}

impl<T> std::fmt::Debug for Handle<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_weak() {
            write!(f, "WeakHandle({:?})", self.id)
        } else {
            write!(f, "StrongHandle({:?})", self.id)
        }
    }
}

/// Generic resource to store assets by their handle ID.
///
/// A plain `HandleId → T` map plus a concurrent queue of ids whose strong handles have all
/// died. Collection is deliberately two-step and never automatic: dropping a handle only
/// *reports* the id, and the entry survives until someone calls [`Assets::process_drops`].
/// Nothing inside this type schedules that call, so a map that is never polled never frees
/// anything.
///
/// Each `Assets` instance owns its own queue, so a strong handle's collection binding is
/// fixed when it is minted: a handle minted by map A reports its death to A's queue, whichever
/// map actually holds the entry. Resolution is a different matter — [`Assets::get`] and
/// friends key on the id alone, so any map holding that id will answer.
///
/// Every mutation goes through `&mut self`; only the drop *reporting* is concurrent, so
/// handles may be dropped on any thread while the map itself is untouched.
pub struct Assets<T> {
    /// The live entries, keyed by the id of the handle that names them.
    ///
    /// Public so callers can iterate in bulk or insert under an id minted elsewhere; both are
    /// supported, with two caveats. Entries put here by hand are collected only if a strong
    /// handle for that id exists somewhere — otherwise they are permanent. And this is a `std`
    /// `HashMap` with the default randomly seeded hasher, so iteration order is arbitrary and
    /// not reproducible from run to run: never let an iteration over `data` decide the order
    /// of anything the simulation hashes or replays.
    pub data: HashMap<HandleId, T>,
    drop_queue: Arc<SegQueue<usize>>,
}

impl<T> Default for Assets<T> {
    fn default() -> Self {
        Self {
            data: HashMap::new(),
            drop_queue: Arc::new(SegQueue::new()),
        }
    }
}

impl<T> Assets<T> {
    /// An empty map and — the part that matters — a brand-new drop queue. Equivalent to
    /// `Assets::default()`.
    ///
    /// That queue is this map's identity for collection purposes. [`Assets::add`] binds every
    /// strong handle it mints to *this* queue, so those handles report their death here and
    /// nowhere else; a strong handle minted against a different `Assets<T>` keeps reporting to
    /// that one, so this map will never collect what it names — even though the id still
    /// resolves here if something inserted under it.
    pub fn new() -> Self {
        Self::default()
    }

    /// Stores `asset` under a freshly minted id and returns the sole strong handle to it.
    ///
    /// The id comes from the process-global counter, so this can never overwrite an existing
    /// entry, not even one added by a different `Assets` map. Keep the returned handle (or a
    /// clone of it) alive for as long as the asset is needed: when the last clone drops, the
    /// id is queued and the entry is erased by the next [`Assets::process_drops`].
    pub fn add(&mut self, asset: T) -> Handle<T> {
        let id = HandleId::new();
        self.data.insert(id, asset);
        Handle::new(id, self.drop_queue.clone())
    }

    /// Resolves `handle` against this map, by id only.
    ///
    /// `None` means "no entry under that id here" and does not distinguish the cases: never
    /// inserted, already removed, or minted against a different `Assets` map. Handle strength
    /// is irrelevant — holding a strong handle is not a promise of `Some`, since
    /// [`Assets::remove`] and an early collection (see [`Handle::tracker`]) both erase the
    /// entry regardless.
    pub fn get(&self, handle: &Handle<T>) -> Option<&T> {
        self.data.get(&handle.id)
    }

    /// Same lookup as [`Assets::get`], borrowing the asset exclusively.
    ///
    /// There is no change detection or version counter on this map, so an in-place edit is
    /// invisible to anything caching data derived from the asset — such a consumer has to be
    /// told about the edit some other way.
    pub fn get_mut(&mut self, handle: &Handle<T>) -> Option<&mut T> {
        self.data.get_mut(&handle.id)
    }

    /// Stores `asset` under `handle.id`, dropping whatever was there before.
    ///
    /// The usual way to fulfil an id that was handed out ahead of the data, e.g. by an async
    /// loader. `handle` is read for its id and nothing else: passing a weak handle is allowed
    /// and is the common case, but then the entry has no owner and nothing will ever queue it
    /// for collection — it lives until [`Assets::remove`].
    pub fn insert(&mut self, handle: &Handle<T>, asset: T) {
        self.data.insert(handle.id, asset);
    }

    /// Takes the entry out of the map and returns it, or `None` if there was none.
    ///
    /// Independent of handle strength: removing while strong handles are still alive is
    /// allowed, and leaves them resolving to `None`. When those handles later die the id is
    /// still queued, and [`Assets::process_drops`] then finds nothing to erase — harmless,
    /// because ids are never reused, so the stale notice cannot hit a newer asset.
    pub fn remove(&mut self, handle: &Handle<T>) -> Option<T> {
        self.data.remove(&handle.id)
    }

    /// Collects dropped handles and removes their corresponding assets.
    ///
    /// Drains the whole queue and erases each reported id, so cost is proportional to the
    /// number of handles that died since the last call, not to the size of the map. Ids with
    /// no entry are skipped silently.
    ///
    /// This is the only path that frees an asset on its own — [`Assets::remove`] and an
    /// overwriting [`Assets::insert`] do so on the caller's say-so — and nothing in this type
    /// schedules it: whoever owns the map has to call it, typically once per frame. Removal is
    /// unconditional — it does not check whether some other strong handle to the same id is
    /// still alive,
    /// which is why creating two independent strong groups for one id (see
    /// [`Handle::tracker`]) can pull an asset out from under a live handle.
    pub fn process_drops(&mut self) {
        while let Some(dropped_id) = self.drop_queue.pop() {
            self.data.remove(&HandleId(dropped_id));
        }
    }
}
