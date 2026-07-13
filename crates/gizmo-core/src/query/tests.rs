//! Query system unit tests, moved out of query/mod.rs (verbatim, de-indented).

use super::*;
use crate::impl_component;

#[derive(Debug, Clone, PartialEq)]
struct Position {
    x: f32,
    y: f32,
}
impl_component!(Position);

#[derive(Debug, Clone, PartialEq)]
struct Velocity {
    x: f32,
    y: f32,
}
impl_component!(Velocity);

/// `Query<(Mut<Position>, Mut<Position>)>` gibi aynı tipe çift mutable erişim
/// denemesi panic ile engellenmeli.
#[test]
#[should_panic(expected = "Query aliasing UB detected")]
fn test_same_type_mut_mut_panics() {
    let mut types = Vec::new();
    // İlk Mut<Position> — sorunsuz eklenir
    check(TypeId::of::<Position>(), true, &mut types);
    // İkinci Mut<Position> — PANIC olmalı!
    check(TypeId::of::<Position>(), true, &mut types);
}

/// `Query<(&Position, Mut<Position>)>` — bir immutable, bir mutable aynı tipe erişim:
/// Bu da panic olmalı çünkü &T + &mut T alias oluşturur.
#[test]
#[should_panic(expected = "Query aliasing UB detected")]
fn test_same_type_ref_mut_panics() {
    let mut types = Vec::new();
    check(TypeId::of::<Position>(), false, &mut types); // &Position
    check(TypeId::of::<Position>(), true, &mut types); // Mut<Position> — PANIC!
}

/// `Query<(Mut<Position>, Mut<Velocity>)>` — farklı tipler, sorunsuz çalışmalı.
#[test]
fn test_different_types_mut_mut_ok() {
    let mut types = Vec::new();
    check(TypeId::of::<Position>(), true, &mut types);
    check(TypeId::of::<Velocity>(), true, &mut types);
    assert_eq!(types.len(), 2);
}

/// `Query<(&Position, &Position)>` — aynı tipe çift immutable erişim güvenlidir.
#[test]
fn test_same_type_ref_ref_ok() {
    let mut types = Vec::new();
    check(TypeId::of::<Position>(), false, &mut types);
    check(TypeId::of::<Position>(), false, &mut types);
    assert_eq!(types.len(), 2);
}

/// World üzerinden Query oluşturulduğunda aliasing kontrolünün çalıştığını doğrular.
#[test]
fn test_query_new_with_valid_types() {
    let mut world = crate::World::new();
    world.register_component_type::<Position>();
    world.register_component_type::<Velocity>();
    let e = world.spawn();
    world.add_component(e, Position { x: 1.0, y: 2.0 });
    world.add_component(e, Velocity { x: 0.0, y: 0.0 });

    // Farklı tipler — Query oluşturulabilmeli
    let q = world.query_mut::<(Mut<Position>, Mut<Velocity>)>();
    assert!(q.is_some());
}

/// `Changed<T>`/`Added<T>` artık referans tick'e (son çalıştırma) göre çalışır,
/// `== current_tick` değil. Kareler arası doğru raporlama doğrulanır.
#[test]
fn change_detection_is_relative_to_ref_tick() {
    let mut world = crate::World::new();
    world.register_component_type::<Position>();
    let e = world.spawn();
    world.add_component(e, Position { x: 1.0, y: 2.0 });

    // Frame 1: ref=0 → ilk gözlem eklenen bileşeni görür.
    world.begin_change_frame(0);
    assert_eq!(world.query::<Changed<Position>>().unwrap().iter().count(), 1);
    assert_eq!(world.query::<Added<Position>>().unwrap().iter().count(), 1);

    // Frame 2: değişiklik yok → Changed boş olmalı.
    let prev = world.tick;
    world.begin_change_frame(prev);
    assert_eq!(
        world.query::<Changed<Position>>().unwrap().iter().count(),
        0,
        "değişiklik olmayan frame'de Changed boş olmalı (eski `==` davranışı her şeyi eşliyordu)"
    );

    // Frame 2 içinde mutasyon → Changed yeniden 1 olmalı.
    {
        let mut q = world.query_mut::<Mut<Position>>().unwrap();
        for (_id, mut p) in q.iter_mut() {
            p.x += 1.0;
        }
    }
    assert_eq!(world.query::<Changed<Position>>().unwrap().iter().count(), 1);
}

/// `get_entity` generation'ı doğrular: despawn edilip slotu yeniden kullanılan bir
/// entity'nin eski handle'ı `None` döner; ham `get(id)` ise (footgun) yeni entity'nin
/// verisini döndürür.
#[test]
fn get_entity_rejects_stale_handle_after_despawn_reuse() {
    let mut world = crate::World::new();
    world.register_component_type::<Position>();

    let e1 = world.spawn();
    world.add_component(e1, Position { x: 1.0, y: 1.0 });
    let stale = e1;

    world.despawn(e1);

    // Slotu yeniden kullan — aynı id, artmış generation.
    let e2 = world.spawn();
    world.add_component(e2, Position { x: 2.0, y: 2.0 });
    assert_eq!(e2.id(), stale.id(), "slot yeniden kullanılmalı (aynı id)");
    assert_ne!(e2.generation(), stale.generation(), "generation artmalı");

    let q = world.query::<&Position>().unwrap();
    // Ham id: generation kontrolü yok → yeni entity'nin verisi (footgun).
    assert_eq!(q.get(stale.id()).map(|p| p.x), Some(2.0));
    // Generation-doğrulamalı: stale handle reddedilir.
    assert!(q.get_entity(stale).is_none(), "stale handle None dönmeli");
    // Geçerli handle çalışır.
    assert_eq!(q.get_entity(e2).map(|p| p.x), Some(2.0));
}

/// `iter_chunks_mut` ile yapılan toplu yazma, değişiklik tespitini tetiklemeli
/// (temkinli işaretleme → gerçek yazmayı asla kaçırmaz, false negative yok).
#[test]
fn iter_chunks_mut_triggers_change_detection() {
    let mut world = crate::World::new();
    world.register_component_type::<Position>();
    let e = world.spawn();
    world.add_component(e, Position { x: 1.0, y: 1.0 });

    // Referansı bu tick'e ayarla ve frame'i ilerlet (Schedule'ın yaptığı gibi).
    world.begin_change_frame(world.tick);
    // Yazmadan önce: değişiklik yok.
    assert_eq!(world.query::<Changed<Position>>().unwrap().iter().count(), 0);

    // Chunked mutable yazma.
    {
        let mut q = world.query_mut::<Mut<Position>>().unwrap();
        for (_ids, slice) in q.iter_chunks_mut() {
            for p in slice.iter_mut() {
                p.x += 10.0;
            }
        }
    }

    // Yazmadan sonra: Changed tetiklenmeli ve değer güncellenmeli.
    assert_eq!(world.query::<Changed<Position>>().unwrap().iter().count(), 1);
    assert_eq!(world.query::<&Position>().unwrap().get(e.id()).map(|p| p.x), Some(11.0));
}

/// SparseSet bileşenlerinde `Changed`/`Added` artık gerçek tick takibi yapar
/// (eskiden her zaman `true` idi). Tablo bileşenleriyle aynı kareler-arası semantik.
#[test]
fn sparse_set_change_detection_tracks_ticks() {
    #[derive(Clone, Debug, PartialEq)]
    struct SparseComp(i32);
    impl crate::component::Component for SparseComp {
        fn storage_type() -> crate::component::StorageType {
            crate::component::StorageType::SparseSet
        }
    }

    let mut world = crate::World::new();
    world.register_component_type::<SparseComp>();
    let e = world.spawn();
    world.add_component(e, SparseComp(1));

    // Frame 1: ref=0 → eklenen bileşen Added ve Changed olarak görülmeli.
    world.begin_change_frame(0);
    assert_eq!(world.query::<Added<SparseComp>>().unwrap().iter().count(), 1);
    assert_eq!(world.query::<Changed<SparseComp>>().unwrap().iter().count(), 1);

    // Frame 2: değişiklik yok → ikisi de boş (eski davranış burada hep 1 verirdi).
    let prev = world.tick;
    world.begin_change_frame(prev);
    assert_eq!(world.query::<Changed<SparseComp>>().unwrap().iter().count(), 0);
    assert_eq!(world.query::<Added<SparseComp>>().unwrap().iter().count(), 0);

    // Frame 2 içinde mutasyon → Changed yeniden tetiklenmeli.
    {
        let mut q = world.query_mut::<Mut<SparseComp>>().unwrap();
        for (_id, mut c) in q.iter_mut() {
            c.0 += 10;
        }
    }
    assert_eq!(world.query::<Changed<SparseComp>>().unwrap().iter().count(), 1);
    assert_eq!(world.query::<&SparseComp>().unwrap().get(e.id()).map(|c| c.0), Some(11));
}

// Sparse queries match EVERY archetype at the archetype level (data lives
// outside archetypes) and narrow per-row in filter_row. This exercises that
// narrowing with MIXED presence — some entities have the sparse component,
// some don't — which the single-entity tests and the all-uniform benches
// never cover. A narrowing bug would leak component-less entities (or read a
// non-existent sparse slot).
#[test]
fn sparse_query_mixed_presence_narrows_correctly() {
    use crate::component::{Component, StorageType};
    #[derive(Clone, Debug, PartialEq)]
    struct TableC(i32);
    impl Component for TableC {}
    #[derive(Clone, Debug, PartialEq)]
    struct SparseC(i32);
    impl Component for SparseC {
        fn storage_type() -> StorageType {
            StorageType::SparseSet
        }
    }

    let mut world = crate::World::new();
    world.register_component_type::<TableC>();
    world.register_component_type::<SparseC>();

    // 3 entities with TableC + SparseC, 2 with only TableC.
    for i in 0..3 {
        let e = world.spawn();
        world.add_component(e, TableC(i));
        world.add_component(e, SparseC(i * 10));
    }
    let mut table_only = Vec::new();
    for i in 3..5 {
        let e = world.spawn();
        world.add_component(e, TableC(i));
        table_only.push(e);
    }

    // &SparseC must yield exactly the 3 holders with the right values.
    {
        let q = world.query::<&SparseC>().unwrap();
        let mut vals: Vec<i32> = q.iter().map(|(_id, s)| s.0).collect();
        vals.sort();
        assert_eq!(vals, vec![0, 10, 20], "sparse query leaked/dropped rows under mixed presence");
    }
    // (&TableC, &SparseC): only the 3 with both.
    assert_eq!(
        world.query::<(&TableC, &SparseC)>().unwrap().iter().count(),
        3,
        "table+sparse tuple query miscounted"
    );
    // With<SparseC> keeps 3; Without<SparseC> keeps the 2 table-only.
    assert_eq!(
        world.query::<(&TableC, With<SparseC>)>().unwrap().iter().count(),
        3,
        "With<Sparse> miscounted"
    );
    assert_eq!(
        world.query::<(&TableC, Without<SparseC>)>().unwrap().iter().count(),
        2,
        "Without<Sparse> miscounted"
    );
    // Random access: table-only entities must report no SparseC.
    for e in &table_only {
        assert!(
            world.query::<&SparseC>().unwrap().get(e.id()).is_none(),
            "get() returned a sparse component for an entity that lacks it"
        );
    }
}
