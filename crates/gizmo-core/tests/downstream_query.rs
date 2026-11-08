//! A primitive query term written outside `gizmo-core`. `WorldQuery` is an `unsafe trait` with its
//! contract on it, and this is the impl a downstream crate writes: `Has<T>`, which yields whether a row
//! has a `T` without reading it — Bevy's `Has<T>`, which Gizmo does not ship.

use std::marker::PhantomData;

use gizmo_core::archetype::sparse_set::ComponentSparseSet;
use gizmo_core::archetype::Archetype;
use gizmo_core::component::{Component, StorageType};
use gizmo_core::component_key::ComponentKey;
use gizmo_core::prelude::*;
use gizmo_core::query::{QueryData, ReadOnlyQuery, WorldQuery};

/// Whether the row has a `T`.
struct Has<T>(PhantomData<T>);

/// Where a row's answer is: settled for the whole archetype, or a sparse set to ask.
#[derive(Clone, Copy)]
enum HasFetch {
    Archetype(bool),
    Sparse(*const ComponentSparseSet),
}

// SAFETY: it reads whether an archetype has a column and whether a sparse set holds an entity, never a
// component's value or ticks, so there is no access to declare and nothing a parallel writer could race;
// `fetch_raw` keeps only a pointer to a sparse set the world owns for `'w`; it matches every archetype
// and every row, which is never narrower than the truth; its `StaticType` is itself, which no other term
// names; and it yields a `bool`, so it is read-only.
unsafe impl<T: Component> WorldQuery for Has<T> {
    type StaticType = Has<T>;
    type Fetch<'w> = HasFetch;
    type Item<'w> = bool;
    type Slice<'w> = ();

    unsafe fn fetch_raw<'w>(world: &'w World, arch: &Archetype, _tick: u32) -> Option<Self::Fetch<'w>> {
        let key = ComponentKey::of::<T>();
        Some(match T::storage_type() {
            StorageType::Table => HasFetch::Archetype(arch.has_component(key)),
            StorageType::SparseSet => world
                .sparse_set(key)
                .map_or(HasFetch::Archetype(false), |set| HasFetch::Sparse(set)),
        })
    }

    fn check_aliasing(_types: &mut Vec<(ComponentKey, bool)>) {}

    fn matches_archetype(_arch: &Archetype) -> bool {
        true
    }

    unsafe fn get_item<'w>(fetch: Self::Fetch<'w>, _row: usize, entity_id: u32) -> Self::Item<'w> {
        match fetch {
            HasFetch::Archetype(has) => has,
            // SAFETY: the set is the world's, borrowed for `'w` by the query this fetch came from.
            HasFetch::Sparse(set) => unsafe { (*set).contains(entity_id) },
        }
    }

    unsafe fn filter_row<'w>(_fetch: Self::Fetch<'w>, _row: usize, _entity_id: u32, _tick: u32) -> bool {
        true
    }

    unsafe fn get_slice<'w>(_fetch: Self::Fetch<'w>, _len: usize) -> Self::Slice<'w> {}
}

// SAFETY: `Item` is a `bool` and `check_aliasing` declares no write.
unsafe impl<T: Component> ReadOnlyQuery for Has<T> {}
impl<T: Component> QueryData for Has<T> {}

#[derive(Clone)]
struct Health(u32);
#[derive(Clone)]
struct Shield;
#[derive(Clone)]
struct Stunned;
gizmo_core::impl_component!(Health, Shield);
gizmo_core::impl_component!(Stunned; StorageType::SparseSet);

/// The term answers for both storages, next to built-in terms in one tuple.
#[test]
fn a_query_term_written_downstream_answers_for_both_storages() {
    let mut world = World::new();
    let bare = world.spawn_bundle(Health(1));
    let guarded = world.spawn_bundle((Health(2), Shield));
    world.add_component(guarded, Stunned);
    let stunned = world.spawn_bundle(Health(3));
    world.add_component(stunned, Stunned);

    let q = world.query::<(Entity, &Health, Has<Shield>, Has<Stunned>)>();
    let mut rows: Vec<_> = q.iter().map(|(e, h, shield, stun)| (e, h.0, shield, stun)).collect();
    rows.sort_by_key(|r| r.1);
    assert_eq!(rows, vec![(bare, 1, false, false), (guarded, 2, true, true), (stunned, 3, false, true)]);
}

#[derive(Clone, Default)]
struct Shielded(usize);
gizmo_core::impl_component!(Shielded);

fn count_shielded(q: Query<Has<Shield>>, mut out: ResMut<Shielded>) {
    out.0 = q.iter().filter(|&has| has).count();
}

/// And it is a system parameter's term like any other: it declares nothing, so a writer of `Shield`
/// shares its batch.
#[test]
fn it_runs_in_a_system_and_declares_no_access() {
    let mut world = World::new();
    world.insert_resource(Shielded::default());
    world.spawn_bundle((Health(1), Shield));
    world.spawn_bundle(Health(2));
    world.spawn_bundle((Health(3), Shield));
    let mut schedule = Schedule::new();
    schedule.add_systems(count_shielded);
    schedule.run(&mut world, 0.016);
    assert_eq!(world.get_resource::<Shielded>().map(|s| s.0), Some(2));

    let mut declared = Vec::new();
    <Has<Shield> as WorldQuery>::check_aliasing(&mut declared);
    assert!(declared.is_empty());
}
