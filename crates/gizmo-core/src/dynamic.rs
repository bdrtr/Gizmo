//! Components defined while the program runs.
//!
//! A script, a mod or an editor names a component and gives its size and alignment, and from then on
//! the world treats it as it treats a Rust type: it gets archetype columns (or a sparse set), takes part
//! in `Changed`/`Added`, conflicts in the scheduler with any system that writes it, is dropped with its
//! entity and cloned with it, and fires the same lifecycle hooks.
//!
//! - [`ComponentDescriptor`] says what the component is; [`World::register_component`] defines it and
//!   returns its [`ComponentKey`], which [`World::component_key_by_name`] finds again.
//! - [`World::insert_by_key`] and [`World::remove_by_key`] put a value on an entity and take it off;
//!   [`World::get_component_ptr`] and [`World::get_component_mut_ptr`] read and write one.
//! - [`DynamicQuery`] iterates every entity with a set of components, typed or not, and
//!   [`DynamicSystem`] runs one in a [`Schedule`](crate::system::Schedule) with its access declared.
//!
//! A component with no type is bytes to Rust, so reading and writing one goes through raw pointers and
//! `unsafe`: what the bytes mean is the caller's to know.
//!
//! ```
//! use std::alloc::Layout;
//! use gizmo_core::prelude::*;
//! use gizmo_core::dynamic::{ComponentDescriptor, DynamicQuery};
//!
//! let mut world = World::new();
//! let health = world.register_component(ComponentDescriptor::new("Health", Layout::new::<f32>())).unwrap();
//! let hero = world.spawn();
//! let mut value = 80.0f32;
//! // SAFETY: `value` is a live, aligned `f32` — `Health`'s layout — and is not used again.
//! unsafe { world.insert_by_key(hero, health, (&raw mut value).cast()) }.unwrap();
//!
//! let heal = DynamicQuery::builder().write(health).build(&world).unwrap();
//! let mut rows = heal.iter_mut(&mut world);
//! while let Some(mut row) = rows.fetch_next() {
//!     // SAFETY: term 0 is `Health`, an `f32`.
//!     unsafe { *row.ptr_mut(0).cast::<f32>() += 20.0 };
//! }
//! let ptr = world.get_component_ptr(hero, health).unwrap();
//! // SAFETY: `Health` is an `f32`, read before anything changes the world.
//! assert_eq!(unsafe { *ptr.cast::<f32>() }, 100.0);
//! ```

use std::alloc::Layout;
use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

use crate::archetype::sparse_set::ComponentSparseSet;
use crate::archetype::{Archetype, ComponentInfo, ComponentTicks};
use crate::component::StorageType;
use crate::component_key::ComponentKey;
use crate::entity::Entity;
use crate::system::{AccessInfo, IntoSystemConfig, System, SystemConfig};
use crate::world::World;

// =========================================================================
// DEFINITION
// =========================================================================

/// What a component with no Rust type is: a name, a size and alignment, and what to run when a value
/// is dropped or cloned. [`World::register_component`] turns it into a component.
///
/// [`new`](Self::new) describes plain bytes — nothing to drop, cloned by copying — which is safe to
/// define, since nothing about bytes can be wrong. A component that owns something (a handle, a heap
/// allocation of a scripting runtime) gives its drop glue with [`with_drop`](Self::with_drop) and its
/// clone glue with [`with_clone`](Self::with_clone); those are `unsafe`, because the world will run
/// them on every value it holds.
#[derive(Clone, Debug)]
pub struct ComponentDescriptor {
    name: String,
    layout: Layout,
    drop_fn: Option<unsafe fn(*mut u8)>,
    clone_fn: Option<unsafe fn(*const u8, *mut u8, usize)>,
    storage: StorageType,
    mutable: bool,
}

impl ComponentDescriptor {
    /// A component of `layout`'s size and alignment that is plain bytes: nothing runs when a value is
    /// dropped, and a clone is a copy of the bytes. Stored in archetype tables, and writable.
    ///
    /// The size is rounded up to a multiple of the alignment, as a Rust type's always is, so that every
    /// value in a column is aligned.
    #[must_use]
    pub fn new(name: impl Into<String>, layout: Layout) -> Self {
        Self {
            name: name.into(),
            layout: layout.pad_to_align(),
            drop_fn: None,
            clone_fn: None,
            storage: StorageType::Table,
            mutable: true,
        }
    }

    /// Gives the component drop glue, which the world runs once on each value it lets go of — removed,
    /// replaced, or despawned with its entity.
    ///
    /// A component with drop glue cannot be cloned by copying its bytes, so unless it also has
    /// [`with_clone`](Self::with_clone), [`World::clone_entity`] refuses an entity that carries it.
    ///
    /// # Safety
    /// `drop` must be sound to call once with a pointer, aligned to the layout, to any value inserted
    /// under this component. The world never reads those bytes again afterwards.
    #[must_use]
    pub unsafe fn with_drop(mut self, drop: unsafe fn(*mut u8)) -> Self {
        self.drop_fn = Some(drop);
        self
    }

    /// Gives the component clone glue, which [`World::clone_entity`] runs to copy a value.
    ///
    /// # Safety
    /// `clone(src, dst, count)` must write `count` values one size apart starting at `dst`, each a
    /// value of this component that the world may later drop on its own — independent of the one at
    /// `src`, which it must leave as it was. `src` points to a value of this component; `dst` is
    /// aligned, uninitialised, does not overlap `src`, and has room for `count` values.
    #[must_use]
    pub unsafe fn with_clone(mut self, clone: unsafe fn(*const u8, *mut u8, usize)) -> Self {
        self.clone_fn = Some(clone);
        self
    }

    /// Stores the component in a sparse set instead of archetype tables: cheaper to add and remove
    /// often, slower to iterate. The same choice a type makes with `impl_component!(T; sparse)`.
    #[must_use]
    pub fn with_storage(mut self, storage: StorageType) -> Self {
        self.storage = storage;
        self
    }

    /// Makes the component immutable: a value changes only by inserting another, which hooks hear, and
    /// a [`DynamicQuery`] that asks to write it is refused. The same as `impl_component!(T; immutable)`.
    #[must_use]
    pub fn immutable(mut self) -> Self {
        self.mutable = false;
        self
    }

    /// The name it will be found by.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Its size and alignment, the size already a multiple of the alignment.
    #[must_use]
    pub fn layout(&self) -> Layout {
        self.layout
    }

    pub(crate) fn into_info(self, key: ComponentKey) -> ComponentInfo {
        ComponentInfo {
            key,
            type_name: intern(&self.name),
            layout: self.layout,
            drop_fn: self.drop_fn,
            clone_fn: self.clone_fn,
            storage_type: self.storage,
            mutable: self.mutable,
        }
    }
}

/// The process-wide copy of a component name. `ComponentInfo` is `Copy` and carries its name as a
/// `&'static str`, which a type gets from `type_name`; a component defined at runtime gets it here,
/// one allocation per distinct name however many worlds register it.
fn intern(name: &str) -> &'static str {
    static NAMES: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    let mut names = NAMES.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    if let Some(&interned) = names.get(name) {
        return interned;
    }
    let interned: &'static str = Box::leak(name.into());
    names.insert(interned);
    interned
}

/// Why a runtime component operation was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DynamicComponentError {
    /// [`World::register_component`] was given a name a component defined at runtime already has —
    /// the one this key names.
    NameTaken(ComponentKey),
    /// The key is not registered in this world. A runtime key belongs to the world that defined it; a
    /// type's key is registered by the first insert of the type or by `register_component_type`.
    NotRegistered(ComponentKey),
    /// The entity is not alive.
    NoSuchEntity(Entity),
    /// A query names this component twice and writes it at least once, or writes it and filters on
    /// its change ticks — two views of the same row, one of them mutable.
    Conflict(ComponentKey),
    /// A query asks to write a component declared immutable.
    Immutable(ComponentKey),
}

impl std::fmt::Display for DynamicComponentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NameTaken(key) => write!(f, "a runtime component of that name is already registered ({key:?})"),
            Self::NotRegistered(key) => write!(f, "{key:?} is not registered in this world"),
            Self::NoSuchEntity(entity) => write!(f, "{entity:?} is not alive"),
            Self::Conflict(key) => write!(f, "the query reaches {key:?} twice, once to write it"),
            Self::Immutable(key) => write!(f, "{key:?} is immutable and cannot be written through a query"),
        }
    }
}

impl std::error::Error for DynamicComponentError {}

// =========================================================================
// QUERY
// =========================================================================

/// One component a query yields: index `i` of a row is the `i`-th `read`/`write` of the builder.
#[derive(Clone, Copy, Debug)]
struct Term {
    key: ComponentKey,
    write: bool,
    sparse: bool,
    size: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FilterKind {
    With,
    Without,
    Changed,
    Added,
}

#[derive(Clone, Copy, Debug)]
struct Filter {
    key: ComponentKey,
    kind: FilterKind,
    sparse: bool,
}

/// Builds a [`DynamicQuery`]. Each `read` and `write` adds a term rows yield, in order; `with`,
/// `without`, `changed` and `added` narrow which entities match, as the typed filters do.
#[derive(Clone, Debug, Default)]
#[must_use]
pub struct DynamicQueryBuilder {
    data: Vec<(ComponentKey, bool)>,
    filters: Vec<(ComponentKey, FilterKind)>,
}

impl DynamicQueryBuilder {
    /// Yields `key`'s value to read — `&T` in a typed query.
    pub fn read(mut self, key: impl Into<ComponentKey>) -> Self {
        self.data.push((key.into(), false));
        self
    }

    /// Yields `key`'s value to write — `&mut T` in a typed query.
    pub fn write(mut self, key: impl Into<ComponentKey>) -> Self {
        self.data.push((key.into(), true));
        self
    }

    /// Only entities that have `key` — `With<T>`.
    pub fn with(mut self, key: impl Into<ComponentKey>) -> Self {
        self.filters.push((key.into(), FilterKind::With));
        self
    }

    /// Only entities that do not have `key` — `Without<T>`.
    pub fn without(mut self, key: impl Into<ComponentKey>) -> Self {
        self.filters.push((key.into(), FilterKind::Without));
        self
    }

    /// Only entities whose `key` was written since the reference tick — `Changed<T>`.
    pub fn changed(mut self, key: impl Into<ComponentKey>) -> Self {
        self.filters.push((key.into(), FilterKind::Changed));
        self
    }

    /// Only entities whose `key` was inserted since the reference tick — `Added<T>`.
    pub fn added(mut self, key: impl Into<ComponentKey>) -> Self {
        self.filters.push((key.into(), FilterKind::Added));
        self
    }

    /// Checks the terms against `world` and builds the query.
    ///
    /// # Errors
    /// [`NotRegistered`](DynamicComponentError::NotRegistered) for a key `world` does not know — every
    /// key must be, a type's included, because how it is stored decides how it is read;
    /// [`Conflict`](DynamicComponentError::Conflict) for a component reached twice with one of the two
    /// a write; [`Immutable`](DynamicComponentError::Immutable) for a write of an immutable component.
    pub fn build(self, world: &World) -> Result<DynamicQuery, DynamicComponentError> {
        let info = |key| world.component_info(key).ok_or(DynamicComponentError::NotRegistered(key));
        let mut data = Vec::with_capacity(self.data.len());
        for (i, &(key, write)) in self.data.iter().enumerate() {
            let info = info(key)?;
            if write && !info.mutable {
                return Err(DynamicComponentError::Immutable(key));
            }
            let twice = self.data.iter().enumerate().any(|(j, &(other, other_write))| {
                j != i && other == key && (write || other_write)
            });
            let ticks_read = write
                && self
                    .filters
                    .iter()
                    .any(|&(other, kind)| other == key && matches!(kind, FilterKind::Changed | FilterKind::Added));
            if twice || ticks_read {
                return Err(DynamicComponentError::Conflict(key));
            }
            data.push(Term {
                key,
                write,
                sparse: info.storage_type == StorageType::SparseSet,
                size: info.layout.size(),
            });
        }
        let mut filters = Vec::with_capacity(self.filters.len());
        for &(key, kind) in &self.filters {
            let info = info(key)?;
            filters.push(Filter { key, kind, sparse: info.storage_type == StorageType::SparseSet });
        }
        Ok(DynamicQuery { data, filters })
    }
}

/// A query whose terms are chosen while the program runs, by [`ComponentKey`]: over components defined
/// at runtime, over types, or both.
///
/// Rows come one at a time from [`DynamicRows::fetch_next`], each borrowed until the next, and yield
/// raw pointers to their terms. [`iter`](Self::iter) reads; [`iter_mut`](Self::iter_mut) can also write,
/// which takes the world exclusively. Inside a schedule the query belongs to a [`DynamicSystem`], which
/// declares its access so the scheduler keeps writers of the same components out of its batch.
#[derive(Clone, Debug)]
pub struct DynamicQuery {
    data: Vec<Term>,
    filters: Vec<Filter>,
}

impl DynamicQuery {
    /// Starts a query with no terms.
    pub fn builder() -> DynamicQueryBuilder {
        DynamicQueryBuilder::default()
    }

    /// How many terms a row yields.
    #[must_use]
    pub fn term_count(&self) -> usize {
        self.data.len()
    }

    /// What the query touches, as the scheduler should see it: the terms it reads and the components
    /// its `changed`/`added` filters read the ticks of, as reads; the terms it writes, as writes.
    /// `with` and `without` touch no component memory and declare nothing — as with typed filters.
    #[must_use]
    pub fn access(&self) -> AccessInfo {
        let mut access = AccessInfo::new();
        for term in &self.data {
            if term.write {
                access.component_writes.push(term.key);
            } else {
                access.component_reads.push(term.key);
            }
        }
        for filter in &self.filters {
            if matches!(filter.kind, FilterKind::Changed | FilterKind::Added) {
                access.component_reads.push(filter.key);
            }
        }
        access
    }

    /// The rows, to read. `changed` and `added` compare against the world's
    /// [`change_ref_tick`](World::change_ref_tick), as [`World::query`] does; [`DynamicRow::ptr_mut`]
    /// panics.
    pub fn iter<'q, 'w>(&'q self, world: &'w World) -> DynamicRows<'q, 'w> {
        DynamicRows::new(self, world, world.change_ref_tick, false, false)
    }

    /// The rows, to read and to write the `write` terms. Takes the world exclusively, which is what
    /// makes the writes sound; inside a schedule use a [`DynamicSystem`].
    pub fn iter_mut<'q, 'w>(&'q self, world: &'w mut World) -> DynamicRows<'q, 'w> {
        let reference = world.change_ref_tick;
        DynamicRows::new(self, world, reference, true, false)
    }

    fn admits(&self, arch: &Archetype) -> bool {
        self.data.iter().all(|t| t.sparse || arch.has_component(t.key))
            && self.filters.iter().all(|f| {
                f.sparse
                    || match f.kind {
                        FilterKind::Without => !arch.has_component(f.key),
                        _ => arch.has_component(f.key),
                    }
            })
    }
}

/// Where one term's values are for the archetype being walked.
#[derive(Clone, Copy)]
enum Fetch {
    Table { data: *mut u8, ticks: *mut ComponentTicks },
    Sparse(*const ComponentSparseSet),
}

/// Where one filter's answer is for the archetype being walked.
#[derive(Clone, Copy)]
enum FilterFetch {
    /// Settled by [`DynamicQuery::admits`] for the whole archetype.
    Admitted,
    Ticks(*const ComponentTicks),
    /// A sparse component; `None` when no entity has ever had it.
    Sparse(Option<*const ComponentSparseSet>),
}

/// The rows of a [`DynamicQuery`], one at a time: [`fetch_next`](Self::fetch_next) lends a row until
/// it is called again, which is what lets a row hand out a writable pointer without two of them being
/// live for one entity.
pub struct DynamicRows<'q, 'w> {
    query: &'q DynamicQuery,
    world: &'w World,
    ref_tick: u32,
    writable: bool,
    default_filters: bool,
    next_archetype: usize,
    entities: &'w [Entity],
    row: usize,
    fetch: Vec<Fetch>,
    filter_fetch: Vec<FilterFetch>,
}

impl<'q, 'w> DynamicRows<'q, 'w> {
    fn new(query: &'q DynamicQuery, world: &'w World, ref_tick: u32, writable: bool, default_filters: bool) -> Self {
        Self {
            query,
            world,
            ref_tick,
            writable,
            default_filters,
            next_archetype: 0,
            entities: &[],
            row: 0,
            fetch: Vec::with_capacity(query.data.len()),
            filter_fetch: Vec::with_capacity(query.filters.len()),
        }
    }

    /// The next matching row, or `None` when there are no more.
    pub fn fetch_next(&mut self) -> Option<DynamicRow<'_>> {
        loop {
            while self.row >= self.entities.len() {
                if !self.enter_next_archetype() {
                    return None;
                }
            }
            let row = self.row;
            self.row += 1;
            let entity = self.entities[row];
            // SAFETY: `row` is below the archetype's length, which is the length of every column the
            // fetches point into, and the world is borrowed for `'w`, so none of them has moved.
            if unsafe { self.row_matches(row, entity.id()) } {
                return Some(DynamicRow {
                    entity,
                    row,
                    tick: self.world.tick,
                    writable: self.writable,
                    terms: &self.query.data,
                    fetch: &self.fetch,
                });
            }
        }
    }

    /// Runs `f` on every remaining row.
    pub fn for_each(mut self, mut f: impl FnMut(DynamicRow<'_>)) {
        while let Some(row) = self.fetch_next() {
            f(row);
        }
    }

    /// How many rows remain, counted by walking them.
    #[must_use]
    pub fn count(mut self) -> usize {
        let mut n = 0;
        while self.fetch_next().is_some() {
            n += 1;
        }
        n
    }

    fn enter_next_archetype(&mut self) -> bool {
        let archetypes = &self.world.archetype_index.archetypes;
        'archetypes: while self.next_archetype < archetypes.len() {
            let arch = &archetypes[self.next_archetype];
            self.next_archetype += 1;
            if arch.is_empty() || !self.query.admits(arch) {
                continue;
            }
            if self.default_filters && self.world.default_query_filters.excludes(arch) {
                continue;
            }
            self.fetch.clear();
            for term in &self.query.data {
                let fetch = if term.sparse {
                    match self.world.sparse_sets.get(&term.key) {
                        Some(set) => Fetch::Sparse(set),
                        // Nothing has ever had it, so nothing matches.
                        None => continue 'archetypes,
                    }
                } else if self.writable && term.write {
                    // SAFETY: the `&mut Column` lives for these two calls only. Exclusive access to
                    // the column for the iteration is what `iter_mut`'s `&mut World` or the
                    // scheduler's declared access gives, and the builder refused a second term on it.
                    let col = unsafe { arch.get_column_mut(term.key) }.expect("admitted, so present");
                    Fetch::Table { data: col.data_ptr_mut(), ticks: col.ticks_ptr_mut() }
                } else {
                    let col = arch.get_column(term.key).expect("admitted, so present");
                    // Cast to `*mut` only to share the enum: `ptr_mut` refuses a term not opened
                    // for writing, so nothing is ever written through these.
                    Fetch::Table { data: col.data_ptr().cast_mut(), ticks: col.ticks_ptr().cast_mut() }
                };
                self.fetch.push(fetch);
            }
            self.filter_fetch.clear();
            for filter in &self.query.filters {
                let fetch = if filter.sparse {
                    FilterFetch::Sparse(self.world.sparse_sets.get(&filter.key).map(|s| s as *const _))
                } else if matches!(filter.kind, FilterKind::Changed | FilterKind::Added) {
                    FilterFetch::Ticks(arch.get_column(filter.key).expect("admitted, so present").ticks_ptr())
                } else {
                    FilterFetch::Admitted
                };
                self.filter_fetch.push(fetch);
            }
            self.entities = arch.entities();
            self.row = 0;
            return true;
        }
        false
    }

    /// # Safety
    /// `row` is a row of the archetype the fetches were taken from.
    unsafe fn row_matches(&self, row: usize, id: u32) -> bool {
        let data = self.query.data.iter().zip(&self.fetch).all(|(_, fetch)| match *fetch {
            // SAFETY: the set is the world's, borrowed for `'w`.
            Fetch::Sparse(set) => unsafe { (*set).contains(id) },
            Fetch::Table { .. } => true,
        });
        data && self.query.filters.iter().zip(&self.filter_fetch).all(|(filter, fetch)| {
            let newer = |t: &ComponentTicks| match filter.kind {
                FilterKind::Changed => t.changed > self.ref_tick,
                _ => t.added > self.ref_tick,
            };
            match *fetch {
                FilterFetch::Admitted => true,
                // SAFETY: the caller's `row` is within the column these ticks belong to.
                FilterFetch::Ticks(ticks) => newer(unsafe { &*ticks.add(row) }),
                FilterFetch::Sparse(set) => {
                    // SAFETY: as above, the world's set.
                    let set = set.map(|s| unsafe { &*s });
                    match filter.kind {
                        FilterKind::With => set.is_some_and(|s| s.contains(id)),
                        FilterKind::Without => !set.is_some_and(|s| s.contains(id)),
                        _ => set.and_then(|s| s.ticks_for(id)).is_some_and(newer),
                    }
                }
            }
        })
    }
}

/// One matching entity of a [`DynamicQuery`], lent by [`DynamicRows::fetch_next`]. Term `i` is the
/// `i`-th `read` or `write` the query was built with.
pub struct DynamicRow<'a> {
    entity: Entity,
    row: usize,
    tick: u32,
    writable: bool,
    terms: &'a [Term],
    fetch: &'a [Fetch],
}

impl DynamicRow<'_> {
    /// The entity this row is.
    #[must_use]
    pub fn entity(&self) -> Entity {
        self.entity
    }

    /// A pointer to term `term`'s value, to read: aligned to the component's layout, valid until this
    /// row is let go of.
    ///
    /// # Panics
    /// If the query has no term `term`.
    #[must_use]
    pub fn ptr(&self, term: usize) -> *const u8 {
        let size = self.terms[term].size;
        match self.fetch[term] {
            // SAFETY: `row` is in bounds for the column, so the offset stays inside its allocation.
            Fetch::Table { data, .. } => unsafe { data.add(self.row * size) },
            // SAFETY: the set is the world's, and the row matched only if it holds the entity.
            Fetch::Sparse(set) => unsafe { (*set).get_ptr(self.entity.id()) }.expect("the row matched, so it is there"),
        }
    }

    /// A pointer to term `term`'s value, to write, and the write is recorded: `Changed` sees it, as it
    /// sees a write through `&mut T`. Valid until this row is let go of.
    ///
    /// # Panics
    /// If the query has no term `term`, if the term was a `read`, or if the rows came from
    /// [`DynamicQuery::iter`], which reads only.
    pub fn ptr_mut(&mut self, term: usize) -> *mut u8 {
        let t = self.terms[term];
        assert!(self.writable, "these rows were opened to read: use `iter_mut` or a `DynamicSystem`");
        assert!(t.write, "term {term} was declared a `read`");
        match self.fetch[term] {
            Fetch::Table { data, ticks } => {
                // SAFETY: `row` is in bounds for the column and its ticks, and the column is this
                // iteration's alone to write (see where the fetch was taken).
                unsafe {
                    (*ticks.add(self.row)).changed = self.tick;
                    data.add(self.row * t.size)
                }
            }
            Fetch::Sparse(set) => {
                // SAFETY: the set is the world's; the row matched, so `sparse` names a live dense row;
                // the ticks are cells, and this row is the only one writing this entity's.
                unsafe {
                    let set = &*set;
                    let dense = set.sparse[self.entity.id() as usize] as usize;
                    (*set.ticks[dense].get()).changed = self.tick;
                    set.dense.get_unchecked_mut(dense)
                }
            }
        }
    }
}

// =========================================================================
// SYSTEM
// =========================================================================

/// The queries of a [`DynamicSystem`] while it runs, handed to its body.
pub struct DynamicQueries<'s> {
    world: &'s World,
    queries: &'s [DynamicQuery],
    ref_tick: u32,
}

impl DynamicQueries<'_> {
    /// The rows of query `index`, to read and to write its `write` terms. `changed` and `added` mean
    /// "since this system last ran", as they do for a typed system's `Query`, and the world's default
    /// query filters apply. One query's rows at a time: they borrow `self`.
    ///
    /// # Panics
    /// If the system has no query `index`.
    pub fn rows(&mut self, index: usize) -> DynamicRows<'_, '_> {
        DynamicRows::new(&self.queries[index], self.world, self.ref_tick, true, true)
    }

    /// How many queries the system has.
    #[must_use]
    pub fn len(&self) -> usize {
        self.queries.len()
    }

    /// Whether the system has no queries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.queries.is_empty()
    }

    /// The tick the system runs at — what a write through [`DynamicRow::ptr_mut`] is stamped with.
    #[must_use]
    pub fn tick(&self) -> u32 {
        self.world.tick
    }
}

type DynamicBody = Box<dyn FnMut(&mut DynamicQueries<'_>, f32) + Send + Sync>;

/// A system whose queries are [`DynamicQuery`]s: the body a scripting layer runs every frame.
///
/// Its access is the union of its queries', so the scheduler batches it with what it does not conflict
/// with and keeps it apart from what it does — a typed `Query<&mut T>` and a dynamic write of `T` are
/// the same write. Add it as any other system:
///
/// ```
/// use std::alloc::Layout;
/// use gizmo_core::prelude::*;
/// use gizmo_core::dynamic::{ComponentDescriptor, DynamicQuery, DynamicSystem};
///
/// let mut world = World::new();
/// let fuel = world.register_component(ComponentDescriptor::new("Fuel", Layout::new::<f32>())).unwrap();
/// let tank = world.spawn();
/// let mut full = 10.0f32;
/// // SAFETY: an aligned `f32`, `Fuel`'s layout, not used again.
/// unsafe { world.insert_by_key(tank, fuel, (&raw mut full).cast()) }.unwrap();
///
/// let burn = DynamicQuery::builder().write(fuel).build(&world).unwrap();
/// let mut schedule = Schedule::new();
/// schedule.add_systems(DynamicSystem::new(vec![burn], |queries, dt| {
///     let mut rows = queries.rows(0);
///     while let Some(mut row) = rows.fetch_next() {
///         // SAFETY: term 0 is `Fuel`, an `f32`.
///         unsafe { *row.ptr_mut(0).cast::<f32>() -= dt };
///     }
/// }));
/// schedule.run(&mut world, 1.0);
/// // SAFETY: `Fuel` is an `f32`.
/// assert_eq!(unsafe { *world.get_component_ptr(tank, fuel).unwrap().cast::<f32>() }, 9.0);
/// ```
pub struct DynamicSystem {
    queries: Vec<DynamicQuery>,
    last_run: u32,
    body: DynamicBody,
}

impl DynamicSystem {
    /// A system that runs `body` with `queries`, and with the time step the schedule was run with.
    pub fn new<F>(queries: Vec<DynamicQuery>, body: F) -> Self
    where
        F: FnMut(&mut DynamicQueries<'_>, f32) + Send + Sync + 'static,
    {
        // The body is boxed, and a boxed closure's type is gone by the time it runs; calling it through
        // a function item generic over `F` keeps it reachable by a hot patch (`crate::hotpatch`).
        fn call<F: FnMut(&mut DynamicQueries<'_>, f32)>(body: &mut F, queries: &mut DynamicQueries<'_>, dt: f32) {
            body(queries, dt)
        }
        let mut body = body;
        Self {
            queries,
            last_run: 0,
            body: Box::new(move |queries: &mut DynamicQueries<'_>, dt| crate::hotpatch::hot_call!(call::<F>, &mut body, queries, dt)),
        }
    }
}

impl System for DynamicSystem {
    fn run(&mut self, world: &World, dt: f32) {
        // As a typed `Query` parameter does: tick 0 is one no world uses, so it means "has not run",
        // which takes the window the schedule opened.
        let ref_tick = if self.last_run == 0 { world.change_ref_tick } else { self.last_run };
        self.last_run = world.tick;
        let mut queries = DynamicQueries { world, queries: &self.queries, ref_tick };
        (self.body)(&mut queries, dt);
    }

    fn access_info(&self) -> AccessInfo {
        let mut access = AccessInfo::new();
        for query in &self.queries {
            access.merge(query.access());
        }
        access
    }
}

impl IntoSystemConfig<()> for DynamicSystem {
    fn into_config(self) -> SystemConfig {
        SystemConfig::new(Box::new(self))
    }
}
