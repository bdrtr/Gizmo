//! REGRESYON (audit 2026-06-29): SparseSet depolamalı bileşenlerde sorgu sağlamlığı.
//!
//! `matches_archetype` SparseSet için her arketipte `true` döner (bilinçli olarak geniş).
//! Eski kodda tekil `&T`/`Mut<T>` ve `With`/`Without` filtreleri satır-başı varlık
//! kontrolü yapmadığından:
//!   - `&T`/`Mut<T>`: bileşeni OLMAYAN entity için `get_item` sparse set'i sınır-dışı
//!     indeksliyordu → güvenli koddan PANİK (veya tombstone slot'ta release'de UB).
//!   - `With<T>`: bileşeni olmayan entity'leri de eşliyordu (yanlış sonuç).
//!   - `Without<T>`: bileşeni OLAN entity'leri de eşliyordu (yanlış sonuç).
//! Bu testler, motorun kendi bileşenleri Table kullandığından, latent ama güvenli-koddan
//! ulaşılabilen public-API hatasını tetikler.

use gizmo_core::component::StorageType;
use gizmo_core::{Component, Mut, With, Without, World};

#[derive(Clone, Debug, PartialEq)]
struct SparseTag(i32);
impl Component for SparseTag {
    fn storage_type() -> StorageType {
        StorageType::SparseSet
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Tableish(i32);
impl Component for Tableish {}

#[test]
fn bare_read_query_skips_entities_without_sparse_component() {
    let mut world = World::new();

    let a = world.spawn();
    world.add_component(a, SparseTag(10));
    // Farklı arketipte, SparseTag'i OLMAYAN bir entity — eski kodda burada OOB olurdu.
    let b = world.spawn();
    world.add_component(b, Tableish(99));

    let q = world.query::<&SparseTag>().expect("query");
    let mut got: Vec<(u32, i32)> = q.iter().map(|(id, c)| (id, c.0)).collect();
    got.sort();
    assert_eq!(got, vec![(a.id(), 10)], "yalnız SparseTag taşıyan entity ziyaret edilmeli");
}

#[test]
fn bare_mut_query_skips_entities_without_sparse_component() {
    let mut world = World::new();

    let a = world.spawn();
    world.add_component(a, SparseTag(10));
    let b = world.spawn();
    world.add_component(b, Tableish(99));

    {
        let mut q = world.query::<Mut<SparseTag>>().expect("query");
        for (_id, mut c) in q.iter_mut() {
            c.0 += 1;
        }
    }

    let q = world.query::<&SparseTag>().expect("query");
    let vals: Vec<i32> = q.iter().map(|(_, c)| c.0).collect();
    assert_eq!(vals, vec![11], "yalnız mevcut sparse bileşen mutasyona uğramalı, b'de OOB olmamalı");
    let _ = b;
}

#[test]
fn empty_sparse_set_query_does_not_panic() {
    let mut world = World::new();
    // Hiç SparseTag eklenmedi (sparse set hiç oluşmadı).
    let _ = world.spawn();
    let e = world.spawn();
    world.add_component(e, Tableish(1));

    let q = world.query::<&SparseTag>().expect("query");
    let n = q.iter().count();
    assert_eq!(n, 0, "sparse set yokken sorgu boş dönmeli (panik değil)");
}

#[test]
fn with_and_without_respect_sparse_presence() {
    let mut world = World::new();

    let a = world.spawn();
    world.add_component(a, SparseTag(10));
    world.add_component(a, Tableish(1));

    let b = world.spawn();
    world.add_component(b, Tableish(2)); // SparseTag YOK

    let c = world.spawn();
    world.add_component(c, SparseTag(30));
    world.add_component(c, Tableish(3));

    // With<SparseTag>: yalnız a ve c eşlenmeli.
    {
        let q = world.query::<(With<SparseTag>, &Tableish)>().expect("query");
        let mut vals: Vec<i32> = q.iter().map(|(_, (_, t))| t.0).collect();
        vals.sort();
        assert_eq!(vals, vec![1, 3], "With<sparse> yalnız bileşeni taşıyanları eşlemeli");
    }

    // Without<SparseTag>: yalnız b eşlenmeli.
    {
        let q = world.query::<(Without<SparseTag>, &Tableish)>().expect("query");
        let vals: Vec<i32> = q.iter().map(|(_, (_, t))| t.0).collect();
        assert_eq!(vals, vec![2], "Without<sparse> yalnız bileşeni TAŞIMAYANLARI eşlemeli");
    }
}

#[test]
fn tuple_query_with_sparse_and_table_components() {
    let mut world = World::new();

    let a = world.spawn();
    world.add_component(a, SparseTag(5));
    world.add_component(a, Tableish(50));

    let b = world.spawn();
    world.add_component(b, Tableish(60)); // SparseTag YOK → tuple eşlememeli

    let q = world.query::<(&SparseTag, &Tableish)>().expect("query");
    let mut got: Vec<(i32, i32)> = q.iter().map(|(_, (s, t))| (s.0, t.0)).collect();
    got.sort();
    assert_eq!(got, vec![(5, 50)], "tuple yalnız her iki bileşeni taşıyan entity'yi vermeli");
    let _ = b;
}
