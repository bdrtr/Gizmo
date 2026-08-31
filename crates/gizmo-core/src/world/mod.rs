//! The [`World`]: entity storage, components, resources, and the operations over them.
//!
//! Everything the simulation owns lives here. Structural changes — spawning, despawning,
//! adding or removing a component — move entities between archetypes and invalidate row
//! indices, which is why systems that need them defer through `Commands` instead.
use crate::archetype::index::ArchetypeIndex;
use crate::archetype::{ComponentInfo, EntityLocation};
use crate::entity::Entity;

use std::any::TypeId;
use std::collections::HashMap;
use std::sync::RwLock;

mod component_ops;
mod entity_lifecycle;
mod hierarchy_sort;
pub mod hooks;
mod introspect;
mod query;
mod registration;
pub mod resources;

pub use self::hooks::*;
pub use self::introspect::{short_type_name, ArchetypeSummary, ComponentSummary, WorldStats};
pub use self::resources::*;
pub use crate::entity::allocator::Entities;

/// The ECS container: entity ids, the archetype tables holding their component data, the
/// sparse-set side storage, the type-keyed resource map, and the change-detection ticks.
///
/// Access rules the rest of the engine leans on:
/// - From `&World` you can build read-only queries and resource guards only. A mutable
///   query needs `&mut World` ([`World::query_mut`]) or the `unsafe`
///   [`World::query_unchecked`] escape hatch the parallel scheduler uses.
/// - Resources are locked individually, so guards for *different* resources coexist
///   happily; a second conflicting guard for the *same* resource fails rather than blocks.
///
/// Iteration order: the sequential query accessors (`iter`, `iter_mut`, `iter_chunks`, …) walk
/// archetypes in table-index order and rows in storage order; the parallel ones
/// (`par_for_each`, `par_for_each_mut`) go through a work-stealing pool and define no order.
/// The sequential order is reproducible for a given sequence of mutations, but it is not spawn
/// order and it is not stable across mutations — [`World::despawn`] and archetype migration
/// both swap-remove, moving an archetype's last row into the vacated slot, and
/// [`World::compact`] renumbers archetypes outright.
pub struct World {
    // Entity'den bağımsız global veriler (Time, WindowSize, Input vs.)
    resources: HashMap<TypeId, RwLock<Box<dyn std::any::Any + Send + Sync>>>,

    /// Entity ID → archetype location. Provides a fast O(1) lookup.
    /// entity_id is used as the index.
    entity_locations: Vec<EntityLocation>,

    /// Archetype-based storage — all component data is held here.
    pub(crate) archetype_index: ArchetypeIndex,

    /// The runtime component metadata cache. It is required in order to create archetype columns.
    component_infos: HashMap<TypeId, ComponentInfo>,

    pub(crate) component_hooks: HashMap<TypeId, ComponentHooks>,
    pub(crate) sparse_sets: HashMap<TypeId, crate::archetype::sparse_set::ComponentSparseSet>,

    despawn_hooks: Vec<DespawnHook>,
    entities_to_despawn: Vec<Entity>,
    is_despawning: bool,
    pub(crate) entity_observers:
        HashMap<TypeId, Box<dyn crate::observer::EntityObserverMap>>,
    /// Listeners for events that belong to no entity and no component type — the third door,
    /// keyed only by the event's own `TypeId`. See [`World::observe_global`].
    ///
    /// Separate from `entity_observers` rather than folded into it under a sentinel entity: a
    /// global event has no target to bubble from, so sharing the map would mean a walk that has
    /// to be skipped and an `On::entity` that has to be lied about.
    pub(crate) global_observers: HashMap<TypeId, Box<dyn std::any::Any + Send + Sync>>,
    /// Frame counter stamped into a component's `ComponentTicks` every time it is written;
    /// with `change_ref_tick` it is the whole of change detection.
    ///
    /// Starts at 1 and is advanced only by [`World::increment_tick`] and
    /// [`World::begin_change_frame`], both of which wrap on overflow and skip 0 — so 0 is
    /// never a valid stamp. `Changed<T>`/`Added<T>` compare with a plain `>`, so the single
    /// frame on which the counter wraps past `u32::MAX` misreports; nothing resets it, so
    /// reaching that needs ~4.3e9 frames in one session.
    ///
    /// It is public so snapshot/restore code can put it back, but writing it without
    /// restoring `change_ref_tick` to a consistent value desynchronises change detection.
    pub tick: u32,
    /// The change detection reference tick: the `Changed<T>`/`Added<T>` filters compare
    /// against this value with `ticks.changed > change_ref_tick`.
    /// At the start of every frame the Schedule sets it to the previous frame's tick; that way
    /// "the ones changed since the last frame" are reported correctly. (It used to be
    /// `== tick`, and since the tick never advanced it matched either nothing or everything.)
    pub change_ref_tick: u32,

    /// Component types whose presence hides an entity from every **system** query.
    ///
    /// Empty by default, so the feature costs nothing until a game asks for it. Reached through
    /// [`World::default_query_filters`] / [`World::default_query_filters_mut`]; see
    /// [`DefaultQueryFilters`](crate::query::DefaultQueryFilters) for where it applies and — as
    /// importantly — where it deliberately does not.
    pub(crate) default_query_filters: crate::query::DefaultQueryFilters,

    /// Set by [`World::stop_propagation`] and read by `trigger` after each listener returns.
    ///
    /// It lives on the world rather than in a return value because the return channel the walk
    /// needed already existed once the listener was handed a `&mut World` — and because adding
    /// it to the listener's signature would have changed that signature twice in two commits.
    ///
    /// `trigger` saves and restores it around its own walk, so a nested dispatch cancelling
    /// itself does not cancel the walk it was called from.
    pub(crate) propagation_stopped: bool,
}

impl World {
    /// Creates an empty world already holding the two resources the rest of the ECS assumes
    /// are always there: the deferred [`CommandQueue`](crate::commands::CommandQueue) and
    /// the entity allocator [`Entities`]. `spawn`, `is_alive`, `entity_count`,
    /// `iter_alive_entities` and friends panic if `Entities` is later removed.
    ///
    /// Storage starts with one archetype (id 0) holding no columns — every freshly spawned,
    /// component-less entity is a row in it. `tick` starts at 1 and `change_ref_tick` at 0,
    /// so until the first [`World::begin_change_frame`] every component matches both
    /// `Added<T>` and `Changed<T>`.
    pub fn new() -> Self {
        let mut world = Self {
            resources: HashMap::new(),
            entity_locations: Vec::new(),
            archetype_index: ArchetypeIndex::new(),
            component_infos: HashMap::new(),
            component_hooks: HashMap::new(),
            sparse_sets: HashMap::new(),
            despawn_hooks: Vec::new(),
            entities_to_despawn: Vec::new(),
            is_despawning: false,
            entity_observers: HashMap::new(),
            global_observers: HashMap::new(),
            tick: 1,
            change_ref_tick: 0,
            default_query_filters: crate::query::DefaultQueryFilters::new(),
            propagation_stopped: false,
        };
        world.insert_resource(crate::commands::CommandQueue::new());
        world.insert_resource(Entities::new());
        world
    }

    fn run_hooks<F>(&mut self, type_id: TypeId, mut f: F)
    where
        F: FnMut(&mut ComponentHooks, &mut World),
    {
        let mut hooks = self.component_hooks.remove(&type_id);
        if let Some(ref mut h) = hooks {
            f(h, self);
        }
        if let Some(h) = hooks {
            if let Some(existing) = self.component_hooks.get_mut(&type_id) {
                existing.on_add.extend(h.on_add);
                existing.on_set.extend(h.on_set);
                existing.on_replace.extend(h.on_replace);
                existing.on_remove.extend(h.on_remove);
            } else {
                self.component_hooks.insert(type_id, h);
            }
        }
    }

    /// Increments the local tick counter, guaranteeing it skips 0 on wrap.
    pub fn increment_tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        if self.tick == 0 {
            self.tick = 1;
        }
        tracing::trace!(tick = self.tick, "increment_tick");

        // Apply topological memory alignment for caching locality
        self.sort_archetype_hierarchy();
    }

    /// Opens the change-detection window at the start of a frame: sets this frame's
    /// comparison reference to `ref_tick` (the previous run's tick) and advances the world
    /// tick for this frame. The `Changed<T>`/`Added<T>` filters compare with
    /// `ticks.changed > change_ref_tick`. Returns the new tick.
    /// (Unlike `increment_tick`, which has a sort side-effect, only the counter advances.)
    pub fn begin_change_frame(&mut self, ref_tick: u32) -> u32 {
        self.change_ref_tick = ref_tick;
        self.tick = self.tick.wrapping_add(1);
        if self.tick == 0 {
            self.tick = 1;
        }
        tracing::trace!(tick = self.tick, ref_tick, "begin_change_frame");
        self.tick
    }

    /// Processes the deferred command queue (CommandQueue).
    /// Entity add/remove operations are thereby applied in batch without suffering a deadlock.
    pub fn apply_commands(&mut self) {
        let queue_opt = self
            .get_resource::<crate::commands::CommandQueue>()
            .map(|q| (*q).clone());
        if let Some(queue) = queue_opt {
            queue.apply(self);
        }
    }
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::Children;

    #[derive(Clone, PartialEq, Debug)]
    struct Transform(f32);
    impl crate::component::Component for Transform {}

    #[test]
    fn test_sort_archetype_hierarchy() {
        let mut world = World::new();

        // 5 entity oluşturalım: e0, e1, e2, e3, e4
        let e0 = world.spawn();
        let e1 = world.spawn();
        let e2 = world.spawn();
        let e3 = world.spawn();
        let e4 = world.spawn();

        // Hepsi aynı bileşenlere sahip olsun (aynı archetype'a girmeleri için)
        // Sırasıyla Transform ekliyoruz:
        world.add_component(e0, Transform(0.0));
        world.add_component(e1, Transform(1.0));
        world.add_component(e2, Transform(2.0));
        world.add_component(e3, Transform(3.0));
        world.add_component(e4, Transform(4.0));

        // Hiyerarşi kuralım: e0'ın çocukları e3 ve e4 olsun.
        // Başlangıçta e0(0), e1(1), e2(2), e3(3), e4(4) sırasıyla dizilidir.
        world.add_component(e0, Children(vec![e3.id(), e4.id()]));

        // Sadece e0'da Children olunca farklı archetype'a geçer (Archetype değişimi).
        // Bu yüzden hepsine Children eklemeliyiz ki AYNI archetype'da kalsınlar.
        world.add_component(e1, Children(vec![]));
        world.add_component(e2, Children(vec![]));
        world.add_component(e3, Children(vec![]));
        world.add_component(e4, Children(vec![]));

        // Şu an hepsi (Transform, Children) archetype'ında.
        // Beklenen indeksler: e0, e1, e2, e3, e4.

        // Hiyerarşi kaydırmasını çalıştır!
        world.sort_archetype_hierarchy();

        // Kontrol edelim. e0'dan hemen sonra e3 ve e4 gelmeli.
        let loc0 = world.entity_location(e0.id());
        let loc3 = world.entity_location(e3.id());
        let loc4 = world.entity_location(e4.id());

        assert_eq!(
            loc0.row + 1,
            loc3.row,
            "e3 (child), e0 (parent)'dan hemen sonra gelmeli"
        );
        assert_eq!(
            loc0.row + 2,
            loc4.row,
            "e4 (child), e3'ten hemen sonra gelmeli"
        );

        // Diğerleri (e1 ve e2) kaydırılmış olmalı.
        let loc1 = world.entity_location(e1.id());
        let loc2 = world.entity_location(e2.id());
        assert!(
            loc1.row > loc4.row || loc2.row > loc4.row,
            "Bağımsız entityler sona itilmeli"
        );
    }

    #[test]
    fn test_sort_archetype_hierarchy_deep() {
        let mut world = World::new();

        let e0 = world.spawn();
        let e1 = world.spawn();
        let e2 = world.spawn();
        let e3 = world.spawn();

        world.add_component(e0, Transform(0.0));
        world.add_component(e1, Transform(1.0));
        world.add_component(e2, Transform(2.0));
        world.add_component(e3, Transform(3.0));

        // e0 -> e1 -> e2 -> e3 zinciri
        world.add_component(e0, Children(vec![e1.id()]));
        world.add_component(e1, Children(vec![e2.id()]));
        world.add_component(e2, Children(vec![e3.id()]));
        world.add_component(e3, Children(vec![]));

        world.sort_archetype_hierarchy();

        let l0 = world.entity_location(e0.id());
        let l1 = world.entity_location(e1.id());
        let l2 = world.entity_location(e2.id());
        let l3 = world.entity_location(e3.id());

        assert_eq!(l0.row + 1, l1.row);
        // Not: Algoritma şu an sadece doğrudan çocukları hemen arkasına koyar.
        // e1 işlendiğinde e2 onun arkasına geçer, e2 işlendiğinde e3 onun arkasına geçer.
        // Sonuçta e0, e1, e2, e3 dizilimi kendiliğinden oluşur (visited mantığı).
        assert_eq!(l1.row + 1, l2.row);
        assert_eq!(l2.row + 1, l3.row);
    }


    #[test]
    fn spawn_despawn_generation() {
        let mut world = World::new();
        let e1 = world.spawn();
        world.despawn(e1);
        
        let e2 = world.spawn(); // aynı id, farklı generation
        assert_eq!(e1.id(), e2.id());
        assert_ne!(e1.generation(), e2.generation());
        
        // Eski handle artık geçersiz
        assert!(!world.is_alive(e1));
        assert!(world.is_alive(e2));
    }

    #[test]
    fn despawn_updates_swapped_entity_location() {
        #[derive(Clone)]
        struct TestComp(i32);
        impl crate::component::Component for TestComp {}

        let mut world = World::new();
        world.register_component_type::<TestComp>();
        
        let e1 = world.spawn(); world.add_component(e1, TestComp(1));
        let e2 = world.spawn(); world.add_component(e2, TestComp(2));
        let e3 = world.spawn(); world.add_component(e3, TestComp(3));
        
        // e2'yi despawn et — e3 onun yerine swap_remove ile gelir
        world.despawn(e2);
        
        // e3 hâlâ erişilebilir olmalı
        let comps = world.borrow::<TestComp>();
        let val = comps.get(e3.id()).unwrap();
        assert_eq!(val.0, 3);
    }

    #[test]
    fn add_component_migrates_archetype() {
        #[derive(Clone, Debug, PartialEq)]
        struct TestCompI32(i32);
        impl crate::component::Component for TestCompI32 {}

        #[derive(Clone, Debug, PartialEq)]
        struct TestCompF32(f32);
        impl crate::component::Component for TestCompF32 {}

        let mut world = World::new();
        world.register_component_type::<TestCompI32>();
        world.register_component_type::<TestCompF32>();
        
        let e = world.spawn();
        world.add_component(e, TestCompI32(10));
        
        let loc1 = world.entity_location(e.id());
        
        world.add_component(e, TestCompF32(2.5));
        
        let loc2 = world.entity_location(e.id());
        assert_ne!(loc1.archetype_id, loc2.archetype_id);
        
        assert_eq!(world.borrow::<TestCompI32>().get(e.id()).unwrap().0, 10);
        assert_eq!(world.borrow::<TestCompF32>().get(e.id()).unwrap().0, 2.5);
    }

    #[test]
    fn spawn_batch_keeps_columns_and_entities_consistent() {
        #[derive(Clone, Debug, PartialEq)]
        struct BatchI(i32);
        impl crate::component::Component for BatchI {}
        #[derive(Clone, Debug, PartialEq)]
        struct BatchF(f32);
        impl crate::component::Component for BatchF {}

        let mut world = World::new();
        world.register_component_type::<BatchI>();
        world.register_component_type::<BatchF>();

        let n = 100usize;
        let bundles = (0..n).map(|i| (BatchI(i as i32), BatchF(i as f32 * 1.5)));
        let ents: Vec<_> = world.spawn_batch(bundles).collect();
        assert_eq!(ents.len(), n);

        // Her entity'nin iki bileşeni de doğru olmalı (column/entities desync veya OOB yok).
        let bi = world.borrow::<BatchI>();
        let bf = world.borrow::<BatchF>();
        for (i, e) in ents.iter().enumerate() {
            assert_eq!(bi.get(e.id()).map(|c| c.0), Some(i as i32), "BatchI[{i}]");
            assert_eq!(bf.get(e.id()).map(|c| c.0), Some(i as f32 * 1.5), "BatchF[{i}]");
        }
        // Query iterasyonu tam n eleman vermeli (her sütun uzunluğu == entities sayısı).
        assert_eq!(bi.iter().count(), n, "column/entities tutarsızlığı");
        assert_eq!(bf.iter().count(), n, "column/entities tutarsızlığı");
    }

    /// A command queued before a clear must not be delivered to an unrelated entity after it.
    ///
    /// This is sharper than an ordinary use-after-despawn. `World::despawn` bumps the slot's
    /// generation, so a stale handle stays dead and the command misses harmlessly; `Entities::clear`
    /// resets the generations *and* the id counter, so the first entity spawned afterwards is
    /// `Entity(0, gen 0)` — the same 64 bits the queued command captured. It does not miss. It
    /// hits a stranger.
    #[test]
    fn a_command_queued_before_a_clear_is_not_delivered_to_the_id_that_replaces_it() {
        #[derive(Clone, Debug, PartialEq)]
        struct Marker(u32);
        impl crate::component::Component for Marker {}

        let mut world = World::new();
        world.register_component_type::<Marker>();

        let doomed = world.spawn();
        {
            let queue = world
                .get_resource::<crate::commands::CommandQueue>()
                .expect("World::new installs a CommandQueue");
            queue.push(move |w: &mut World| {
                w.add_component(doomed, Marker(1));
            });
        }

        world.clear_entities();

        let stranger = world.spawn();
        // The premise, asserted rather than assumed: the id really does come back identical.
        assert_eq!(stranger, doomed, "a clear resets generations, so the handle is reused exactly");

        world.apply_commands();

        assert!(
            world.query_entity::<&Marker>(stranger.id()).is_none(),
            "a command queued for an entity destroyed by clear_entities was delivered to its \
             bit-identical successor"
        );
    }

    /// Compacting a set that has become EMPTY takes `BlobVec::shrink_to_fit`'s other branch: at
    /// `len == 0` it deallocates outright and leaves a dangling pointer behind, which nothing
    /// else in the suite reaches. The set must survive that and still accept an insert
    /// afterwards — a dangling `BlobVec` that cannot be pushed to again would turn a memory
    /// reclamation into a component type that silently stops working after a GC tick.
    #[test]
    fn compacting_an_emptied_sparse_set_leaves_it_usable() {
        #[derive(Clone, Debug, PartialEq)]
        struct SparseE(String);
        impl crate::component::Component for SparseE {
            fn storage_type() -> crate::component::StorageType {
                crate::component::StorageType::SparseSet
            }
        }

        let mut world = World::new();
        world.register_component_type::<SparseE>();

        // A heap-owning component on purpose: the dealloc branch runs the type's drop glue, and
        // a leak or a double free here is what a ZST payload would hide.
        let a = world.spawn();
        world.add_component(a, SparseE("first".into()));
        world.remove_component::<SparseE>(a);

        world.compact();

        {
            let set = &world.sparse_sets[&std::any::TypeId::of::<SparseE>()];
            assert_eq!(set.sparse.len(), 0, "an emptied index truncates to nothing");
            assert_eq!(set.dense.len(), 0);
            assert_eq!(set.dense.capacity, 0, "and the blob gave its allocation back");
        }

        // The set is kept rather than dropped, so this must be an ordinary insert into the same
        // set — not a re-registration — and it must read back.
        let b = world.spawn();
        world.add_component(b, SparseE("second".into()));
        assert_eq!(
            world.query_entity::<&SparseE>(b.id()).map(|c| c.0.clone()),
            Some("second".to_string()),
            "a compacted-empty set must still accept and return a value"
        );
    }

    /// `clear_entities` reached from INSIDE a queued command cancels the rest of that same
    /// flush, because `CommandQueue::apply` pops from the queue the clear just emptied. That is
    /// a consequence of the drain, not a bug in it — a command that tore the world down has no
    /// business having its siblings run against the wreckage — but it is invisible from the call
    /// site, so it is pinned here rather than left to be discovered.
    #[test]
    fn clear_entities_from_inside_a_flush_cancels_the_commands_behind_it() {
        #[derive(Clone, Debug, PartialEq)]
        struct Late(u32);
        impl crate::component::Component for Late {}

        let mut world = World::new();
        world.register_component_type::<Late>();
        let survivor = world.spawn();

        {
            let queue = world
                .get_resource::<crate::commands::CommandQueue>()
                .expect("World::new installs a CommandQueue");
            queue.push(move |w: &mut World| {
                w.clear_entities();
            });
            // Queued AFTER the clear command, so `apply` would reach it next.
            queue.push(move |w: &mut World| {
                w.add_component(survivor, Late(1));
            });
        }

        world.apply_commands();

        let fresh = world.spawn();
        assert_eq!(fresh, survivor, "ids restart, so this is the same handle the second command holds");
        assert!(
            world.query_entity::<&Late>(fresh.id()).is_none(),
            "the command queued behind the clear must not have run — and must certainly not \
             have landed on the id that replaced its target"
        );
    }

    /// The destructive variant, and the one `ENGINE.md` actually names: a queued `despawn`.
    /// The `insert` case above leaves a stranger holding a component nobody asked for; this one
    /// deletes the stranger outright. Both come from the same queue, so one guard covers both —
    /// but they fail so differently that a reader would not predict the second from the first.
    #[test]
    fn a_queued_despawn_from_before_a_clear_does_not_kill_the_id_that_replaces_it() {
        let mut world = World::new();

        let doomed = world.spawn();
        {
            let queue = world
                .get_resource::<crate::commands::CommandQueue>()
                .expect("World::new installs a CommandQueue");
            queue.push(move |w: &mut World| {
                w.despawn(doomed);
            });
        }

        world.clear_entities();

        let stranger = world.spawn();
        assert_eq!(stranger, doomed, "a clear resets generations, so the handle is reused exactly");

        world.apply_commands();

        assert!(
            world.is_alive(stranger),
            "a despawn queued for an entity destroyed by clear_entities killed its \
             bit-identical successor"
        );
    }

    /// The counterpart, so the fix is "discard at the clear" and not "the queue never runs":
    /// a command queued *after* the clear must still be applied.
    #[test]
    fn a_command_queued_after_a_clear_still_applies() {
        #[derive(Clone, Debug, PartialEq)]
        struct Kept(u32);
        impl crate::component::Component for Kept {}

        let mut world = World::new();
        world.register_component_type::<Kept>();
        world.spawn();
        world.clear_entities();

        let e = world.spawn();
        {
            let queue = world
                .get_resource::<crate::commands::CommandQueue>()
                .expect("World::new installs a CommandQueue");
            queue.push(move |w: &mut World| {
                w.add_component(e, Kept(7));
            });
        }
        world.apply_commands();

        assert_eq!(
            world.query_entity::<&Kept>(e.id()).map(|c| c.0),
            Some(7),
            "clearing the queue at teardown must not disable it afterwards"
        );
    }

    /// `remove_component` on an entity that was RESERVED but never flushed must be a no-op, not
    /// a panic.
    ///
    /// `Commands::spawn` hands out an id before its queued `flush_spawn` runs, so between those
    /// two moments the entity is `is_alive` yet owns no archetype row and has no slot in
    /// `entity_locations`. `remove_component` read that slot *before* the lookup that would have
    /// told it so, and indexed out of bounds. `add_component` and `add_bundle` both order those
    /// two steps the other way round — `add_bundle` even carries a comment explaining this exact
    /// entity — which is what made the asymmetry findable.
    #[test]
    fn removing_a_component_from_a_reserved_but_unflushed_entity_is_a_no_op() {
        #[derive(Clone, Debug, PartialEq)]
        struct Tbl(u32);
        impl crate::component::Component for Tbl {}

        let mut world = World::new();
        world.register_component_type::<Tbl>();
        // Give the world one flushed entity first, so `entity_locations` is non-empty and the
        // failure is an out-of-range index rather than an empty-vector one.
        world.spawn();

        let reserved = {
            let entities = world
                .get_resource::<crate::entity::allocator::Entities>()
                .expect("World::new installs an Entities allocator");
            entities.reserve_entity()
        };
        assert!(
            world.is_alive(reserved),
            "a reserved id is alive — which is why the is_alive check at the top does not catch it"
        );

        // Reaching the next line at all is the assertion.
        world.remove_component::<Tbl>(reserved);

        assert!(
            world.query_entity::<&Tbl>(reserved.id()).is_none(),
            "and nothing was invented for it"
        );
    }

    /// The same defect in the batch APIs. Both grouping loops read the location raw between
    /// `is_alive` — which a reserved id passes — and `is_valid()`, which would have rejected it;
    /// the raw index is the step in between, so the check that was supposed to catch this ran one
    /// line too late. A batch is the likelier way to meet it in practice: the caller collects a
    /// slice of entities from somewhere and hands the whole thing over, so it takes only one
    /// unflushed member to take the call down.
    #[test]
    fn the_batch_apis_skip_a_reserved_but_unflushed_entity_instead_of_panicking() {
        #[derive(Clone, Debug, PartialEq)]
        struct Tag(u32);
        impl crate::component::Component for Tag {}

        let mut world = World::new();
        world.register_component_type::<Tag>();

        let real = world.spawn_bundle(Tag(1));
        let reserved = {
            let entities = world
                .get_resource::<crate::entity::allocator::Entities>()
                .expect("World::new installs an Entities allocator");
            entities.reserve_entity()
        };

        // A mixed batch — one flushed entity, one reserved-but-unflushed.
        let batch = [real, reserved];
        world.insert_batch(&batch, Tag(7));
        world.remove_batch::<Tag>(&batch);

        // Reaching this line is the assertion; the rest is evidence the real entity was still
        // processed rather than the whole call bailing out.
        assert!(
            world.query_entity::<&Tag>(real.id()).is_none(),
            "the flushed entity's component was removed by the batch"
        );
        assert!(world.is_alive(reserved), "and the reserved id is untouched");
    }

    /// A hook that tears the world down mid-despawn must not take the despawn with it.
    ///
    /// `run_hooks` hands each `on_remove` hook a `&mut World`, and `hooks.rs` documents them as
    /// free to spawn, despawn and mutate. `World::clear_entities` empties `entity_locations`
    /// outright, so the re-fetch `despawn` performs *after* its hooks — the one whose whole
    /// reason for existing is that state may have changed — indexed a vector that was no longer
    /// there. The entry read is bounds-checked and says so in a comment; the re-fetch said
    /// "safely" and was not.
    #[test]
    fn an_on_remove_hook_that_clears_the_world_does_not_crash_the_despawn() {
        #[derive(Clone, Debug, PartialEq)]
        struct Doomed(u32);
        impl crate::component::Component for Doomed {}

        let mut world = World::new();
        world.register_component_type::<Doomed>();
        world.register_on_remove::<Doomed>(Box::new(|w: &mut World, _e: Entity| {
            w.clear_entities();
        }));

        let e = world.spawn_bundle(Doomed(1));
        // Reaching the line after this is the assertion.
        world.despawn(e);

        // And the world is consistent afterwards: the clear ran, so ids restart.
        let fresh = world.spawn();
        assert_eq!(fresh.id(), 0, "the hook's clear took effect");
        assert!(world.is_alive(fresh));
    }

    /// `spawn_batch` spawns its first bundle normally to discover the archetype the rest are
    /// appended into. That first spawn fires `on_add`/`on_set`, and a hook may despawn the
    /// entity it was just handed — leaving the batch with no archetype at all.
    ///
    /// The location then reads `INVALID`, whose `archetype_id` is `u32::MAX`, and the append
    /// loop indexed the archetype vector with it. So this is not a truncation hazard waiting to
    /// happen; it is a live "index out of bounds" for any batch of two or more, and a one-element
    /// batch hides it because the loop body never runs.
    #[test]
    fn spawn_batch_survives_a_hook_that_despawns_its_first_entity() {
        #[derive(Clone, Debug, PartialEq)]
        struct Suicidal(u32);
        impl crate::component::Component for Suicidal {}

        let mut world = World::new();
        world.register_component_type::<Suicidal>();
        world.register_on_add::<Suicidal>(Box::new(|w: &mut World, e: Entity| {
            w.despawn(e);
        }));

        // Three bundles, so the append loop below the first spawn really runs.
        let spawned: Vec<Entity> = world
            .spawn_batch((0..3).map(Suicidal))
            .collect();

        // Reaching this line at all is the assertion.
        assert_eq!(spawned.len(), 3, "every bundle is still accounted for");
        // The hook killed each of them, which is what the caller asked for — the point is that
        // it did so without taking the batch down.
        for e in spawned {
            assert!(!world.is_alive(e), "the hook despawned it, as written");
        }
    }

    /// Every entity listed in an archetype's row must have a location naming that archetype and
    /// that row. Nothing enforces it; this checks it.
    fn assert_locations_agree_with_archetypes(world: &World, context: &str) {
        for (arch_idx, arch) in world.archetype_index.archetypes.iter().enumerate() {
            for (row, &id) in arch.entities().iter().enumerate() {
                let loc = world.entity_location(id);
                assert!(
                    loc.is_valid(),
                    "{context}: entity {id} sits at archetype {arch_idx} row {row} but its \
                     location is INVALID — it is a ghost, present in the archetype and \
                     unreachable through the location table"
                );
                assert_eq!(
                    (loc.archetype_id as usize, loc.row as usize),
                    (arch_idx, row),
                    "{context}: entity {id} sits at archetype {arch_idx} row {row} but its \
                     location says archetype {} row {}",
                    loc.archetype_id,
                    loc.row
                );
            }
        }
    }

    /// A duplicated entity in a batch slice must not corrupt the world.
    ///
    /// The grouping loop does not deduplicate, so one entity named twice in the caller's slice
    /// lands in its group twice. The migration loop then re-reads its location — which the first
    /// pass has just moved to the TARGET archetype — and hands that row to `move_entity_to` on
    /// the SOURCE archetype, which is a different archetype entirely. So it drags whichever
    /// entity now occupies that source row into the target and records the new row under the
    /// duplicated entity's id, leaving the dragged one listed in an archetype it does not
    /// believe it is in.
    ///
    /// Nothing in `insert_batch`'s documentation forbids duplicates, and a caller who collects
    /// entities from two overlapping sources has no reason to expect them to be forbidden.
    #[test]
    fn a_duplicated_entity_in_an_insert_batch_does_not_corrupt_the_world() {
        #[derive(Clone, Debug, PartialEq)]
        struct A(u32);
        impl crate::component::Component for A {}
        #[derive(Clone, Debug, PartialEq)]
        struct B(u32);
        impl crate::component::Component for B {}

        let mut world = World::new();
        world.register_component_type::<A>();
        world.register_component_type::<B>();

        let ents: Vec<Entity> = (0..4).map(|i| world.spawn_bundle(A(i))).collect();
        assert_locations_agree_with_archetypes(&world, "before");

        world.insert_batch(&[ents[0], ents[0]], B(9));

        assert_locations_agree_with_archetypes(&world, "after a duplicated insert_batch");
        // And the component actually arrived, exactly once, on exactly that entity.
        assert_eq!(world.query_entity::<&B>(ents[0].id()).map(|c| c.0), Some(9));
        assert_eq!(world.query::<&B>().unwrap().iter().count(), 1);
    }

    /// The same shape through `remove_batch`, whose grouping loop is the same code.
    #[test]
    fn a_duplicated_entity_in_a_remove_batch_does_not_corrupt_the_world() {
        #[derive(Clone, Debug, PartialEq)]
        struct A2(u32);
        impl crate::component::Component for A2 {}
        #[derive(Clone, Debug, PartialEq)]
        struct B2(u32);
        impl crate::component::Component for B2 {}

        let mut world = World::new();
        world.register_component_type::<A2>();
        world.register_component_type::<B2>();

        let ents: Vec<Entity> = (0..4).map(|i| world.spawn_bundle((A2(i), B2(i)))).collect();
        assert_locations_agree_with_archetypes(&world, "before");

        world.remove_batch::<B2>(&[ents[0], ents[0]]);

        assert_locations_agree_with_archetypes(&world, "after a duplicated remove_batch");
        assert!(world.query_entity::<&B2>(ents[0].id()).is_none());
        assert_eq!(world.query::<&B2>().unwrap().iter().count(), 3);
    }

    /// `spawn_batch` discovers the batch's archetype from its first entity, and hooks run
    /// between the spawn and the read. A hook that MOVES that entity to a different archetype —
    /// by removing the component it was just given, say — leaves the discovered archetype
    /// pointing somewhere whose columns do not match the bundle at all.
    ///
    /// The guard added for the despawn case only asks whether the location is valid. This one
    /// is valid; it is just wrong.
    #[test]
    fn spawn_batch_survives_a_hook_that_moves_its_first_entity_to_another_archetype() {
        #[derive(Clone, Debug, PartialEq)]
        struct Tag(u32);
        impl crate::component::Component for Tag {}

        let mut world = World::new();
        world.register_component_type::<Tag>();
        // The hook takes the component straight back off, which migrates the entity out of the
        // archetype `spawn_bundle` had just put it in.
        world.register_on_add::<Tag>(Box::new(|w: &mut World, e: Entity| {
            w.remove_component::<Tag>(e);
        }));

        let spawned: Vec<Entity> = world.spawn_batch((0..3).map(Tag)).collect();
        assert_eq!(spawned.len(), 3);

        // Whatever the hook did to each entity, the world must still agree with itself: every
        // entity listed in an archetype has a location naming that archetype and that row.
        for (arch_idx, arch) in world.archetype_index.archetypes.iter().enumerate() {
            for (row, &id) in arch.entities().iter().enumerate() {
                let loc = world.entity_location(id);
                assert!(
                    loc.is_valid()
                        && loc.archetype_id as usize == arch_idx
                        && loc.row as usize == row,
                    "entity {id} is listed at archetype {arch_idx} row {row} but its location \
                     says archetype {} row {}",
                    loc.archetype_id,
                    loc.row
                );
            }
        }
    }

    /// Asserts INV: every entity listed in an archetype row has a location naming that row.
    fn assert_inv(world: &World, context: &str) {
        for (arch_idx, arch) in world.archetype_index.archetypes.iter().enumerate() {
            let mut seen = std::collections::HashSet::new();
            for (row, &id) in arch.entities().iter().enumerate() {
                assert!(
                    seen.insert(id),
                    "{context}: entity {id} is listed TWICE in archetype {arch_idx}"
                );
                let loc = world.entity_location(id);
                assert!(
                    loc.is_valid()
                        && loc.archetype_id as usize == arch_idx
                        && loc.row as usize == row,
                    "{context}: entity {id} at archetype {arch_idx} row {row}, location says \
                     archetype {} row {}",
                    loc.archetype_id,
                    loc.row
                );
            }
        }
    }

    /// A sparse `on_remove` hook runs after the entity's archetype row has already been
    /// swap-removed. Until the location was cleared first, the hook saw a location still naming
    /// that row — which by then belonged to whoever had been last in the archetype — and
    /// anything routing through it acted on a stranger.
    ///
    /// `add_component` is the shortest way there: it reads the location, hands the row to
    /// `move_entity_to`, and that function moves whoever is *in* the row rather than whoever the
    /// caller meant.
    #[test]
    fn a_sparse_on_remove_hook_does_not_see_a_location_pointing_at_someone_elses_row() {
        #[derive(Clone, Debug, PartialEq)]
        struct SparseTag(u32);
        impl crate::component::Component for SparseTag {
            fn storage_type() -> crate::component::StorageType {
                crate::component::StorageType::SparseSet
            }
        }
        #[derive(Clone, Debug, PartialEq)]
        struct Late(u32);
        impl crate::component::Component for Late {}

        let mut world = World::new();
        world.register_component_type::<SparseTag>();
        world.register_component_type::<Late>();
        world.register_on_remove::<SparseTag>(Box::new(|w: &mut World, e: Entity| {
            // The entity is mid-despawn: no row, no location. This must be a no-op rather than
            // an action on the row that used to be its.
            w.add_component(e, Late(1));
        }));

        // Several entities in one archetype so the victim's row is NOT the last one — that is
        // what makes the swap-remove move a different entity into it.
        let a = world.spawn_bundle(Late(10));
        let victim = world.spawn_bundle(Late(11));
        let tail = world.spawn_bundle(Late(12));
        world.add_component(victim, SparseTag(1));

        world.despawn(victim);

        assert_inv(&world, "after a despawn whose sparse on_remove hook touched the world");
        // The survivors keep their own values: nothing was dragged anywhere.
        assert_eq!(world.query_entity::<&Late>(a.id()).map(|c| c.0), Some(10));
        assert_eq!(world.query_entity::<&Late>(tail.id()).map(|c| c.0), Some(12));
        assert!(!world.is_alive(victim));
    }

    /// The other half of the same window: a hook that puts the entity BACK. It cannot be
    /// allowed — the row it adds is orphaned the moment the despawn finishes clearing the
    /// location — and `flush_spawn`'s own liveness guard does not catch it, because the id is
    /// not freed until after the hooks. A debug build names it at the despawn.
    #[test]
    #[should_panic(expected = "re-listed entity")]
    fn an_on_remove_hook_that_re_lists_the_entity_is_reported() {
        #[derive(Clone, Debug, PartialEq)]
        struct SparseBack(u32);
        impl crate::component::Component for SparseBack {
            fn storage_type() -> crate::component::StorageType {
                crate::component::StorageType::SparseSet
            }
        }

        let mut world = World::new();
        world.register_component_type::<SparseBack>();
        world.register_on_remove::<SparseBack>(Box::new(|w: &mut World, e: Entity| {
            w.flush_spawn(e);
        }));

        let e = world.spawn();
        world.add_component(e, SparseBack(1));
        world.despawn(e);
    }

    /// A component whose `Drop` panics must not leave the world describing an entity two ways.
    ///
    /// `add_bundle`'s migration branch calls two pieces of user code AFTER the entity has been
    /// moved into the target archetype and BEFORE its location is updated: the `Drop` of every
    /// component that came across, and `Bundle::write_to_archetype`. A panic from either unwinds
    /// past the location write, leaving the entity listed in the target while its location still
    /// names the source — and a row there that no longer exists.
    ///
    /// Nothing in the crate promises panic safety for a component's `Drop`, `catch_unwind` is
    /// safe std, and no assertion fires along the way, so this was reachable without `unsafe` and
    /// without breaking any documented contract. The location is written before the user code
    /// now, so an unwind leaves the world merely missing a value rather than lying about where
    /// an entity is.
    #[test]
    fn a_panicking_component_drop_during_a_migration_does_not_strand_the_entity() {
        // The panic is ARMED for exactly one drop. A component whose drop panics leaves its
        // column slot half-dropped, so the same value is dropped a second time when the world
        // is torn down at the end of the test — and a second panic there is a double panic, not
        // the thing under test. (That half-dropped slot is its own question and is not this
        // test's; see `docs/ENGINE.md` §3.)
        static ARMED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

        #[derive(Clone, Debug)]
        struct Loud(&'static str);
        impl crate::component::Component for Loud {}
        impl Drop for Loud {
            fn drop(&mut self) {
                if self.0 == "BOOM"
                    && ARMED.swap(false, std::sync::atomic::Ordering::SeqCst)
                {
                    panic!("Loud::drop");
                }
            }
        }
        #[derive(Clone, Debug)]
        struct Tag;
        impl crate::component::Component for Tag {}

        let mut world = World::new();
        let neighbour = world.spawn();
        world.add_bundle(neighbour, (Loud("n"),));
        let victim = world.spawn();
        world.add_bundle(victim, (Loud("BOOM"),));
        assert_inv(&world, "before");

        // The migration {Loud} -> {Loud, Tag} drops the carried-across `Loud("BOOM")` to make
        // room for the bundle's own value, and that drop panics.
        ARMED.store(true, std::sync::atomic::Ordering::SeqCst);
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            world.add_bundle(victim, (Loud("x"), Tag));
        }));
        assert!(unwound.is_err(), "the drop was supposed to panic; the fixture is wrong if not");

        // Reaching this line at all is most of the assertion — before the fix the world was
        // internally inconsistent and the next structural touch read a row that was gone.
        assert_inv(&world, "after an unwind out of the middle of a migration");
        assert!(world.is_alive(neighbour) && world.is_alive(victim));
    }

    /// A deferred spawn whose entity is despawned before the queue is flushed.
    ///
    /// `Commands::spawn` reserves the id now and queues a `flush_spawn` for later. Despawning
    /// that handle in between is ordinary — it is a handle, it is alive, `despawn` accepts it —
    /// and it frees the id. The queued `flush_spawn` then runs anyway: it consults neither the
    /// allocator nor the existing location, so it appends a row for a dead id.
    ///
    /// Nothing here breaks `flush_spawn`'s documented contract, which is "call it exactly once
    /// per reserved id". It is called exactly once.
    #[test]
    fn a_deferred_spawn_despawned_before_its_flush_leaves_no_ghost() {
        let mut world = World::new();
        // Exactly what `Commands::spawn` does: reserve the id now, queue the flush for later.
        let e = {
            let entities = world
                .get_resource::<crate::entity::allocator::Entities>()
                .expect("Entities");
            let e = entities.reserve_entity();
            drop(entities);
            let queue = world
                .get_resource::<crate::commands::CommandQueue>()
                .expect("CommandQueue");
            queue.push(move |w: &mut World| w.flush_spawn(e));
            e
        };
        world.despawn(e);
        world.apply_commands();
        assert_inv(&world, "after flushing a deferred spawn that was despawned first");

        // …and the id is now free, so the next spawn takes it back. `flush_spawn` appended a row
        // for it a moment ago without consulting the allocator, so the archetype can end up
        // listing one id twice with only the second row recorded.
        let reused = world.spawn();
        assert_inv(&world, "after the freed id was recycled by a later spawn");
        assert!(world.is_alive(reused));
        assert_eq!(
            world.query::<&crate::component::EntityName>().map(|q| q.iter().count()),
            Some(0),
            "sanity: nothing invented a component"
        );
    }

    /// A sparse set's reverse index is sized by the largest entity id ever inserted, not by the
    /// number of entries, and nothing but `clear_entities` used to give that back. `compact`
    /// existed to return RAM "towards the initial defragmented state" and walked straight past
    /// one of the two largest allocations in the world — the other being `entity_locations`,
    /// which is the same shape at twice the bytes per id and is not truncated (see `compact`).
    ///
    /// The assertion is on `len()`, not `capacity()`: `Vec::shrink_to_fit` is allowed to keep
    /// more than it needs, so an exact capacity is not a promise the standard library makes.
    /// What IS exact is that every trailing entry was the absent-sentinel and is gone.
    #[test]
    fn compact_reclaims_a_sparse_sets_reverse_index() {
        #[derive(Clone, Debug, PartialEq)]
        struct SparseC(i32);
        impl crate::component::Component for SparseC {
            fn storage_type() -> crate::component::StorageType {
                crate::component::StorageType::SparseSet
            }
        }

        let mut world = World::new();
        world.register_component_type::<SparseC>();

        const N: usize = 2_000;
        let ents: Vec<_> = (0..N)
            .map(|i| {
                let e = world.spawn();
                world.add_component(e, SparseC(i as i32));
                e
            })
            .collect();
        // Everything except the first entity goes. The survivor is the LOWEST id on purpose:
        // it is the shape that leaves the whole index behind describing one entry.
        for &e in &ents[1..] {
            world.despawn(e);
        }

        let set_id = std::any::TypeId::of::<SparseC>();
        let before = world.sparse_sets[&set_id].sparse.len();
        assert_eq!(before, N, "the index is sized by the largest id inserted");

        world.compact();

        let set = &world.sparse_sets[&set_id];
        assert_eq!(set.sparse.len(), 1, "trailing absent entries are pure absence and must go");
        assert!(set.sparse.capacity() < before, "and the allocation must actually shrink");
        // The other three arrays hold one row each and always did — `len` alone would stay green
        // if their `shrink_to_fit` calls were deleted, because it was never the length that was
        // wasted there but the capacity a million pushes doubled their way to.
        assert_eq!(set.dense.len(), 1);
        assert_eq!(set.entities.len(), 1);
        assert_eq!(set.ticks.len(), 1);
        assert!(set.entities.capacity() < before, "entities kept its grown capacity");
        assert!(set.ticks.capacity() < before, "ticks kept its grown capacity");
        assert!(set.dense.capacity < before, "dense kept its grown capacity");

        // The survivor is still reachable — a truncation that went one entry too far would
        // make this `None` (or, on the unchecked query path, panic).
        assert_eq!(
            world.query_entity::<&SparseC>(ents[0].id()).map(|c| c.0),
            Some(0),
            "compaction must not lose the entry it kept"
        );
    }

    /// After compaction, a **live entity id can be larger than `sparse.len()`** — a world state
    /// that was structurally impossible before, because the index only ever grew. Every query
    /// path has to keep reading that as "absent" rather than panicking or, worse, matching.
    ///
    /// The two tests around it prove the index shrinks and that it does not shrink too far.
    /// Neither visits an id past the new end, which is precisely the state the shrinking creates.
    #[test]
    fn a_live_id_past_the_compacted_index_reads_as_absent_on_every_query_path() {
        #[derive(Clone, Debug, PartialEq)]
        struct SparseP(i32);
        impl crate::component::Component for SparseP {
            fn storage_type() -> crate::component::StorageType {
                crate::component::StorageType::SparseSet
            }
        }
        #[derive(Clone, Debug, PartialEq)]
        struct Tableish(i32);
        impl crate::component::Component for Tableish {}

        let mut world = World::new();
        world.register_component_type::<SparseP>();
        world.register_component_type::<Tableish>();

        const N: usize = 512;
        let ents: Vec<_> = (0..N).map(|_| world.spawn()).collect();
        // The low id keeps the set non-empty; the high id pushes `sparse` out to N and then
        // gives its entry back — so after compaction the index is 1 long while ids 1..N-1 are
        // all still ALIVE and all past its end.
        world.add_component(ents[0], SparseP(0));
        world.add_component(ents[N - 1], SparseP(1));
        world.remove_component::<SparseP>(ents[N - 1]);
        // …and a table component, so a query has a reason to visit that high id at all.
        world.add_component(ents[N - 1], Tableish(9));

        world.compact();

        assert_eq!(
            world.sparse_sets[&std::any::TypeId::of::<SparseP>()].sparse.len(),
            1,
            "the index is now SHORTER than the live id range — the state under test"
        );

        // A bare read must skip the out-of-range ids rather than index them.
        assert_eq!(
            world.query::<&SparseP>().unwrap().iter().count(),
            1,
            "only the entity that still has the component"
        );
        assert!(world.query_entity::<&SparseP>(ents[N - 1].id()).is_none());
        // The high entity is alive and keeps its table component: sparse compaction must not
        // have reached anything but the sparse side.
        assert_eq!(
            world.query_entity::<&Tableish>(ents[N - 1].id()).map(|c| c.0),
            Some(9)
        );
        assert_eq!(world.query::<&Tableish>().unwrap().iter().count(), 1);
    }

    /// The other direction: a surviving HIGH id pins the index, and compaction must not shorten
    /// past it. Without this, "truncate to the last live entry" and "truncate to the number of
    /// live entries" — which differ by everything — both pass the test above.
    #[test]
    fn compact_keeps_the_reverse_index_long_enough_for_a_high_surviving_id() {
        #[derive(Clone, Debug, PartialEq)]
        struct SparseH(i32);
        impl crate::component::Component for SparseH {
            fn storage_type() -> crate::component::StorageType {
                crate::component::StorageType::SparseSet
            }
        }

        let mut world = World::new();
        world.register_component_type::<SparseH>();

        const N: usize = 512;
        let ents: Vec<_> = (0..N)
            .map(|i| {
                let e = world.spawn();
                world.add_component(e, SparseH(i as i32));
                e
            })
            .collect();
        // This time the LAST id survives.
        for &e in &ents[..N - 1] {
            world.despawn(e);
        }

        world.compact();

        let set = &world.sparse_sets[&std::any::TypeId::of::<SparseH>()];
        assert_eq!(
            set.sparse.len(),
            N,
            "the surviving entry sits at the far end; the index has to reach it"
        );
        assert_eq!(set.dense.len(), 1, "…while the dense side holds exactly one row");
        assert_eq!(
            world.query_entity::<&SparseH>(ents[N - 1].id()).map(|c| c.0),
            Some(N as i32 - 1),
        );
    }

    // Regression: spawn_batch's fast path wrote every bundle straight into
    // archetype columns, but SparseSet components have no column — so the 2nd+
    // entity panicked ("Component column missing in Archetype"). A bundle with a
    // sparse component must now route every entity's sparse component into the
    // sparse set (spawn_batch falls back to per-entity spawn_bundle).
    #[test]
    fn spawn_batch_routes_sparse_components() {
        #[derive(Clone, Debug, PartialEq)]
        struct TableC(i32);
        impl crate::component::Component for TableC {}
        #[derive(Clone, Debug, PartialEq)]
        struct SparseC(i32);
        impl crate::component::Component for SparseC {
            fn storage_type() -> crate::component::StorageType {
                crate::component::StorageType::SparseSet
            }
        }

        let mut world = World::new();
        world.register_component_type::<TableC>();
        world.register_component_type::<SparseC>();

        let n = 50usize;
        let bundles = (0..n).map(|i| (TableC(i as i32), SparseC(i as i32 * 2)));
        let ents: Vec<_> = world.spawn_batch(bundles).collect();
        assert_eq!(ents.len(), n);

        // Every entity must have BOTH the table and the sparse component, and the
        // sparse one must carry the right value (routed, not lost/panicked).
        let mut query = world.query_mut::<(&TableC, &SparseC)>().unwrap();
        let mut count = 0;
        for (_id, (t, s)) in query.iter_mut() {
            assert_eq!(s.0, t.0 * 2, "sparse component value mismatch");
            count += 1;
        }
        assert_eq!(count, n, "all entities must have both components");
    }

    // Regression: add_bundle built the archetype signature from ALL component
    // types (including SparseSet ones) and wrote them all into archetype columns,
    // so a sparse component in the bundle was silently stored as a table column
    // instead of in `sparse_sets` — invisible to sparse-storage queries.
    #[test]
    fn add_bundle_routes_sparse_components() {
        #[derive(Clone, Debug, PartialEq)]
        struct TableC(i32);
        impl crate::component::Component for TableC {}
        #[derive(Clone, Debug, PartialEq)]
        struct SparseC(i32);
        impl crate::component::Component for SparseC {
            fn storage_type() -> crate::component::StorageType {
                crate::component::StorageType::SparseSet
            }
        }

        let mut world = World::new();
        world.register_component_type::<TableC>();
        world.register_component_type::<SparseC>();

        let e = world.spawn();
        world.add_bundle(e, (TableC(7), SparseC(9)));

        // The sparse component must be reachable through a sparse-storage query.
        let mut query = world.query_mut::<(&TableC, &SparseC)>().unwrap();
        let mut found = None;
        for (_id, (t, s)) in query.iter_mut() {
            found = Some((t.0, s.0));
        }
        assert_eq!(found, Some((7, 9)), "add_bundle must route the sparse component");
    }

    // Regression: remove_bundle only rearranged archetype (table) columns; a
    // SparseSet component in the bundle was never removed from `sparse_sets`, so
    // it leaked (stayed queryable after removal).
    #[test]
    fn remove_bundle_removes_sparse_components() {
        #[derive(Clone, Debug, PartialEq)]
        struct TableC(i32);
        impl crate::component::Component for TableC {}
        #[derive(Clone, Debug, PartialEq)]
        struct SparseC(i32);
        impl crate::component::Component for SparseC {
            fn storage_type() -> crate::component::StorageType {
                crate::component::StorageType::SparseSet
            }
        }

        let mut world = World::new();
        world.register_component_type::<TableC>();
        world.register_component_type::<SparseC>();

        let e = world.spawn();
        world.add_component(e, TableC(1));
        world.add_component(e, SparseC(2)); // correctly in the sparse set
        world.remove_bundle::<(TableC, SparseC)>(e);

        let query = world.query::<&SparseC>().unwrap();
        assert_eq!(
            query.iter().count(),
            0,
            "remove_bundle must also remove the sparse component from its set"
        );
    }

    // Regression: despawn swap-removed the entity from its archetype but never
    // touched `sparse_sets` — so a SparseSet component leaked and, because the set
    // is keyed by raw entity id, a REUSED id inherited the dead entity's stale
    // component.
    #[test]
    fn despawn_clears_sparse_components() {
        #[derive(Clone, Debug, PartialEq)]
        struct SparseC(i32);
        impl crate::component::Component for SparseC {
            fn storage_type() -> crate::component::StorageType {
                crate::component::StorageType::SparseSet
            }
        }

        let mut world = World::new();
        world.register_component_type::<SparseC>();

        let e = world.spawn();
        world.add_component(e, SparseC(5));
        world.despawn(e);

        // No SparseC must survive the despawn (leak)...
        assert_eq!(
            world.query::<&SparseC>().unwrap().iter().count(),
            0,
            "despawn leaked a sparse component"
        );
        // ...and a reused id must not inherit it (stale data).
        let e2 = world.spawn();
        assert!(
            world.query_entity::<&SparseC>(e2.id()).is_none(),
            "reused entity id inherited a stale sparse component from the despawned entity"
        );
    }

    // Regression: the same hazard as the test directly above, on the wholesale-reset path
    // instead of the per-entity one — and the harder of the two to notice. `clear_entities`
    // reset the archetype rows, the entity locations and the id allocator, and never touched
    // `sparse_sets`, so the first entity spawned afterwards took id 0 back and inherited
    // whatever id 0 used to hold. Nothing dangles and nothing panics; the new entity simply
    // *has* a component nobody gave it.
    //
    // It is worse here than after a `despawn` for a reason the despawn case does not have:
    // `Entities::clear` resets the GENERATIONS too, so the recycled handle is identical bit for
    // bit rather than merely sharing an id. `observer::tests::clear_entities_drops_entity_listeners`
    // is the other half of that — the world's second per-entity map, found by the sweep this
    // fix asked for.
    #[test]
    fn clear_entities_clears_sparse_components() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        #[derive(Clone, Debug, PartialEq)]
        struct SparseC(i32);
        impl crate::component::Component for SparseC {
            fn storage_type() -> crate::component::StorageType {
                crate::component::StorageType::SparseSet
            }
        }

        static ADDS: AtomicUsize = AtomicUsize::new(0);
        ADDS.store(0, Ordering::SeqCst);

        let mut world = World::new();
        world.register_component_type::<SparseC>();
        world.register_on_add::<SparseC>(Box::new(|_, _| {
            ADDS.fetch_add(1, Ordering::SeqCst);
        }));

        let e = world.spawn();
        world.add_component(e, SparseC(5));
        assert_eq!(ADDS.load(Ordering::SeqCst), 1, "sanity: a first attach fires on_add");

        world.clear_entities();

        // Pin the id reuse the whole scenario rests on, so this cannot pass for the wrong
        // reason if the allocator ever stops restarting at 0.
        let e2 = world.spawn();
        assert_eq!(e2.id(), e.id(), "sanity: clear_entities restarts ids");

        assert!(
            world.query_entity::<&SparseC>(e2.id()).is_none(),
            "an entity spawned after clear_entities inherited a stale sparse component"
        );
        assert_eq!(
            world.query::<&SparseC>().unwrap().iter().count(),
            0,
            "clear_entities leaked a sparse component into the query iterator"
        );
        assert!(
            !world
                .entity_component_types(e2)
                .contains(&std::any::TypeId::of::<SparseC>()),
            "entity_component_types reports a stale sparse component after clear_entities"
        );

        // The stale row also corrupted the WRITE path: `add_component`'s sparse branch reads
        // `set.contains(id)` to choose add-vs-overwrite, so a leftover entry made a genuine
        // first attach look like an overwrite and swallowed `on_add`.
        world.add_component(e2, SparseC(7));
        assert_eq!(
            ADDS.load(Ordering::SeqCst),
            2,
            "a first attach after clear_entities was mistaken for an overwrite"
        );
    }

    // Regression: clone_entity (prefab splice) clones archetype/table columns via
    // batch_clone_row but never copied SparseSet components, so clones silently
    // lacked the source's sparse components.
    #[test]
    fn clone_entity_copies_sparse_components() {
        #[derive(Clone, Debug, PartialEq)]
        struct TableC(i32);
        impl crate::component::Component for TableC {}
        #[derive(Clone, Debug, PartialEq)]
        struct SparseC(i32);
        impl crate::component::Component for SparseC {
            fn storage_type() -> crate::component::StorageType {
                crate::component::StorageType::SparseSet
            }
        }

        let mut world = World::new();
        world.register_component_type::<TableC>();
        world.register_component_type::<SparseC>();

        let src = world.spawn();
        world.add_component(src, TableC(1));
        world.add_component(src, SparseC(2));

        let clones = world.clone_entity(src.id(), 3).expect("clone_entity");
        assert_eq!(clones.len(), 3);
        for c in &clones {
            assert_eq!(
                world.query_entity::<&SparseC>(c.id()).map(|s| s.0),
                Some(2),
                "clone is missing the source's sparse component"
            );
        }
    }

    // Regression: the type-erased accessors used by reflection / scene
    // serialization only looked at archetype columns, so SparseSet components
    // were invisible — entity_component_types omitted them and get_component_ptr
    // returned None, silently dropping them from saved scenes.
    #[test]
    fn type_erased_access_includes_sparse() {
        #[derive(Clone, Debug, PartialEq)]
        struct TableC(i32);
        impl crate::component::Component for TableC {}
        #[derive(Clone, Debug, PartialEq)]
        struct SparseC(i32);
        impl crate::component::Component for SparseC {
            fn storage_type() -> crate::component::StorageType {
                crate::component::StorageType::SparseSet
            }
        }

        let mut world = World::new();
        world.register_component_type::<TableC>();
        world.register_component_type::<SparseC>();

        let e = world.spawn();
        world.add_component(e, TableC(1));
        world.add_component(e, SparseC(42));

        let types = world.entity_component_types(e);
        assert!(
            types.contains(&std::any::TypeId::of::<TableC>()),
            "entity_component_types missed the table component"
        );
        assert!(
            types.contains(&std::any::TypeId::of::<SparseC>()),
            "entity_component_types missed the sparse component"
        );

        let ptr = world
            .get_component_ptr(e, std::any::TypeId::of::<SparseC>())
            .expect("get_component_ptr returned None for a sparse component");
        // SAFETY: test-local — the pointer was looked up by `TypeId::of::<SparseC>()`, so the cast matches
        // the bytes, and the world outlives this read.
        let val = unsafe { &*(ptr as *const SparseC) };
        assert_eq!(val.0, 42, "get_component_ptr read the wrong sparse value");
    }

    #[test]
    fn add_same_component_overwrites() {
        #[derive(Clone, Debug, PartialEq)]
        struct TestCompI32(i32);
        impl crate::component::Component for TestCompI32 {}

        let mut world = World::new();
        world.register_component_type::<TestCompI32>();
        
        let e = world.spawn();
        world.add_component(e, TestCompI32(1));
        world.add_component(e, TestCompI32(99)); // overwrite
        
        assert_eq!(world.borrow::<TestCompI32>().get(e.id()).unwrap().0, 99);
    }

    #[test]
    fn archetype_graph_reuses_archetypes() {
        #[derive(Clone, Debug, PartialEq)]
        struct TestCompI32(i32);
        impl crate::component::Component for TestCompI32 {}

        #[derive(Clone, Debug, PartialEq)]
        struct TestCompF32(f32);
        impl crate::component::Component for TestCompF32 {}

        let mut world = World::new();
        world.register_component_type::<TestCompI32>();
        world.register_component_type::<TestCompF32>();
        
        let e1 = world.spawn(); world.add_component(e1, TestCompI32(1)); world.add_component(e1, TestCompF32(1.0));
        let e2 = world.spawn(); world.add_component(e2, TestCompI32(2)); world.add_component(e2, TestCompF32(2.0));
        
        let loc1 = world.entity_location(e1.id());
        let loc2 = world.entity_location(e2.id());
        assert_eq!(loc1.archetype_id, loc2.archetype_id);
        
        assert!(world.archetype_index.archetypes.len() < 5);
    }

    #[test]
    fn query_finds_matching_archetypes() {
        #[derive(Clone)]
        #[allow(dead_code)]
        struct TestCompI32(i32);
        impl crate::component::Component for TestCompI32 {}

        #[derive(Clone)]
        #[allow(dead_code)]
        struct TestCompF32(f32);
        impl crate::component::Component for TestCompF32 {}

        #[derive(Clone)]
        #[allow(dead_code)]
        struct TestCompBool(bool);
        impl crate::component::Component for TestCompBool {}

        let mut world = World::new();
        world.register_component_type::<TestCompI32>();
        world.register_component_type::<TestCompF32>();
        world.register_component_type::<TestCompBool>();
        
        let e1 = world.spawn(); world.add_component(e1, TestCompI32(1)); world.add_component(e1, TestCompF32(1.0));
        let e2 = world.spawn(); world.add_component(e2, TestCompI32(2)); world.add_component(e2, TestCompBool(true));
        let e3 = world.spawn(); world.add_component(e3, TestCompI32(3)); // sadece i32
        
        // i32 query'si 3 entity'yi de bulmalı
        let count = world.query::<&TestCompI32>().unwrap().iter().count();
        assert_eq!(count, 3);
        
        // (i32, f32) query'si sadece e1'i bulmalı
        let count = world.query::<(&TestCompI32, &TestCompF32)>().unwrap().iter().count();
        assert_eq!(count, 1);
    }

    #[test]
    fn query_mut_modifies_data() {
        #[derive(Clone)]
        struct TestCompI32(i32);
        impl crate::component::Component for TestCompI32 {}

        let mut world = World::new();
        world.register_component_type::<TestCompI32>();
        
        let e1 = world.spawn(); world.add_component(e1, TestCompI32(1));
        let e2 = world.spawn(); world.add_component(e2, TestCompI32(2));
        
        // Query ile tüm i32'leri iki katına çıkar
        if let Some(mut q) = world.query_mut::<crate::query::Mut<TestCompI32>>() {
            for (_, mut val) in q.iter_mut() {
                val.0 *= 2;
            }
        }
        
        assert_eq!(world.borrow::<TestCompI32>().get(e1.id()).unwrap().0, 2);
        assert_eq!(world.borrow::<TestCompI32>().get(e2.id()).unwrap().0, 4);
    }

    #[test]
    fn query_skips_non_matching() {
        #[derive(Clone)]
        struct CompA;
        impl crate::component::Component for CompA {}
        #[derive(Clone)]
        struct CompB;
        impl crate::component::Component for CompB {}

        let mut world = World::new();
        world.register_component_type::<CompA>();
        world.register_component_type::<CompB>();

        for _ in 0..100 {
            let e = world.spawn();
            world.add_component(e, CompA);
        }

        for _ in 0..50 {
            let e = world.spawn();
            world.add_component(e, CompB);
        }

        let a_count = world.query::<&CompA>().unwrap().iter().count();
        let b_count = world.query::<&CompB>().unwrap().iter().count();
        let both_count = world.query::<(&CompA, &CompB)>().unwrap().iter().count();

        assert_eq!(a_count, 100);
        assert_eq!(b_count, 50);
        assert_eq!(both_count, 0);
    }

    #[test]
    fn spawn_despawn_10k_entities_archetype_stability() {
        #[derive(Clone)]
        #[allow(dead_code)]
        struct CompA(i32);
        impl crate::component::Component for CompA {}
        #[derive(Clone)]
        #[allow(dead_code)]
        struct CompB(f32);
        impl crate::component::Component for CompB {}

        let mut world = World::new();
        world.register_component_type::<CompA>();
        world.register_component_type::<CompB>();

        let initial_archetypes = world.archetype_index.archetypes.len();

        // Spawn 10k entities
        let mut entities = Vec::new();
        for i in 0..10_000 {
            let e = world.spawn();
            world.add_component(e, CompA(i));
            if i % 2 == 0 {
                world.add_component(e, CompB(i as f32));
            }
            entities.push(e);
        }

        // Despawn all
        for e in entities {
            world.despawn(e);
        }

        // Archetype sayısı aynı kalmalı
        let final_archetypes = world.archetype_index.archetypes.len();
        // 1 empty, 1 for CompA, 1 for (CompA, CompB) = 3 total usually.
        assert!(final_archetypes <= initial_archetypes + 2);
    }
}
