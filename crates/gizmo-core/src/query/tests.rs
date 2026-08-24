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

/// An attempt at double mutable access to the same type, such as
/// `Query<(Mut<Position>, Mut<Position>)>`, must be blocked with a panic.
#[test]
#[should_panic(expected = "Query aliasing UB detected")]
fn test_same_type_mut_mut_panics() {
    let mut types = Vec::new();
    // İlk Mut<Position> — sorunsuz eklenir
    check(TypeId::of::<Position>(), true, &mut types);
    // İkinci Mut<Position> — PANIC olmalı!
    check(TypeId::of::<Position>(), true, &mut types);
}

/// `Query<(&Position, Mut<Position>)>` — one immutable, one mutable access to the same type:
/// this must panic too, because &T + &mut T forms an alias.
#[test]
#[should_panic(expected = "Query aliasing UB detected")]
fn test_same_type_ref_mut_panics() {
    let mut types = Vec::new();
    check(TypeId::of::<Position>(), false, &mut types); // &Position
    check(TypeId::of::<Position>(), true, &mut types); // Mut<Position> — PANIC!
}

/// `Query<(Mut<Position>, Mut<Velocity>)>` — different types, must work without problems.
#[test]
fn test_different_types_mut_mut_ok() {
    let mut types = Vec::new();
    check(TypeId::of::<Position>(), true, &mut types);
    check(TypeId::of::<Velocity>(), true, &mut types);
    assert_eq!(types.len(), 2);
}

/// `Query<(&Position, &Position)>` — double immutable access to the same type is safe.
#[test]
fn test_same_type_ref_ref_ok() {
    let mut types = Vec::new();
    check(TypeId::of::<Position>(), false, &mut types);
    check(TypeId::of::<Position>(), false, &mut types);
    assert_eq!(types.len(), 2);
}

/// Verifies that the aliasing check runs when a Query is created through the World.
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

/// `Changed<T>`/`Added<T>` now work relative to the reference tick (the last run),
/// not `== current_tick`. Correct reporting across frames is verified.
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

/// `get_entity` validates the generation: the old handle of an entity that was despawned and
/// had its slot reused returns `None`; raw `get(id)` on the other hand (footgun) returns the
/// new entity's data.
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

/// A bulk write made with `iter_chunks_mut` must trigger change detection
/// (conservative marking → never misses a real write, no false negative).
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

/// On SparseSet components `Changed`/`Added` now do real tick tracking
/// (formerly they were always `true`). The same across-frames semantics as Table components.
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

// Regression: get()/get_entity()/contains() must honour archetype-level (TABLE-storage)
// With/Without filters exactly like iter(). Table-storage With/Without is decided ONLY by
// matches_archetype — its fetch_raw always succeeds and filter_row always returns true
// (see impl_presence_filter). get_inner used to index the entity's OWN archetype directly,
// bypassing `matching_archetypes`, so get()/contains() returned Some/true for an entity that
// iter() correctly skipped. (The sparse case above is already narrowed by filter_row, so it
// never exhibited this; the table case is the one that leaked.)
#[test]
fn get_honours_table_with_without_like_iter() {
    let mut world = crate::World::new();
    world.register_component_type::<Position>();
    world.register_component_type::<Velocity>();

    // Entity with Position but NOT Velocity (both are table-storage components).
    let e = world.spawn();
    world.add_component(e, Position { x: 1.0, y: 2.0 });

    // With<Velocity>: iter skips e ⇒ get/get_entity/contains must all agree (skip).
    let q = world.query::<(&Position, With<Velocity>)>().unwrap();
    assert_eq!(q.iter().count(), 0, "iter should skip the Velocity-less entity");
    assert!(q.get(e.id()).is_none(), "get() must honour With<Velocity>");
    assert!(q.get_entity(e).is_none(), "get_entity() must honour With<Velocity>");
    assert!(!q.contains(e.id()), "contains() must honour With<Velocity>");

    // Without<Velocity>: iter yields e ⇒ get/contains must also yield it (consistency both ways).
    let q2 = world.query::<(&Position, Without<Velocity>)>().unwrap();
    assert_eq!(q2.iter().count(), 1, "iter should include the Velocity-less entity");
    assert!(q2.get(e.id()).is_some(), "get() must include under Without<Velocity>");
    assert!(q2.contains(e.id()), "contains() must include under Without<Velocity>");
}


// =========================================================================
// DEFAULT QUERY FILTERS
// =========================================================================

#[cfg(test)]
mod default_filter_tests {
    use crate::impl_component;
    use crate::query::{IgnoreDefaultFilters, Mut, Query, With, Without};
    use crate::system::{IntoSystemConfig, ResMut, Schedule};
    use crate::world::World;

    #[derive(Clone, Copy)]
    struct Disabled;
    impl_component!(Disabled);

    #[derive(Clone, Copy, Debug, PartialEq)]
    struct Hp(u32);
    impl_component!(Hp);

    /// What each system saw, so a test can compare views rather than side effects.
    #[derive(Default)]
    struct Seen {
        filtered: usize,
        hatched: usize,
    }

    /// Three entities with `Hp`; the middle one also carries `Disabled`.
    fn world_with_one_disabled(register_filter: bool) -> World {
        let mut world = World::new();
        for i in 0..3u32 {
            let e = world.spawn();
            world.add_component(e, Hp(i));
            if i == 1 {
                world.add_component(e, Disabled);
            }
        }
        world.insert_resource(Seen::default());
        if register_filter {
            world.default_query_filters_mut().add::<Disabled>();
        }
        world
    }

    fn run_once(world: &mut World, config: crate::system::SystemConfig) {
        let mut schedule = Schedule::new();
        schedule.add_di_system(config);
        schedule.build();
        schedule.run(world, 0.0);
    }

    /// A system query skips a filtered entity. This is the feature in one assertion.
    #[test]
    fn a_system_query_skips_a_filtered_entity() {
        fn count(q: Query<&Hp>, mut seen: ResMut<Seen>) {
            seen.filtered = q.iter().count();
        }

        let mut world = world_with_one_disabled(true);
        run_once(&mut world, count.into_config());
        assert_eq!(
            world.get_resource::<Seen>().map(|s| s.filtered),
            Some(2),
            "the system saw the disabled entity — the default filter is not reaching a system query",
        );
    }

    /// …and with no filter registered it sees everything, so the feature is free until asked for.
    #[test]
    fn nothing_is_filtered_until_a_filter_is_registered() {
        fn count(q: Query<&Hp>, mut seen: ResMut<Seen>) {
            seen.filtered = q.iter().count();
        }

        let mut world = world_with_one_disabled(false);
        run_once(&mut world, count.into_config());
        assert_eq!(
            world.get_resource::<Seen>().map(|s| s.filtered),
            Some(3),
            "a world with no registered filter dropped an entity anyway",
        );
    }

    /// `IgnoreDefaultFilters` brings it back — and it is the only thing that does.
    #[test]
    fn the_hatch_sees_the_filtered_entity() {
        fn both(q: Query<&Hp>, hatched: Query<(&Hp, IgnoreDefaultFilters)>, mut seen: ResMut<Seen>) {
            seen.filtered = q.iter().count();
            seen.hatched = hatched.iter().count();
        }

        let mut world = world_with_one_disabled(true);
        run_once(&mut world, both.into_config());
        let seen = world.get_resource::<Seen>().map(|s| (s.filtered, s.hatched));
        assert_eq!(
            seen,
            Some((2, 3)),
            "the two views in one system disagree by exactly the disabled entity, or should",
        );
    }

    /// **Nothing but a system parameter is filtered** — the other half of the rule, and the half
    /// the engine depends on.
    ///
    /// Every constructor is asserted, `unsafe` ones included, and the `unsafe` ones are the point.
    /// The first version of this feature filtered `query_unchecked`, on the theory that it was
    /// "the system path". It is not: it is also the engine's mutable-from-shared hatch, used from
    /// editor inspector panels, physics, netcode and audio — none of them system parameters. That
    /// version blanked the inspector for a disabled entity, froze whole transform subtrees, and
    /// made `sync_bodies` *destroy* a disabled rigid body instead of pausing it. Review caught it
    /// before it shipped; this test is what keeps it caught.
    #[test]
    fn nothing_but_a_system_parameter_is_filtered() {
        let mut world = world_with_one_disabled(true);

        assert_eq!(world.borrow::<Hp>().iter().count(), 3, "`World::borrow`");
        assert_eq!(world.query::<&Hp>().map(|q| q.iter().count()), Some(3), "`World::query`");
        assert!(
            world.query_entity::<&Hp>(1).is_some(),
            "`World::query_entity` — its own fetch path, outside `Query` entirely",
        );
        // SAFETY: single-threaded test, no other query alive.
        let unchecked = unsafe { world.query_unchecked::<&Hp>() };
        assert_eq!(
            unchecked.map(|q| q.iter().count()),
            Some(3),
            "`World::query_unchecked` was filtered — it is the engine's `&mut`-from-`&World` hatch, \
             not the system-parameter route, and 98 call sites read the world through it",
        );
        // SAFETY: as above. A mutable query has no `&self` iterator (that is what keeps two
        // `&mut T` unbuildable), so the count goes through `iter_mut`.
        let mut borrowed = unsafe { world.borrow_mut_unchecked::<Hp>() };
        assert_eq!(
            borrowed.iter_mut().count(),
            3,
            "`World::borrow_mut_unchecked` was filtered — this is what the editor inspector uses",
        );
        drop(borrowed);
        assert_eq!(world.borrow_mut::<Hp>().iter_mut().count(), 3, "`World::borrow_mut`");
        assert_eq!(
            world.query_cached::<&Hp>().map(|q| q.iter().count()),
            Some(3),
            "`World::query_cached`",
        );
    }

    /// `Or` must forward the opt-out, or a query that reads as an escape hatch is filtered anyway.
    ///
    /// Found by review before this shipped: `Or` forwards `check_aliasing` and `has_row_filter`
    /// explicitly and had simply not been given the third. There is no error to see — just a query
    /// quietly missing rows.
    #[test]
    fn or_forwards_the_opt_out() {
        use crate::query::Or;
        fn through_or(
            q: Query<(&Hp, Or<IgnoreDefaultFilters, With<Hp>>)>,
            mut seen: ResMut<Seen>,
        ) {
            seen.filtered = q.iter().count();
        }

        let mut world = world_with_one_disabled(true);
        run_once(&mut world, through_or.into_config());
        assert_eq!(
            world.get_resource::<Seen>().map(|s| s.filtered),
            Some(3),
            "`Or<IgnoreDefaultFilters, _>` was filtered — the opt-out did not survive the operand",
        );
    }

    /// **Every entry point of a filtered query agrees**, because they all go through one archetype
    /// list. `iter`, `get`, `contains` and `count` are asserted together on purpose: an earlier bug
    /// in this crate had `get`/`contains` bypass `matching_archetypes` and answer `Some`/`true` for
    /// an entity `iter` had excluded, and this is the shape of that bug.
    #[test]
    fn every_entry_point_of_a_filtered_query_agrees() {
        #[derive(Default)]
        struct Answers {
            iter_ids: Vec<u32>,
            got_disabled: bool,
            contains_disabled: bool,
            count: usize,
            chunk_total: usize,
        }

        fn probe(q: Query<&Hp>, mut out: ResMut<Answers>) {
            out.iter_ids = q.iter().map(|(id, _)| id).collect();
            out.count = q.iter().count();
            // The disabled entity is id 1 — spawned second, and ids start at 0.
            out.got_disabled = q.get(1).is_some();
            out.contains_disabled = q.contains(1);
            out.chunk_total = q.iter_chunks().map(|(_, s)| s.len()).sum();
        }

        let mut world = world_with_one_disabled(true);
        world.insert_resource(Answers::default());
        run_once(&mut world, probe.into_config());

        let a = world.get_resource::<Answers>().expect("answers");
        assert_eq!(a.iter_ids, vec![0, 2], "iter");
        assert_eq!(a.count, 2, "count");
        assert!(!a.got_disabled, "`get` returned the entity `iter` skipped");
        assert!(!a.contains_disabled, "`contains` claimed the entity `iter` skipped");
        assert_eq!(
            a.chunk_total, 2,
            "chunk iteration disagreed with `iter` — a filtered archetype is still being visited",
        );
    }

    /// Chunk iteration keeps working through a filter.
    ///
    /// It is worth its own name: the filter is applied to the ARCHETYPE LIST, not to rows, so
    /// `has_row_filter` is untouched and `iter_chunks` — which cannot honour a row filter and
    /// rejects any query carrying one — still serves the query. A design that filtered per row
    /// would have taken chunk iteration away from every query in the engine.
    #[test]
    fn a_filter_does_not_cost_chunk_iteration() {
        fn chunked(q: Query<&Hp>, mut seen: ResMut<Seen>) {
            // `iter_chunks` panics rather than lying if the query has a row filter.
            seen.filtered = q.iter_chunks().map(|(_, s)| s.len()).sum();
        }

        let mut world = world_with_one_disabled(true);
        run_once(&mut world, chunked.into_config());
        assert_eq!(world.get_resource::<Seen>().map(|s| s.filtered), Some(2));
    }

    /// The hatch composes with ordinary filters rather than replacing them.
    ///
    /// `With<Disabled>` + the hatch is how the system that RE-ENABLES entities is written, and it
    /// must see exactly the disabled ones — not everything, and not nothing.
    #[test]
    fn the_hatch_composes_with_an_ordinary_filter() {
        fn only_disabled(
            q: Query<(&Hp, With<Disabled>, IgnoreDefaultFilters)>,
            mut seen: ResMut<Seen>,
        ) {
            seen.filtered = q.iter().count();
        }
        fn only_enabled(q: Query<(&Hp, Without<Disabled>)>, mut seen: ResMut<Seen>) {
            seen.hatched = q.iter().count();
        }

        let mut world = world_with_one_disabled(true);
        run_once(&mut world, only_disabled.into_config());
        assert_eq!(
            world.get_resource::<Seen>().map(|s| s.filtered),
            Some(1),
            "`With<Disabled>` + the hatch must see exactly the disabled entity",
        );

        run_once(&mut world, only_enabled.into_config());
        assert_eq!(
            world.get_resource::<Seen>().map(|s| s.hatched),
            Some(2),
            "an explicit `Without` still works, and agrees with the implicit one",
        );
    }

    /// A mutable system query is filtered too — the write path, not only the read one.
    #[test]
    fn a_mutable_system_query_is_filtered() {
        fn bump(mut q: Query<Mut<Hp>>) {
            for (_, mut hp) in q.iter_mut() {
                hp.0 += 100;
            }
        }

        let mut world = world_with_one_disabled(true);
        run_once(&mut world, bump.into_config());

        let hp = world.borrow::<Hp>();
        let mut values: Vec<u32> = hp.iter().map(|(_, h)| h.0).collect();
        values.sort_unstable();
        assert_eq!(
            values,
            vec![1, 100, 102],
            "the disabled entity (Hp(1)) was written by a filtered mutable query",
        );
    }

    /// **The measurement, moved out of the demo.**
    ///
    /// `demo/src/bin/entity_disabling.rs` measured the cost of forgetting the filter once: two
    /// systems doing the same work, one writing `Without<Disabled>` and one not, diverging by 543
    /// extra updates by frame 240. With the implicit filter the forgetful system cannot be written
    /// — both see the same set — and the divergence is **zero**, which is what this asserts over
    /// the same 240 frames.
    #[test]
    fn the_forgetful_system_can_no_longer_be_written() {
        #[derive(Default)]
        struct Work {
            remembered: usize,
            forgot: usize,
        }

        fn remembers(q: Query<(&Hp, Without<Disabled>)>, mut w: ResMut<Work>) {
            w.remembered += q.iter().count();
        }
        // The same system with the filter left out. Before the implicit filter this processed the
        // disabled entity every frame; now the two are the same view.
        fn forgets(q: Query<&Hp>, mut w: ResMut<Work>) {
            w.forgot += q.iter().count();
        }

        let mut world = world_with_one_disabled(true);
        world.insert_resource(Work::default());
        let mut schedule = Schedule::new();
        schedule.add_di_system(remembers.into_config());
        schedule.add_di_system(forgets.into_config());
        schedule.build();
        for _ in 0..240 {
            schedule.run(&mut world, 0.0);
        }

        let w = world.get_resource::<Work>().expect("work");
        assert_eq!(w.remembered, 480, "2 live entities × 240 frames");
        assert_eq!(
            w.forgot, w.remembered,
            "the system that forgot the filter did {} updates against the careful one's {} — the \
             implicit filter is not covering it, which is the whole feature",
            w.forgot, w.remembered,
        );
    }
}
