//! Regression (audit round 2, 2026-06-29): per-row filtering must be honored by the
//! `Or` combinator and rejected (not silently ignored) by `iter_chunks`.
//!
//! Pattern (A) sibling-divergence: every WorldQuery impl narrows at two levels —
//! `matches_archetype` (archetype) + `filter_row` (per-row). Operands whose
//! `matches_archetype` is intentionally WIDE (sparse `With`/`Without`, `Changed`/`Added`)
//! do the real test in `filter_row`. `Or` widened `matches_archetype` (T1||T2) but stubbed
//! `filter_row` to `true`, so it matched entities with NEITHER component. `iter_chunks`
//! returned whole contiguous archetype slices and never called `filter_row`, silently
//! ignoring those filters.

use gizmo_core::component::{Component, StorageType};
use gizmo_core::{Or, With, Without, World};

#[derive(Clone)]
struct Marker(u32);
impl Component for Marker {}

#[derive(Clone)]
struct SparseA;
impl Component for SparseA {
    fn storage_type() -> StorageType {
        StorageType::SparseSet
    }
}
#[derive(Clone)]
struct SparseB;
impl Component for SparseB {
    fn storage_type() -> StorageType {
        StorageType::SparseSet
    }
}

#[derive(Clone)]
struct TableA;
impl Component for TableA {}
#[derive(Clone)]
struct TableB;
impl Component for TableB {}

#[test]
fn or_with_sparse_operands_matches_either_never_neither() {
    let mut world = World::new();
    let a = world.spawn();
    world.add_component(a, Marker(1));
    world.add_component(a, SparseA);
    let b = world.spawn();
    world.add_component(b, Marker(2));
    world.add_component(b, SparseB);
    let _c = {
        let c = world.spawn();
        world.add_component(c, Marker(3)); // has NEITHER tag
        c
    };

    let mut got: Vec<u32> = world
        .query::<(&Marker, Or<With<SparseA>, With<SparseB>>)>()
        .unwrap()
        .iter()
        .map(|(_, (m, _))| m.0)
        .collect();
    got.sort();
    assert_eq!(
        got,
        vec![1, 2],
        "Or<With<sparse>,With<sparse>> must match entities with A or B, never the one with neither"
    );
}

#[test]
fn or_with_without_sparse_mixed() {
    let mut world = World::new();
    // a: A & B | b: B only | c: neither | d: A only
    let a = world.spawn();
    world.add_component(a, Marker(1));
    world.add_component(a, SparseA);
    world.add_component(a, SparseB);
    let b = world.spawn();
    world.add_component(b, Marker(2));
    world.add_component(b, SparseB);
    let c = world.spawn();
    world.add_component(c, Marker(3));
    let d = world.spawn();
    world.add_component(d, Marker(4));
    world.add_component(d, SparseA);

    // has A  OR  lacks B  → a(hasA), c(lacksB), d(hasA). b is excluded (no A, has B).
    let mut got: Vec<u32> = world
        .query::<(&Marker, Or<With<SparseA>, Without<SparseB>>)>()
        .unwrap()
        .iter()
        .map(|(_, (m, _))| m.0)
        .collect();
    got.sort();
    assert_eq!(got, vec![1, 3, 4]);
}

#[test]
fn or_with_table_operands_still_works() {
    let mut world = World::new();
    let a = world.spawn();
    world.add_component(a, Marker(1));
    world.add_component(a, TableA);
    let b = world.spawn();
    world.add_component(b, Marker(2));
    world.add_component(b, TableB);
    let c = world.spawn();
    world.add_component(c, Marker(3));

    let mut got: Vec<u32> = world
        .query::<(&Marker, Or<With<TableA>, With<TableB>>)>()
        .unwrap()
        .iter()
        .map(|(_, (m, _))| m.0)
        .collect();
    got.sort();
    assert_eq!(got, vec![1, 2], "the common Or<With<table>,...> case must still work");
    let _ = c;
}

#[test]
#[should_panic(expected = "per-row-filtered")]
fn iter_chunks_rejects_sparse_filtered_query() {
    let mut world = World::new();
    let a = world.spawn();
    world.add_component(a, Marker(1));
    world.add_component(a, SparseA);
    let q = world.query::<(&Marker, With<SparseA>)>().unwrap();
    let _ = q.iter_chunks().count(); // must panic, not silently ignore the filter
}

#[test]
fn iter_chunks_works_on_plain_data_and_table_filter() {
    let mut world = World::new();
    for i in 0..3 {
        let e = world.spawn();
        world.add_component(e, Marker(i));
    }
    // plain data query → fast path
    let q = world.query::<&Marker>().unwrap();
    let total: usize = q.iter_chunks().map(|(ids, _)| ids.len()).sum();
    assert_eq!(total, 3);

    // table With is archetype-level (no per-row filter) → chunks allowed
    let mut w2 = World::new();
    let a = w2.spawn();
    w2.add_component(a, Marker(1));
    w2.add_component(a, TableA);
    let b = w2.spawn();
    w2.add_component(b, Marker(2)); // no TableA → different archetype
    let q2 = w2.query::<(&Marker, With<TableA>)>().unwrap();
    let total2: usize = q2.iter_chunks().map(|(ids, _)| ids.len()).sum();
    assert_eq!(total2, 1, "table With selects the archetype; chunks yields only it");
}
