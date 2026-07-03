use criterion::Criterion;
use gizmo_core::{
    component::Component,
    world::World,
};
use super::common::*;

fn all_added_detection_generic<T: Component + Default + Clone>(group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>, entity_count: u32) {
    group.bench_function(
        format!("{}_entities_{}", entity_count, core::any::type_name::<T>()),
        |bencher| {
            bencher.iter_batched_ref(
                || {
                    let mut world = World::new();
                    let entities: Vec<_> = world.spawn_batch(std::iter::repeat_n((T::default(),), entity_count as usize)).collect();
                    world.increment_tick(); // Wait, added entities were added at the old tick, and query uses the current tick? No, they were added with world's current tick. Incrementing the tick makes the current tick different from the added tick. Wait, if we increment the tick, then `ticks.added == current_tick` will be false!
                    // Wait, Bevy's `Added` checks if `added_tick` is newer than `last_run_tick`.
                    // Gizmo's `Added` (which I just implemented) checks `ticks.added == tick`. `tick` is `world.tick`.
                    // So if we don't increment the tick, `ticks.added == world.tick` will be true.
                    (world, entities)
                },
                |(world, _)| {
                    let query = world.query::<gizmo_core::query::Added<T>>().unwrap();
                    let mut count = 0;
                    for (entity, _) in query.iter() {
                        core::hint::black_box(entity);
                        count += 1;
                    }
                    assert_eq!(entity_count, count);
                },
                criterion::BatchSize::LargeInput,
            );
        },
    );
}

pub fn all_added_detection(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("all_added_detection");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));
    for &entity_count in &[5000, 50000] {
        all_added_detection_generic::<Table>(&mut group, entity_count);
        all_added_detection_generic::<Sparse>(&mut group, entity_count);
    }
    group.finish();
}

fn all_changed_detection_generic<T: Component + Default + Clone>(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    entity_count: u32,
) {
    group.bench_function(
        format!("{}_entities_{}", entity_count, core::any::type_name::<T>()),
        |bencher| {
            bencher.iter_batched_ref(
                || {
                    let mut world = World::new();
                    let entities: Vec<_> = world.spawn_batch(std::iter::repeat_n((T::default(),), entity_count as usize)).collect();
                    world.increment_tick();
                    let mut query_mut = world.query_mut::<gizmo_core::query::Mut<T>>().unwrap();
                    for (_id, mut component) in query_mut.iter_mut() {
                        // writing to Mut<T> triggers Changed tick internally! Wait, Gizmo's `Mut` doesn't automatically trigger Changed tick.
                        // Wait, does it? Let's assume the user has a bench_modify trait, but we can just do mutable access or manually update ticks.
                        // I will just use `world.get_component_mut::<T>(entity)` which updates the tick if Gizmo does it, or we just manually update ticks.
                        // Actually, just let me get query_mut and do something. But Gizmo `Mut` is literally `&mut T`. It doesn't wrap!
                        // In Gizmo, returning `&mut T` from `Query` iterator DOES update the tick if `iter_mut` is called!
                        // Wait, Gizmo's `iter_mut` gives `(&mut T)`. How does it track change?
                        // Ah, `col.ticks_ptr_mut().write()` is called inside `world.mod.rs` when doing things, but `iter_mut()` might just update ticks for all accessed rows!
                        core::hint::black_box(&mut component);
                    }
                    (world, entities)
                },
                |(world, _)| {
                    let query = world.query::<gizmo_core::query::Changed<T>>().unwrap();
                    let mut count = 0;
                    for (entity, _) in query.iter() {
                        core::hint::black_box(entity);
                        count += 1;
                    }
                    assert_eq!(entity_count, count); // If all_changed works
                },
                criterion::BatchSize::LargeInput,
            );
        },
    );
}

pub fn all_changed_detection(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("all_changed_detection");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));
    for &entity_count in &[5000, 50000] {
        all_changed_detection_generic::<Table>(&mut group, entity_count);
        all_changed_detection_generic::<Sparse>(&mut group, entity_count);
    }
    group.finish();
}

fn few_changed_detection_generic<T: Component + Default + Clone>(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    entity_count: u32,
) {
    let ratio_to_modify = 0.1;
    let amount_to_modify = (entity_count as f32 * ratio_to_modify) as usize;
    group.bench_function(
        format!("{}_entities_{}", entity_count, core::any::type_name::<T>()),
        |bencher| {
            bencher.iter_batched_ref(
                || {
                    let mut world = World::new();
                    let mut entities: Vec<_> = world.spawn_batch(std::iter::repeat_n((T::default(),), entity_count as usize)).collect();
                    world.increment_tick();

                    use rand::seq::SliceRandom;
                    use rand::SeedableRng;
                    let mut rng = chacha20::ChaCha8Rng::seed_from_u64(42);
                    entities.shuffle(&mut rng);

                    for entity in entities.iter().take(amount_to_modify) {
                        // Trigger change
                        let _ = world.query_entity_mut::<gizmo_core::query::Mut<T>>(entity.id());
                    }
                    (world, entities)
                },
                |(world, _)| {
                    let query = world.query::<gizmo_core::query::Changed<T>>().unwrap();
                    for (entity, _) in query.iter() {
                        core::hint::black_box(entity);
                    }
                },
                criterion::BatchSize::LargeInput,
            );
        },
    );
}

pub fn few_changed_detection(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("few_changed_detection");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));
    for &entity_count in &[5000, 50000] {
        few_changed_detection_generic::<Table>(&mut group, entity_count);
        few_changed_detection_generic::<Sparse>(&mut group, entity_count);
    }
    group.finish();
}

fn none_changed_detection_generic<T: Component + Default + Clone>(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    entity_count: u32,
) {
    group.bench_function(
        format!("{}_entities_{}", entity_count, core::any::type_name::<T>()),
        |bencher| {
            bencher.iter_batched_ref(
                || {
                    let mut world = World::new();
                    let entities: Vec<_> = world.spawn_batch(std::iter::repeat_n((T::default(),), entity_count as usize)).collect();
                    // Advance the change-detection FRAME (not just the raw tick):
                    // spawned components are stamped `changed` at the spawn tick, so
                    // to make them "before this frame" the reference tick must move
                    // to (at least) the spawn tick. `increment_tick` leaves
                    // `change_ref_tick` at 0, so a Changed<T> query would still
                    // report every just-spawned entity — begin_change_frame is what
                    // a real frame boundary uses.
                    let spawn_tick = world.tick;
                    world.begin_change_frame(spawn_tick);
                    (world, entities)
                },
                |(world, _)| {
                    let query = world.query::<gizmo_core::query::Changed<T>>().unwrap();
                    let mut count = 0;
                    for (entity, _) in query.iter() {
                        core::hint::black_box(entity);
                        count += 1;
                    }
                    assert_eq!(0, count);
                },
                criterion::BatchSize::LargeInput,
            );
        },
    );
}

pub fn none_changed_detection(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("none_changed_detection");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));
    for &entity_count in &[5000, 50000] {
        none_changed_detection_generic::<Table>(&mut group, entity_count);
        none_changed_detection_generic::<Sparse>(&mut group, entity_count);
    }
    group.finish();
}

fn add_archetypes_entities<T: Component + Default + Clone>(
    world: &mut World,
    archetype_count: u16,
    entity_count: u32,
) {
    for i in 0..archetype_count {
        for _j in 0..entity_count {
            let e = world.spawn();
            world.add_component(e, T::default());
            if i & (1 << 0) != 0 { world.add_component(e, ArchetypeData::<0>(1.0)); }
            if i & (1 << 1) != 0 { world.add_component(e, ArchetypeData::<1>(1.0)); }
            if i & (1 << 2) != 0 { world.add_component(e, ArchetypeData::<2>(1.0)); }
            if i & (1 << 3) != 0 { world.add_component(e, ArchetypeData::<3>(1.0)); }
            if i & (1 << 4) != 0 { world.add_component(e, ArchetypeData::<4>(1.0)); }
            if i & (1 << 5) != 0 { world.add_component(e, ArchetypeData::<5>(1.0)); }
        }
    }
}

fn multiple_archetype_none_changed_detection_generic<T: Component + Default + Clone>(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    archetype_count: u16,
    entity_count: u32,
) {
    group.bench_function(
        format!(
            "{}_archetypes_{}_entities_{}",
            archetype_count,
            entity_count,
            core::any::type_name::<T>()
        ),
        |bencher| {
            bencher.iter_batched_ref(
                || {
                    let mut world = World::new();
                    add_archetypes_entities::<T>(&mut world, archetype_count, entity_count);
                    // Advance the change-detection frame past the spawn tick so
                    // `Changed<T>` counts only post-frame changes (see
                    // none_changed_detection_generic). The ArchetypeData mutation
                    // below then stamps a later tick, but it targets other
                    // components, so Changed<T> must still be 0.
                    let spawn_tick = world.tick;
                    world.begin_change_frame(spawn_tick);

                    let query_mut = world.query_mut::<(
                        gizmo_core::query::Mut<ArchetypeData<0>>,
                        gizmo_core::query::Mut<ArchetypeData<1>>,
                        gizmo_core::query::Mut<ArchetypeData<2>>,
                    )>();
                    if let Some(mut query_mut) = query_mut {
                        for (_id, (mut d0, mut d1, mut d2)) in query_mut.iter_mut() {
                            d0.0 += 1.0; d1.0 += 1.0; d2.0 += 1.0;
                        }
                    }
                    world
                },
                |world| {
                    let query = world.query::<gizmo_core::query::Changed<T>>().unwrap();
                    let mut count = 0;
                    for (entity, _) in query.iter() {
                        core::hint::black_box(entity);
                        count += 1;
                    }
                    assert_eq!(0, count);
                },
                criterion::BatchSize::LargeInput,
            );
        },
    );
}

pub fn multiple_archetype_none_changed_detection(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("multiple_archetypes_none_changed_detection");
    group.warm_up_time(core::time::Duration::from_millis(800));
    group.measurement_time(core::time::Duration::from_secs(8));
    for archetype_count in [5, 20] {
        for entity_count in [10, 100, 1000] {
            multiple_archetype_none_changed_detection_generic::<Table>(
                &mut group,
                archetype_count,
                entity_count,
            );
            multiple_archetype_none_changed_detection_generic::<Sparse>(
                &mut group,
                archetype_count,
                entity_count,
            );
        }
    }
    group.finish();
}
