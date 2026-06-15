//! Model-based (oracle) DIFFERENTIAL test for the archetype ECS (`World`).
//!
//! Faz 1 — ECS, motorun temel taşı ve `UnsafeCell<Column>` ile iç-değişebilirlik
//! kullanıyor; component ekleme/çıkarma ARKETİP GÖÇÜ tetikliyor (sütun-desync,
//! stale-handle, generation bug'larının yaşadığı yer). Bu test rastgele bir
//! spawn/add/remove/despawn dizisini hem gerçek `World`'e hem de basit bir
//! referans modele (oracle) uygular, sonra ikisinin TAM uyuştuğunu doğrular:
//!   * canlı entity sayısı,
//!   * her entity'nin component değerleri (generation-doğrulamalı `get_entity`),
//!   * `&A`, `&B` ve birleşik `(&A,&B)` query'lerinin döndürdüğü entity kümeleri,
//!   * despawn edilen handle'ların stale (None / !is_alive) olması.
//!
//! Bir tutarsızlık → arketip göçü veya handle doğrulamasında gerçek bir bug.

use gizmo_core::impl_component;
use gizmo_core::{Entity, World};
use proptest::prelude::*;
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq)]
struct CompA(u64);
impl_component!(CompA);

#[derive(Debug, Clone, PartialEq)]
struct CompB(u64);
impl_component!(CompB);

#[derive(Debug, Clone)]
enum Op {
    Spawn,
    Add { is_a: bool, idx: usize, val: u64 },
    Remove { is_a: bool, idx: usize },
    Despawn { idx: usize },
}

fn arb_op() -> impl Strategy<Value = Op> {
    prop_oneof![
        2 => Just(Op::Spawn),
        4 => (any::<bool>(), 0usize..64, any::<u64>())
            .prop_map(|(is_a, idx, val)| Op::Add { is_a, idx, val }),
        2 => (any::<bool>(), 0usize..64).prop_map(|(is_a, idx)| Op::Remove { is_a, idx }),
        1 => (0usize..64).prop_map(|idx| Op::Despawn { idx }),
    ]
}

/// Referans modeldeki tek bir canlı entity.
#[derive(Clone)]
struct Live {
    e: Entity,
    a: Option<u64>,
    b: Option<u64>,
}

fn read_a(world: &World, e: Entity) -> Option<u64> {
    world.query::<&CompA>().and_then(|q| q.get_entity(e).map(|c| c.0))
}
fn read_b(world: &World, e: Entity) -> Option<u64> {
    world.query::<&CompB>().and_then(|q| q.get_entity(e).map(|c| c.0))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn ecs_matches_reference_model(ops in prop::collection::vec(arb_op(), 2..90)) {
        let mut world = World::new();
        world.register_component_type::<CompA>();
        world.register_component_type::<CompB>();

        let mut live: Vec<Live> = Vec::new();
        let mut stale: Vec<Entity> = Vec::new();

        for op in &ops {
            match *op {
                Op::Spawn => {
                    let e = world.spawn();
                    live.push(Live { e, a: None, b: None });
                }
                Op::Add { is_a, idx, val } => {
                    if live.is_empty() { continue; }
                    let i = idx % live.len();
                    let e = live[i].e;
                    if is_a {
                        world.add_component(e, CompA(val));
                        live[i].a = Some(val);
                    } else {
                        world.add_component(e, CompB(val));
                        live[i].b = Some(val);
                    }
                }
                Op::Remove { is_a, idx } => {
                    if live.is_empty() { continue; }
                    let i = idx % live.len();
                    let e = live[i].e;
                    if is_a {
                        world.remove_component::<CompA>(e);
                        live[i].a = None;
                    } else {
                        world.remove_component::<CompB>(e);
                        live[i].b = None;
                    }
                }
                Op::Despawn { idx } => {
                    if live.is_empty() { continue; }
                    let i = idx % live.len();
                    let removed = live.remove(i);
                    world.despawn(removed.e);
                    stale.push(removed.e);
                }
            }
        }

        // 1) Canlı entity sayısı eşleşir.
        prop_assert_eq!(world.entity_count() as usize, live.len(),
            "canlı entity sayısı modelle uyuşmuyor");

        // 2) Her canlı entity'nin component değerleri (generation-doğrulamalı).
        for l in &live {
            prop_assert!(world.is_alive(l.e), "model canlı ama world ölü: {:?}", l.e);
            prop_assert_eq!(read_a(&world, l.e), l.a, "CompA değeri uyuşmuyor: {:?}", l.e);
            prop_assert_eq!(read_b(&world, l.e), l.b, "CompB değeri uyuşmuyor: {:?}", l.e);
        }

        // 3) Query'lerin döndürdüğü entity kümeleri modelle eşleşir.
        let model_a: HashSet<u32> = live.iter().filter(|l| l.a.is_some()).map(|l| l.e.id()).collect();
        let model_b: HashSet<u32> = live.iter().filter(|l| l.b.is_some()).map(|l| l.e.id()).collect();
        let model_ab: HashSet<u32> =
            live.iter().filter(|l| l.a.is_some() && l.b.is_some()).map(|l| l.e.id()).collect();

        let world_a: HashSet<u32> = world.query::<&CompA>()
            .map(|q| q.iter().map(|(id, _)| id).collect()).unwrap_or_default();
        let world_b: HashSet<u32> = world.query::<&CompB>()
            .map(|q| q.iter().map(|(id, _)| id).collect()).unwrap_or_default();
        let world_ab: HashSet<u32> = world.query::<(&CompA, &CompB)>()
            .map(|q| q.iter().map(|(id, _)| id).collect()).unwrap_or_default();

        prop_assert_eq!(world_a, model_a, "&CompA query kümesi uyuşmuyor");
        prop_assert_eq!(world_b, model_b, "&CompB query kümesi uyuşmuyor");
        prop_assert_eq!(world_ab, model_ab, "(&CompA,&CompB) query kümesi uyuşmuyor");

        // 4) Despawn edilen handle'lar stale: !is_alive ve get_entity None.
        //    (id slot'u yeniden kullanılsa bile generation farkı reddetmeli.)
        for &s in &stale {
            prop_assert!(!world.is_alive(s), "stale handle hâlâ canlı: {:?}", s);
            prop_assert_eq!(read_a(&world, s), None, "stale handle CompA döndürdü: {:?}", s);
            prop_assert_eq!(read_b(&world, s), None, "stale handle CompB döndürdü: {:?}", s);
        }
    }
}
