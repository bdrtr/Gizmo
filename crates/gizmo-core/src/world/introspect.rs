//! World introspection / analysis surface.
//!
//! Read-only accessors that do not change behaviour. The goal: for the analysis layer on the
//! outside (gizmo-analysis) to be able to reach the **smallest detail** of a running engine's
//! ECS state — how many of which components are in which archetype, how many bytes they hold.
//!
//! Since this module is a sub-module of the `world` module it can reach `World`'s private
//! fields (archetype_index, component_infos, sparse_sets, resources); it changes none of them,
//! it only reads.

use super::World;
use std::any::TypeId;

/// Summary of a single component type inside an archetype.
#[derive(Debug, Clone)]
pub struct ComponentSummary {
    /// The component type's `TypeId` — the key everything else in the ECS is indexed by
    /// (archetype signatures, hook registration, sparse sets). Meaningful inside this
    /// process only: `TypeId` values are not stable across compilations, so never
    /// serialise one. [`Self::name`] carries the already-resolved human-readable name.
    pub type_id: TypeId,
    /// `std::any::type_name` (the full path captured at registration time).
    pub name: &'static str,
    /// The byte size of a single instance (`Layout::size`).
    pub item_size: usize,
    /// The number of instances in this archetype (= the archetype's entity count).
    pub count: usize,
    /// `item_size * count`.
    pub bytes: usize,
}

impl ComponentSummary {
    /// The last segment of the type name (`a::b::Transform` → `Transform`). For generics it
    /// trims the leading path but leaves the inside of `<...>` alone.
    pub fn short_name(&self) -> &str {
        short_type_name(self.name)
    }
}

/// Summary of a single archetype (the entity table with the same component composition).
#[derive(Debug, Clone)]
pub struct ArchetypeSummary {
    /// The archetype's id, equal to its index in the world's archetype table. Not a stable
    /// name for a component set across time: [`World::compact`] garbage-collects empty
    /// archetypes by swap-removing them, renumbering whichever archetype takes the freed
    /// slot.
    pub id: u32,
    /// Rows in this archetype — entities holding exactly this component set. Always ≥ 1,
    /// because [`World::archetype_summaries`] skips empty archetypes entirely.
    pub entity_count: usize,
    /// The total bytes of all of this archetype's component columns.
    pub bytes: usize,
    /// The components in descending order of byte usage.
    pub components: Vec<ComponentSummary>,
}

/// The world's top-level counters — the state at a glance.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WorldStats {
    /// Live entity count (the sum of the rows in all archetypes).
    pub entities: usize,
    /// Total archetype count (empty ones included).
    pub archetypes: usize,
    /// The number of archetypes with at least one entity in them.
    pub non_empty_archetypes: usize,
    /// The number of registered (seen) component types.
    pub registered_components: usize,
    /// The number of component types with sparse-set storage.
    pub sparse_set_components: usize,
    /// The number of registered resources.
    pub resources: usize,
    /// Live component bytes in the archetype columns — `layout.size() * rows`, summed.
    ///
    /// Table storage only, and deliberately so now that [`WorldStats::sparse_component_bytes`]
    /// exists beside it. Until 2026-08-31 this was the only figure, and it made an entire
    /// storage class invisible: a `SparseSet` component contributed nothing here however many
    /// entities carried it, so the memory `World::compact` reclaims from a sparse set's reverse
    /// index — the largest per-type allocation in the world — could not be seen in the numbers
    /// the engine prints about itself.
    pub component_bytes: usize,
    /// Live component bytes in the sparse sets, plus the reverse indices that address them.
    ///
    /// Two things, because they scale differently and the second is the one that surprises:
    /// `dense` is `layout.size() * entries`, but `sparse` is **four bytes per id up to the
    /// largest ever inserted**, whether or not those ids still carry the component. A set with
    /// one entry can hold megabytes. That is what `World::compact` truncates, and this is where
    /// to watch it happen.
    pub sparse_component_bytes: usize,
    /// The world tick.
    pub tick: u32,
}

/// A short name along the lines of `a::b::c::Type<x::y::Z>` → `Type<Z>`. Shallow but practical.
pub fn short_type_name(full: &str) -> &str {
    // Generic argümanların başlangıcından önceki son `::`'yi bul.
    let head_end = full.find('<').unwrap_or(full.len());
    let head = &full[..head_end];
    match head.rfind("::") {
        Some(pos) => &full[pos + 2..],
        None => full,
    }
}

impl World {
    /// The total number of live rows in the archetype tables. (`World::entity_count`
    /// is already defined by the allocator; this is the count as seen by the storage —
    /// normally they are equal.)
    #[inline]
    pub fn stored_entity_count(&self) -> usize {
        self.archetype_index
            .archetypes
            .iter()
            .map(|a| a.len())
            .sum()
    }

    /// The total archetype count.
    #[inline]
    pub fn archetype_count(&self) -> usize {
        self.archetype_index.archetypes.len()
    }

    /// The number of registered resources.
    #[inline]
    pub fn resource_count(&self) -> usize {
        self.resources.len()
    }

    /// The human-readable name of a component type (if it is registered).
    #[inline]
    pub fn component_type_name(&self, type_id: TypeId) -> Option<&'static str> {
        self.component_infos.get(&type_id).map(|i| i.type_name)
    }

    /// Top-level world statistics.
    pub fn world_stats(&self) -> WorldStats {
        let mut entities = 0usize;
        let mut non_empty = 0usize;
        let mut component_bytes = 0usize;

        for arch in &self.archetype_index.archetypes {
            let n = arch.len();
            if n == 0 {
                continue;
            }
            non_empty += 1;
            entities += n;
            for type_id in arch.component_types() {
                if let Some(info) = self.component_infos.get(&type_id) {
                    component_bytes += info.layout.size() * n;
                }
            }
        }

        // SparseSet storage lives outside the archetypes, so the loop above cannot see any of
        // it. `dense` scales with entries; `sparse` scales with the largest id ever inserted,
        // which is the interesting half.
        let sparse_component_bytes: usize = self
            .sparse_sets
            .values()
            .map(|set| {
                set.info.layout.size() * set.dense.len()
                    + std::mem::size_of::<u32>() * set.sparse.len()
            })
            .sum();

        WorldStats {
            entities,
            archetypes: self.archetype_index.archetypes.len(),
            non_empty_archetypes: non_empty,
            registered_components: self.component_infos.len(),
            sparse_set_components: self.sparse_sets.len(),
            resources: self.resources.len(),
            component_bytes,
            sparse_component_bytes,
            tick: self.tick,
        }
    }

    /// A detailed summary for every non-empty archetype (component names + bytes + count).
    /// The result is in descending order of entity count.
    pub fn archetype_summaries(&self) -> Vec<ArchetypeSummary> {
        let mut out = Vec::new();

        for arch in &self.archetype_index.archetypes {
            let n = arch.len();
            if n == 0 {
                continue;
            }

            let mut components = Vec::new();
            let mut arch_bytes = 0usize;
            for type_id in arch.component_types() {
                let (name, item_size) = match self.component_infos.get(&type_id) {
                    Some(info) => (info.type_name, info.layout.size()),
                    None => ("<unregistered>", 0),
                };
                let bytes = item_size * n;
                arch_bytes += bytes;
                components.push(ComponentSummary {
                    type_id,
                    name,
                    item_size,
                    count: n,
                    bytes,
                });
            }
            // Bayt kullanımına göre azalan; eşitlikte ada göre deterministik.
            components.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.name.cmp(b.name)));

            out.push(ArchetypeSummary {
                id: arch.id,
                entity_count: n,
                bytes: arch_bytes,
                components,
            });
        }

        out.sort_by(|a, b| b.entity_count.cmp(&a.entity_count).then(a.id.cmp(&b.id)));
        out
    }
}

#[cfg(test)]
mod tests {
    use crate::world::World;

    /// A sparse set's memory has to be visible in the numbers the engine prints about itself,
    /// and the reverse index is the half worth seeing: it is four bytes per id up to the largest
    /// ever inserted, so a set with ONE entry can be arbitrarily large.
    ///
    /// This is also the only place `World::compact`'s sparse reclamation is observable from
    /// outside the crate. A fix nobody can see in the reporting is a fix nobody can confirm.
    #[test]
    fn world_stats_sees_sparse_storage_and_its_reverse_index() {
        #[derive(Clone, Debug, PartialEq)]
        struct SparseW(u64);
        impl crate::component::Component for SparseW {
            fn storage_type() -> crate::component::StorageType {
                crate::component::StorageType::SparseSet
            }
        }

        let mut world = World::new();
        world.register_component_type::<SparseW>();

        const N: usize = 512;
        let ents: Vec<_> = (0..N).map(|_| world.spawn()).collect();
        for &e in &ents {
            world.add_component(e, SparseW(1));
        }
        let full = world.world_stats();
        assert_eq!(
            full.component_bytes, 0,
            "a SparseSet component is not in an archetype column, so the table figure stays 0"
        );
        assert_eq!(
            full.sparse_component_bytes,
            std::mem::size_of::<SparseW>() * N + std::mem::size_of::<u32>() * N,
            "512 live values plus a 512-entry reverse index"
        );

        // Drop all but the lowest id. The dense side collapses; the reverse index does NOT,
        // because it is sized by the largest id ever inserted.
        for &e in &ents[1..] {
            world.despawn(e);
        }
        let sparse_only = world.world_stats();
        assert_eq!(
            sparse_only.sparse_component_bytes,
            std::mem::size_of::<SparseW>() + std::mem::size_of::<u32>() * N,
            "one value left, and the whole index still addressing 512 ids"
        );

        // …until `compact` truncates it, which is the thing that was invisible before.
        world.compact();
        let compacted = world.world_stats();
        assert_eq!(
            compacted.sparse_component_bytes,
            std::mem::size_of::<SparseW>() + std::mem::size_of::<u32>(),
            "compaction is now visible in the reporting"
        );
        assert!(compacted.sparse_component_bytes < sparse_only.sparse_component_bytes);
    }


    #[derive(Clone)]
    struct Position {
        _x: f32,
        _y: f32,
        _z: f32,
    }
    #[derive(Clone)]
    struct Velocity {
        _v: [f32; 3],
    }
    impl crate::component::Component for Position {}
    impl crate::component::Component for Velocity {}

    #[test]
    fn introspection_reports_entities_archetypes_and_names() {
        let mut world = World::new();

        let a = world.spawn();
        world.add_component(a, Position { _x: 0.0, _y: 0.0, _z: 0.0 });

        let b = world.spawn();
        world.add_component(b, Position { _x: 1.0, _y: 0.0, _z: 0.0 });
        world.add_component(b, Velocity { _v: [0.0; 3] });

        let stats = world.world_stats();
        assert_eq!(stats.entities, 2);
        assert!(stats.registered_components >= 2);
        assert!(stats.component_bytes >= std::mem::size_of::<Position>() * 2);

        let summaries = world.archetype_summaries();
        // {Position} ve {Position,Velocity} olmak üzere iki dolu archetype.
        assert_eq!(summaries.len(), 2);

        // En az bir archetype Position içermeli ve adı çözülmüş olmalı.
        let has_position = summaries.iter().any(|s| {
            s.components
                .iter()
                .any(|c| c.short_name() == "Position" && c.item_size == std::mem::size_of::<Position>())
        });
        assert!(has_position, "Position component adı/boyutu çözülemedi");

        // Toplam entity, archetype-özet sayımıyla tutarlı.
        let total: usize = summaries.iter().map(|s| s.entity_count).sum();
        assert_eq!(total, 2);
    }

    #[test]
    fn short_type_name_strips_path_keeps_generics_tail() {
        use super::short_type_name;
        assert_eq!(short_type_name("gizmo_physics_core::Transform"), "Transform");
        assert_eq!(short_type_name("Foo"), "Foo");
    }
}
