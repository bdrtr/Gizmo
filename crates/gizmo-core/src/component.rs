//! What makes a type a component, and where its rows are stored.
//!
//! [`Component`] is the trait every stored type implements — usually through the
//! `impl_component!` macro rather than by hand — and [`StorageType`] chooses between the
//! archetype table (dense, fast to iterate, the default) and a sparse set (cheap to add and
//! remove on a small fraction of entities).
//!
//! The storage choice is not cosmetic: it decides which query operands are legal. Table
//! storage backs the chunked/contiguous iteration paths; sparse-set components cannot be
//! served as slices and are rejected — or panic — there.
use std::any::Any;

/// Which of the world's two backing stores holds the data of a component type.
///
/// The choice is a property of the *type*, not of an entity: it comes from
/// [`Component::storage_type`], which must return the same answer on every call — see there for
/// what an inconsistent impl breaks. The two stores are disjoint: a component lives in one of
/// them, never both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageType {
    /// Inside the archetype, as a contiguous column with one row per entity.
    ///
    /// Iteration is a linear scan over that column, and this is the only storage a chunked
    /// query can hand out as a `&[T]` slice. The price is structural churn: adding or removing
    /// a table component migrates the entity into a different archetype, which copies all of
    /// its *other* components into a fresh row and swap-removes the old one (so an unrelated
    /// entity changes row as a side effect).
    ///
    /// The default, and the right answer unless the component is added and removed far more
    /// often than it is read.
    Table,
    /// Outside the archetype, in one per-type sparse set keyed by raw entity id.
    ///
    /// Adding or removing costs no archetype migration, which is what makes this suitable for
    /// churny short-lived tags. In exchange: every access is an indirection through the
    /// id → row table; archetype-level query matching cannot narrow anything, so
    /// a `With`/`Without` on a sparse component matches *every* archetype and degrades into a
    /// per-row presence test (unlike the table case, where the archetype test alone settles it);
    /// and asking a query for a contiguous slice (`iter_chunks`) panics rather than degrading.
    ///
    /// Not every code path supports it — the bundle fast path writes archetype columns only,
    /// so a bundle containing a sparse component is routed component-by-component instead
    /// (see [`Bundle::apply`]).
    SparseSet,
}

/// Data that can be attached to an entity.
///
/// There is no derive macro in this workspace; write the impl by hand or use
/// [`impl_component!`](crate::impl_component). Implementing it is the only registration a type
/// needs — the world records a component's runtime metadata the first time it sees the type.
///
/// The supertraits are load-bearing rather than decorative. `'static` because a component's
/// identity everywhere in the ECS is its `TypeId`, so two distinct components can never share
/// a type and a component can never borrow. `Send + Sync` because component storage is shared
/// across worker threads by parallel query iteration. `Clone` because the storage layer records
/// a clone thunk for every component type at registration; that thunk is what entity cloning
/// (prefab splicing) uses, and a type that cannot clone cannot be a component.
///
/// Zero-sized components are legal and cost no allocation; they are the usual shape for
/// markers such as [`IsHidden`].
pub trait Component: 'static + Any + Send + Sync + Clone {
    /// Where instances of this type are stored; see [`StorageType`].
    ///
    /// Answered by the type, not by an instance, so it must be a constant — the value is
    /// captured into the world's component metadata the first time the type is registered, and
    /// an impl that returned different values on different calls would leave the storage and
    /// the metadata disagreeing about where the data lives.
    ///
    /// Defaults to [`StorageType::Table`].
    fn storage_type() -> StorageType {
        StorageType::Table
    }
}

/// Writes an empty [`Component`] impl for one or more types, optionally choosing their
/// [`StorageType`].
///
/// ```
/// # #[derive(Clone)] struct Position; #[derive(Clone)] struct Velocity;
/// # #[derive(Clone)] struct Frozen;   #[derive(Clone)] struct Stunned;
/// // Neither is in the prelude: `StorageType` has to be nameable in *your* scope for the
/// // `; $storage` argument below, and `Component` for `storage_type()`.
/// use gizmo_core::component::{Component, StorageType};
/// use gizmo_core::impl_component;
///
/// impl_component!(Position, Velocity);                       // default: Table storage
/// impl_component!(Frozen, Stunned; StorageType::SparseSet);  // explicit storage
///
/// assert_eq!(Position::storage_type(), StorageType::Table);
/// assert_eq!(Frozen::storage_type(), StorageType::SparseSet);
/// ```
///
/// The macro only writes the impl: the types must already satisfy `Component`'s supertraits
/// (`'static + Send + Sync + Clone`), and the expansion is an ordinary trait impl, so the usual
/// orphan rule applies — outside gizmo-core itself, only types local to the invoking crate can
/// be passed.
///
/// The `; $storage` argument is an expression expanded in the *caller's* scope, so whatever
/// path it names (`StorageType::SparseSet`, `gizmo_core::component::StorageType::SparseSet`, …)
/// has to resolve there. Only the storage-less form accepts a trailing comma after the last
/// type; `impl_component!(A, B,; …)` does not parse.
///
/// `#[macro_export]` puts it at the crate root: `gizmo_core::impl_component!`.
#[macro_export]
macro_rules! impl_component {
    ($($t:ty),+ $(,)?) => {
        $(
            impl $crate::Component for $t {}
        )+
    };
    ($($t:ty),+ ; $storage:expr) => {
        $(
            impl $crate::Component for $t {
                fn storage_type() -> $crate::component::StorageType {
                    $storage
                }
            }
        )+
    };
}

// --- Hiyerarşi (Scene Graph) Bileşenleri ---
/// Up-link from a child to its parent, carrying the parent's raw
/// [`Entity::id`](crate::Entity::id) — a slot index with the generation stripped off.
///
/// Because the generation is gone the value cannot distinguish a live parent from a recycled
/// id, and nothing revalidates it when the parent is despawned. Resolve it through
/// [`World::entity`](crate::World::entity), which returns `None` for an id that is dead or has
/// no storage, instead of trusting the number.
///
/// This is only half of a link: [`Children`] on the parent is the other half. Nothing
/// synchronises the two halves automatically, so adding or editing this component directly
/// leaves the parent's `Children` list stale.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Parent(pub u32);

/// Down-link from a parent to its children: raw entity ids (no generations), in the order they
/// were attached.
///
/// The order is stable and meaningful. [`HierarchyExt::add_child`](crate::HierarchyExt::add_child)
/// appends and skips ids already in the list, and `remove_child` retains, so iteration over a
/// `Children` list is deterministic across a run — which is what lets hierarchy walks be
/// replayed. Presence of this component is also what marks an archetype as a candidate for
/// [`World::sort_archetype_hierarchy`](crate::World::sort_archetype_hierarchy), which permutes
/// rows to put parents and children next to each other.
///
/// Entries are not validated: a child despawned outside `HierarchyExt` leaves a dangling id.
/// Duplicates and cycles are impossible through `HierarchyExt` (it rejects self-parenting and
/// any reparent that would close a loop) but perfectly possible via direct component writes or
/// a hand-edited scene file, so `despawn_recursive` carries a visited set rather than assuming
/// the graph is acyclic.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Children(pub Vec<u32>);

/// Human-readable label for an entity — for editor lists, logs and scene files.
///
/// Purely descriptive: nothing enforces uniqueness, nothing indexes it, and gizmo-core itself
/// never reads it. Two entities may hold the same name, and an absent `EntityName` is normal
/// (most entities never get one).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EntityName(pub String);

impl EntityName {
    /// Allocates an owned copy of `name`. The field is public, so an already-owned `String` can
    /// be moved in as `EntityName(s)` instead of paying for a second allocation here.
    ///
    /// No validation and no normalisation: empty, blank and already-used names are all accepted.
    /// That matters because the derived `PartialEq` compares the stored text byte for byte —
    /// `"Cube"` and `"cube "` are different labels to anything that searches by name.
    pub fn new(name: &str) -> Self {
        Self(name.to_string())
    }
}

/// Zero-sized marker meaning "do not display this entity".
///
/// It carries no data, so visibility is binary and expressed structurally: hide with
/// `add_component(e, IsHidden)`, show with `remove_component::<IsHidden>(e)`. gizmo-core
/// attaches no behaviour to it whatsoever. A hidden entity is still alive and still visited by
/// every query that does not explicitly exclude it.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct IsHidden;

/// Zero-sized marker meaning "this entity is the editor's own tooling, not scene content":
/// grids, light icons, selection boxes, handles.
///
/// # Why this is in the ECS floor and not in the editor
///
/// Because four unrelated layers need the answer and only this one is below all of them: the
/// hierarchy panel hides these rows, the studio's game view refuses to draw them, the windowed
/// app's editor runtime skips them, and — the one that matters most — [`crate`]'s sibling
/// `gizmo-scene` leaves them out of a saved scene. That last consumer cannot see a renderer
/// component: scene sits beside the renderer in the graph, not above it.
///
/// Until this existed, all four asked the same question by **string prefix on the entity name**
/// (`"Editor "` / `"Highlight Box"`), written out four times. That works only by convention and
/// fails in a way nobody would debug quickly: an office scene with a desk named "Editor Desk"
/// loses it from the hierarchy and from every save.
///
/// The name rule is still honoured — see [`is_editor_only`] — because scenes saved before this
/// component existed carry names and not markers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EditorOnly;

/// Is this entity the editor's own tooling rather than scene content?
///
/// The single place that decides. Callers pass whatever they have — a marker lookup, a name, or
/// both — because the four call sites reach the world differently and a shared signature that
/// takes `&World` would force three of them to look up something they already hold.
///
/// The legacy half (the name rule) is a **transition**, not a design: it exists so that scenes
/// written before [`EditorOnly`] still round-trip. New tooling entities should carry the marker
/// and need no particular name.
#[inline]
pub fn is_editor_only(has_marker: bool, name: Option<&str>) -> bool {
    has_marker || name.is_some_and(|n| n.starts_with("Editor ") || n == "Highlight Box")
}

/// Zero-sized marker meaning "soft-deleted": the entity is still alive with its id, handles and
/// components intact, but is meant to be skipped by processing.
///
/// gizmo-core never acts on it. It is a convention for the layers above, which exclude it with
/// `Without<IsDeleted>` — the rigid-body physics systems do exactly that — and later despawn
/// the marked entities for real. Because nothing is destroyed when the marker is added,
/// removing it again restores the entity exactly; that reversibility is the point of the flag.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct IsDeleted;

/// A request to populate this entity from a named prefab, left on the entity until something
/// fulfils it.
///
/// The string is an opaque key: gizmo-core neither resolves nor validates it. No system in this
/// workspace consumes the component — the Lua scripting bridge is its only in-tree producer —
/// so it does nothing unless the application runs its own resolver, which must also remove the
/// component afterwards, since nothing clears it automatically.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PrefabRequest(pub String);

impl PrefabRequest {
    /// Stores an owned copy of `name` as the request key.
    ///
    /// Infallible and unvalidated: gizmo-core holds no prefab catalogue, so an empty or
    /// misspelled key is indistinguishable here from a good one and can only fail later,
    /// wherever the application resolves it.
    pub fn new(name: &str) -> Self {
        Self(name.to_string())
    }
    /// The request key, borrowed from the component — no allocation, no copy.
    ///
    /// May be empty, and is never checked against anything (see
    /// [`new`](PrefabRequest::new)). The tuple field is public, so this is a reading
    /// convenience rather than encapsulation: `req.0` reaches the same `String` and can
    /// replace it.
    pub fn name(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for EntityName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Renderer-independent request for a mesh, as an asset key that the renderer's asset-loading
/// pass turns into a GPU `Mesh` component.
///
/// The key is not simply a file path. The loader recognises the built-in primitives
/// `"standard_cube"`, `"inverted_cube"`, `"plane"`, `"sphere"` and `"sprite_quad"`, the
/// `"gltf_mesh_<file>.glb…"` / `"gltf_mesh_<file>.gltf…"` form for a mesh inside a glTF scene,
/// and `"obj:<path>"`; anything else is treated as an OBJ path. Nothing validates the key when
/// the component is attached, and a key that fails to load does not fail the frame: the entity
/// is given a stand-in mesh and the failure is only logged.
///
/// Upload happens only while the entity has *no* `Mesh` yet, so this is a one-shot request:
/// editing the string afterwards changes nothing until the `Mesh` component is removed.
/// Keeping the key on the entity is also what lets a scene be saved back out.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MeshSource(pub String);

/// Renderer-independent material description, converted into a GPU `Material` by the renderer's
/// asset-loading pass.
///
/// Like [`MeshSource`] this is consumed once — the conversion runs only while the entity has no
/// `Material` component, so later edits to these fields do not reach the GPU material.
///
/// The numeric fields are carried through verbatim: nothing here clamps, normalises or
/// colour-converts them, so the ranges named below are conventions rather than guarantees.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MaterialSource {
    /// Base colour and opacity as `[r, g, b, a]`, conventionally `0.0..=1.0` per channel.
    ///
    /// The fourth channel is the only opacity control on this struct — there is no separate
    /// alpha, blend-mode or cutoff field. It is also the one channel group that has a texture
    /// counterpart: [`texture_source`](Self::texture_source) is an *albedo* map, and no other
    /// field here can be driven by a texture.
    pub albedo: [f32; 4],
    /// Microfacet roughness: `0.0` is a perfect mirror, `1.0` fully diffuse.
    ///
    /// A shading parameter, not a shading mode — which mode runs is decided solely by
    /// [`unlit`](Self::unlit), and writing a roughness never changes it.
    pub roughness: f32,
    /// Metalness: `0.0` for a dielectric, `1.0` for a metal.
    ///
    /// Unlike [`roughness`](Self::roughness) this is conceptually two-valued rather than a
    /// continuum — the two ends are the physical materials, and an intermediate number describes
    /// a blend between them, which is normally only wanted where a texture crosses from one to
    /// the other.
    pub metallic: f32,
    /// Shading mode selector, despite the name and the type: it is a *tri-state* float, not a
    /// boolean.
    ///
    /// The renderer thresholds it — above `1.5` the material becomes a skybox, above `0.5` it
    /// becomes unlit, and anything else (including `0.0`) is lit PBR. Values in between behave
    /// like the lower bucket, and negative values are lit like zero.
    ///
    /// `MaterialSource` derives no `Default`, so there is no `..Default::default()` shorthand:
    /// every literal must state this field, and `0.0` is the lit-PBR choice.
    pub unlit: f32,
    /// Path of the albedo texture to load, or `None` for an untextured material.
    ///
    /// `None` and a failed load are handled the same way — the material falls back to the
    /// shared default white texture — except that a failure also logs a warning, so a
    /// mistyped path renders as plain white rather than failing loudly.
    pub texture_source: Option<String>,
}

impl_component!(Parent, Children, EntityName, IsHidden, EditorOnly, PrefabRequest, IsDeleted, MeshSource, MaterialSource);

// ============================================================
//  Bundle Trait
// ============================================================

/// A group of components that can be attached to an entity as one unit.
///
/// Implemented blanket-wise for every [`Component`] (a lone component is a one-element bundle)
/// and for tuples of bundles up to 16 elements, which nest freely — `(A, (B, C))` is a bundle.
///
/// There are two ways into the world and an implementor owns both:
/// [`apply`](Bundle::apply) inserts the components one at a time through the world, and
/// [`write_to_archetype`](Bundle::write_to_archetype) blits them directly into archetype
/// columns. They are not interchangeable — the second cannot store `SparseSet` components at
/// all — and the caller picks: `World::spawn_bundle` always goes through `apply`;
/// `World::add_bundle` writes the archetype directly; `World::spawn_batch` spawns its first
/// entity through `apply` to discover the archetype and appends the rest directly. Whenever
/// [`get_infos`](Bundle::get_infos) declares a `SparseSet` member, the direct paths give up and
/// fall back to `apply`.
///
/// Since `apply` has a do-nothing default, a hand-written bundle that implements only
/// `write_to_archetype` compiles and then silently attaches nothing wherever the `apply` path
/// is used. Implement both.
pub trait Bundle {
    /// Runtime metadata for every component this bundle will write, in the order it writes
    /// them.
    ///
    /// A property of the type — it takes no `self` and is called before any data moves — so it
    /// must describe exactly what `apply`/`write_to_archetype` produce. The world relies on it
    /// twice: to work out the destination archetype, and to spot `SparseSet` members that make
    /// the fast path unusable. Nested bundles simply concatenate their infos, and repeated
    /// component types are *not* deduplicated here; the world folds duplicates away when it
    /// builds the archetype's sorted type set, but a duplicate still means the same column is
    /// written twice.
    fn get_infos() -> Vec<crate::archetype::ComponentInfo>;
    /// Moves the bundle's components straight into `arch`'s columns at `row`, bypassing the
    /// world.
    ///
    /// The fast path, and a narrow one: it can only reach `Table`-storage components, it
    /// updates no entity location and fires no hooks, and it takes `arch` as given rather than
    /// choosing it — the caller is responsible for the row belonging to the right entity in the
    /// right archetype.
    ///
    /// Each component is appended when its column is still short of `row`, and otherwise raw-
    /// written into slot `row` — treating that slot as *uninitialised*, so an already-live value
    /// there is overwritten without being dropped (a leak for anything owning an allocation).
    /// Either way the row's ticks are reset to `tick` in both fields, i.e. it reads as freshly
    /// added, not merely changed.
    ///
    /// # Safety
    /// `arch` must contain the component columns that `Self::get_infos()` returns, and `_row`
    /// must be a valid row reserved in this archetype. The data is copied raw; ownership is
    /// transferred to the archetype.
    unsafe fn write_to_archetype(self, arch: &mut crate::archetype::Archetype, _row: usize, tick: u32);
    /// Attaches the bundle to an entity that already exists, component by component, through
    /// `World::add_component`.
    ///
    /// The storage-agnostic route: each component reaches whichever store its
    /// [`StorageType`] says, and the normal `on_add`/`on_set` hooks fire. It is the only route
    /// that can place a `SparseSet` component, and the slow one — every table component the
    /// entity does not already have migrates it to another archetype, so an n-component bundle
    /// can pay n migrations where the direct path pays one.
    ///
    /// The default body does nothing at all. It exists so that a bundle which only supports the
    /// archetype fast path still compiles; an implementor who forgets to override it gets an
    /// entity with none of the components and no error.
    fn apply(self, _world: &mut crate::world::World, _entity: crate::entity::Entity) where Self: Sized {}
}

/// A bundle with one extra component appended, as produced by [`BundleExt::with`].
///
/// Composition is purely type-level: `get_infos` lists `B`'s components followed by `C`, and
/// `write_to_archetype` writes `B` first and `C` second. If `C` also occurs in `B` the later
/// write wins by overwriting the slot in place — without dropping what was there, which leaks
/// whatever that value owned. An archetype with no column for `C` (a `SparseSet` `C`, or simply
/// the wrong archetype) is a bare `unwrap` panic, without the explanation the single-component
/// path prints.
///
/// **Known limitation.** This type does not override [`Bundle::apply`], so it inherits the
/// no-op default: passing a `DynamicBundle` to `World::spawn_bundle` — or to `World::add_bundle`
/// when it contains a `SparseSet` component — produces an entity with *none* of the components,
/// silently. Only the direct archetype path (`World::add_bundle` with all-table components)
/// stores anything. Nothing in this workspace constructs a `DynamicBundle`; treat it as
/// experimental until `apply` is implemented.
pub struct DynamicBundle<B: Bundle, C: Component> {
    /// The base bundle, written before `component` and listed first in `get_infos`.
    pub bundle: B,
    /// The appended component, written last — so on a type collision with `bundle` this is the
    /// value that survives.
    pub component: C,
}

impl<B: Bundle, C: Component> Bundle for DynamicBundle<B, C> {
    fn get_infos() -> Vec<crate::archetype::ComponentInfo> {
        let mut infos = B::get_infos();
        infos.push(crate::archetype::ComponentInfo::of::<C>());
        infos
    }

    unsafe fn write_to_archetype(self, arch: &mut crate::archetype::Archetype, row: usize, tick: u32) {
        self.bundle.write_to_archetype(arch, row, tick);
        let col = arch.get_column_mut(std::any::TypeId::of::<C>()).unwrap();
        if col.len() <= row {
            col.push_raw(&self.component as *const _ as *const u8, tick);
            std::mem::forget(self.component);
        } else {
            let ptr = col.get_mut_ptr(row) as *mut C;
            std::ptr::write(ptr, self.component);
            *col.ticks_ptr_mut().add(row) = crate::archetype::ComponentTicks::new(tick);
        }
    }
}

/// Chaining sugar for growing a bundle one component at a time.
///
/// Blanket-implemented for every [`Bundle`], so `a.with(b).with(c)` type-checks for any
/// components; the result is a nest of [`DynamicBundle`]s, whose limitations apply — read them
/// before using this.
pub trait BundleExt: Bundle + Sized {
    /// Appends `component` to this bundle and returns the combined value.
    ///
    /// Pure value construction: nothing touches a world until the result is spawned or added,
    /// and both operands are moved. It is additive, never a replacement — appending a component
    /// type the bundle already carries produces a bundle listing that type twice rather than
    /// substituting the earlier one.
    fn with<C: Component>(self, component: C) -> DynamicBundle<Self, C> {
        DynamicBundle { bundle: self, component }
    }
}

impl<T: Bundle> BundleExt for T {}

impl<T: Component> Bundle for T {
    fn get_infos() -> Vec<crate::archetype::ComponentInfo> {
        vec![crate::archetype::ComponentInfo::of::<T>()]
    }

    fn apply(self, world: &mut crate::world::World, entity: crate::entity::Entity) {
        world.add_component(entity, self);
    }

    unsafe fn write_to_archetype(self, arch: &mut crate::archetype::Archetype, row: usize, tick: u32) {
        let col = arch.get_column_mut(std::any::TypeId::of::<T>()).unwrap_or_else(|| {
            panic!(
                "Component column for `{}` missing in Archetype. The bundle fast-path \
                 (write_to_archetype) only handles Table-storage components; SparseSet \
                 components must be routed via World::add_component. spawn_batch already \
                 falls back for sparse bundles — reaching here means another bundle path \
                 wrote a sparse component into the archetype.",
                std::any::type_name::<T>()
            )
        });
        if col.len() <= row {
            col.push_raw(&self as *const _ as *const u8, tick);
            std::mem::forget(self);
        } else {
            let ptr = col.get_mut_ptr(row) as *mut T;
            std::ptr::write(ptr, self);
            *col.ticks_ptr_mut().add(row) = crate::archetype::ComponentTicks::new(tick);
        }
    }
}

macro_rules! impl_bundle_tuple {
    ($($name:ident),*) => {
        #[allow(non_snake_case)]
        impl<$($name: crate::component::Bundle),*> Bundle for ($($name,)*) {
            fn get_infos() -> Vec<crate::archetype::ComponentInfo> {
                let mut infos = Vec::new();
                $(
                    infos.extend(<$name as crate::component::Bundle>::get_infos());
                )*
                infos
            }

            fn apply(self, world: &mut crate::world::World, entity: crate::entity::Entity) {
                let ($($name,)*) = self;
                $(
                    $name.apply(world, entity);
                )*
            }

            unsafe fn write_to_archetype(self, arch: &mut crate::archetype::Archetype, row: usize, tick: u32) {
                let ($($name,)*) = self;
                $(
                    $name.write_to_archetype(arch, row, tick);
                )*
            }
        }
    };
}

impl_bundle_tuple!(A);
impl_bundle_tuple!(A, B);
impl_bundle_tuple!(A, B, C);
impl_bundle_tuple!(A, B, C, D);
impl_bundle_tuple!(A, B, C, D, E);
impl_bundle_tuple!(A, B, C, D, E, F);
impl_bundle_tuple!(A, B, C, D, E, F, G);
impl_bundle_tuple!(A, B, C, D, E, F, G, H);
impl_bundle_tuple!(A, B, C, D, E, F, G, H, I);
impl_bundle_tuple!(A, B, C, D, E, F, G, H, I, J);
impl_bundle_tuple!(A, B, C, D, E, F, G, H, I, J, K);
impl_bundle_tuple!(A, B, C, D, E, F, G, H, I, J, K, L);
impl_bundle_tuple!(A, B, C, D, E, F, G, H, I, J, K, L, M);
impl_bundle_tuple!(A, B, C, D, E, F, G, H, I, J, K, L, M, N);
impl_bundle_tuple!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O);
impl_bundle_tuple!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P);


#[cfg(test)]
mod editor_only_tests {
    use super::*;

    /// The predicate itself, both halves.
    #[test]
    fn the_marker_wins_and_the_legacy_names_still_count() {
        // The marker alone is enough — no name needed, which is the point of having it.
        assert!(is_editor_only(true, None));
        assert!(is_editor_only(true, Some("Enemy")));

        // Legacy: scenes written before the marker existed carry only names.
        assert!(is_editor_only(false, Some("Editor Grid")));
        assert!(is_editor_only(false, Some("Editor Light Icon 1")));
        assert!(is_editor_only(false, Some("Highlight Box")));

        // Content stays content.
        assert!(!is_editor_only(false, Some("Enemy")));
        assert!(!is_editor_only(false, None));
        assert!(
            !is_editor_only(false, Some("Editor")),
            "the legacy rule needs the trailing space; a scene object merely named \"Editor\" is \
             not tooling"
        );
        assert!(
            !is_editor_only(false, Some("My Editor Desk")),
            "the prefix is anchored — only names that START with it"
        );
    }

    /// The name rule's failure mode, stated so it is not mistaken for a design.
    ///
    /// A scene object legitimately named "Editor Desk" is invisible in the hierarchy and dropped
    /// from every save. That is why the marker exists; the rule survives only for old scenes.
    #[test]
    fn the_legacy_name_rule_still_swallows_a_legitimately_named_object() {
        assert!(
            is_editor_only(false, Some("Editor Desk")),
            "documented wart: this is what the marker is for"
        );
    }

    /// Nothing may re-implement the rule.
    ///
    /// It lived in **eight** places — the hierarchy panel (twice), the windowed app's editor
    /// runtime, two filters in `gizmo-scene`'s snapshot, one in its scene writer, the studio's
    /// protected-entity set, its delete guard, its select-all shortcut and its play-mode hide.
    /// Eight copies of one string comparison, and every one of them had to agree about the
    /// trailing space.
    #[test]
    fn the_editor_only_rule_is_written_once() {
        let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .expect("crates/gizmo-core sits two levels below the workspace root")
            .to_path_buf();
        if !workspace.join("crates/gizmo-studio").is_dir() {
            return; // packaged crate
        }

        let mut sources = Vec::new();
        collect_rs(&workspace.join("crates"), &mut sources);
        collect_rs(&workspace.join("demo"), &mut sources);
        assert!(sources.len() > 100, "source walk found only {} files", sources.len());

        let this_file = std::path::Path::new(file!()).file_name().unwrap();
        let mut offenders = Vec::new();
        for path in &sources {
            if path.file_name() == Some(this_file) {
                continue;
            }
            let text = std::fs::read_to_string(path).unwrap_or_default();
            // Production code only. A test that asserts "this got filtered out" names the strings
            // on purpose and is checking the outcome, not re-deciding it — `gizmo-scene`'s save
            // test does exactly that. Cutting at the first `#[cfg(test)]` is approximate and
            // deliberately so: the cost of a miss is a test module that could re-implement the
            // rule unnoticed, and a test module that did would be caught by its own assertions
            // disagreeing with production.
            let code = text.split("#[cfg(test)]").next().unwrap_or("");
            for (i, line) in code.lines().enumerate() {
                let t = line.trim_start();
                if t.starts_with("//") || t.starts_with("///") {
                    continue;
                }
                if line.contains("starts_with(\"Editor \")") || line.contains("== \"Highlight Box\"") {
                    offenders.push(format!("{}:{}", path.display(), i + 1));
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "the editor-only rule must be asked of `component::is_editor_only`, not re-written. \
             Offenders:\n{}",
            offenders.join("\n")
        );
    }

    fn collect_rs(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|n| n == "target") {
                    continue;
                }
                collect_rs(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
}
