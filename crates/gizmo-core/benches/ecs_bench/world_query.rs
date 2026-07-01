use criterion::Criterion;
use gizmo_core::{
    world::World,
    query::Mut,
    entity::Entity,
};
use super::common::*;

const SIZES: [usize; 5] = [100, 316, 1000, 3162, 10000];

fn make_entity(rng: &mut impl rand::RngExt, size: usize) -> Entity {
    let x: f64 = rng.random();
    let id = -(1.0 - x).log2() * (size as f64);
    let x: f64 = rng.random();
    let generation = 1.0 + -(1.0 - x).log2() * 2.0;

    let id = id as u32 + 1;
    let bits = ((generation as u64) << 32) | (id as u64);
    Entity::from_bits(bits)
}

pub fn entity_set_build_and_lookup(c: &mut Criterion) {
    use chacha20::ChaCha8Rng;
    use rand::SeedableRng;
    use std::collections::HashSet;
    use criterion::Throughput;

    let mut group = c.benchmark_group("entity_hash");
    for size in SIZES {
        let mut rng = ChaCha8Rng::seed_from_u64(size as u64);
        let entities =
            Vec::from_iter(core::iter::repeat_with(|| make_entity(&mut rng, size)).take(size));

        group.throughput(Throughput::Elements(size as u64));
        group.bench_function(criterion::BenchmarkId::new("entity_set_build", size), |bencher| {
            bencher.iter_with_large_drop(|| HashSet::<Entity>::from_iter(entities.iter().copied()));
        });
        group.bench_function(criterion::BenchmarkId::new("entity_set_lookup_hit", size), |bencher| {
            let set = HashSet::<Entity>::from_iter(entities.iter().copied());
            bencher.iter(|| entities.iter().copied().filter(|e| set.contains(e)).count());
        });
        group.bench_function(
            criterion::BenchmarkId::new("entity_set_lookup_miss_id", size),
            |bencher| {
                let set = HashSet::<Entity>::from_iter(entities.iter().copied());
                bencher.iter(|| {
                    entities
                        .iter()
                        .copied()
                        .map(|e| Entity::from_bits(e.to_bits() + 1))
                        .filter(|e| set.contains(e))
                        .count()
                });
            },
        );
        group.bench_function(
            criterion::BenchmarkId::new("entity_set_lookup_miss_gen", size),
            |bencher| {
                let set = HashSet::<Entity>::from_iter(entities.iter().copied());
                bencher.iter(|| {
                    entities
                        .iter()
                        .copied()
                        .map(|e| Entity::from_bits(e.to_bits() + (1 << 32)))
                        .filter(|e| set.contains(e))
                        .count()
                });
            },
        );
    }
}

pub fn entity_allocator_benches(criterion: &mut Criterion) {
    const ENTITY_COUNTS: [u32; 3] = [1, 100, 10_000];
    use gizmo_core::entity::allocator::Entities;

    let mut group = criterion.benchmark_group("entity_allocator_allocate_fresh");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    for entity_count in ENTITY_COUNTS {
        group.bench_function(format!("{entity_count}_entities"), |bencher| {
            bencher.iter_batched_ref(
                Entities::new,
                |allocator| {
                    for _ in 0..entity_count {
                        let entity = allocator.reserve_entity();
                        core::hint::black_box(entity);
                    }
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }

    group.finish();

    let mut group = criterion.benchmark_group("entity_allocator_allocate_fresh_bulk");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    for entity_count in ENTITY_COUNTS {
        group.bench_function(format!("{entity_count}_entities"), |bencher| {
            bencher.iter_batched_ref(
                Entities::new,
                |allocator| {
                    // Gizmo doesn't have bulk allocation yet, so we loop.
                    for _ in 0..entity_count {
                        let entity = allocator.reserve_entity();
                        core::hint::black_box(entity);
                    }
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }

    group.finish();

    let mut group = criterion.benchmark_group("entity_allocator_free");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    for entity_count in ENTITY_COUNTS {
        group.bench_function(format!("{entity_count}_entities"), |bencher| {
            bencher.iter_batched_ref(
                || {
                    let allocator = Entities::new();
                    let mut entities = Vec::with_capacity(entity_count as usize);
                    for _ in 0..entity_count {
                        entities.push(allocator.reserve_entity());
                    }
                    (allocator, entities)
                },
                |(allocator, entities)| {
                    entities.drain(..).for_each(|e| {
                        allocator.free(e);
                    });
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }

    group.finish();

    let mut group = criterion.benchmark_group("entity_allocator_free_bulk");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    for entity_count in ENTITY_COUNTS {
        group.bench_function(format!("{entity_count}_entities"), |bencher| {
            bencher.iter_batched_ref(
                || {
                    let allocator = Entities::new();
                    let mut entities = Vec::with_capacity(entity_count as usize);
                    for _ in 0..entity_count {
                        entities.push(allocator.reserve_entity());
                    }
                    (allocator, entities)
                },
                |(allocator, entities)| {
                    for e in entities {
                        allocator.free(*e);
                    }
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }

    group.finish();

    let mut group = criterion.benchmark_group("entity_allocator_allocate_reused");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    for entity_count in ENTITY_COUNTS {
        group.bench_function(format!("{entity_count}_entities"), |bencher| {
            bencher.iter_batched_ref(
                || {
                    let allocator = Entities::new();
                    let mut entities = Vec::with_capacity(entity_count as usize);
                    for _ in 0..entity_count {
                        entities.push(allocator.reserve_entity());
                    }
                    for e in &entities {
                        allocator.free(*e);
                    }
                    allocator
                },
                |allocator| {
                    for _ in 0..entity_count {
                        let entity = allocator.reserve_entity();
                        core::hint::black_box(entity);
                    }
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }

    group.finish();

    let mut group = criterion.benchmark_group("entity_allocator_allocate_reused_bulk");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    for entity_count in ENTITY_COUNTS {
        group.bench_function(format!("{entity_count}_entities"), |bencher| {
            bencher.iter_batched_ref(
                || {
                    let allocator = Entities::new();
                    let mut entities = Vec::with_capacity(entity_count as usize);
                    for _ in 0..entity_count {
                        entities.push(allocator.reserve_entity());
                    }
                    for e in &entities {
                        allocator.free(*e);
                    }
                    allocator
                },
                |allocator| {
                    for _ in 0..entity_count {
                        let entity = allocator.reserve_entity();
                        core::hint::black_box(entity);
                    }
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }

    group.finish();

    // Since Gizmo does not have Remote Allocators, these are tested via standard Entities allocations.
    let mut group = criterion.benchmark_group("entity_allocator_allocate_fresh_remote");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    for entity_count in ENTITY_COUNTS {
        group.bench_function(format!("{entity_count}_entities"), |bencher| {
            bencher.iter_batched_ref(
                Entities::new,
                |allocator| {
                    for _ in 0..entity_count {
                        let entity = allocator.reserve_entity();
                        core::hint::black_box(entity);
                    }
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }

    group.finish();

    let mut group = criterion.benchmark_group("entity_allocator_allocate_reused_remote");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    for entity_count in ENTITY_COUNTS {
        group.bench_function(format!("{entity_count}_entities"), |bencher| {
            bencher.iter_batched_ref(
                || {
                    let allocator = Entities::new();
                    let mut entities = Vec::with_capacity(entity_count as usize);
                    for _ in 0..entity_count {
                        entities.push(allocator.reserve_entity());
                    }
                    for e in &entities {
                        allocator.free(*e);
                    }
                    allocator
                },
                |allocator| {
                    for _ in 0..entity_count {
                        let entity = allocator.reserve_entity();
                        core::hint::black_box(entity);
                    }
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

pub fn world_entity(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("world_entity");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    for entity_count in RANGE.map(|i| i * 10_000) {
        group.bench_function(format!("{entity_count}_entities"), |bencher| {
            let (world, entities) = setup::<Table>(entity_count);

            bencher.iter(|| {
                for entity in &entities {
                    core::hint::black_box(world.is_alive(*entity));
                }
            });
        });
    }

    group.finish();
}

pub fn world_get(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("world_get");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    for entity_count in RANGE.map(|i| i * 10_000) {
        group.bench_function(format!("{entity_count}_entities_table"), |bencher| {
            let (world, entities) = setup::<Table>(entity_count);

            bencher.iter(|| {
                for entity in &entities {
                    assert!(world.query_entity::<&Table>(entity.id()).is_some());
                }
            });
        });
        group.bench_function(format!("{entity_count}_entities_sparse"), |bencher| {
            let (world, entities) = setup::<Sparse>(entity_count);

            bencher.iter(|| {
                for entity in &entities {
                    assert!(world.query_entity::<&Sparse>(entity.id()).is_some());
                }
            });
        });
    }

    group.finish();
}

pub fn world_query_get(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("world_query_get");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    for entity_count in RANGE.map(|i| i * 10_000) {
        group.bench_function(format!("{entity_count}_entities_table"), |bencher| {
            let (world, entities) = setup::<Table>(entity_count);
            let query = world.query::<&Table>().unwrap();

            bencher.iter(|| {
                for entity in &entities {
                    assert!(query.get(entity.id()).is_some());
                }
            });
        });
        group.bench_function(format!("{entity_count}_entities_table_wide"), |bencher| {
            let (world, entities) = setup_wide::<(
                WideTable<0>,
                WideTable<1>,
                WideTable<2>,
                WideTable<3>,
                WideTable<4>,
                WideTable<5>,
            )>(entity_count);
            let query = world.query::<(
                &WideTable<0>,
                &WideTable<1>,
                &WideTable<2>,
                &WideTable<3>,
                &WideTable<4>,
                &WideTable<5>,
            )>().unwrap();

            bencher.iter(|| {
                for entity in &entities {
                    assert!(query.get(entity.id()).is_some());
                }
            });
        });
        group.bench_function(format!("{entity_count}_entities_sparse"), |bencher| {
            let (world, entities) = setup::<Sparse>(entity_count);
            let query = world.query::<&Sparse>().unwrap();

            bencher.iter(|| {
                for entity in &entities {
                    assert!(query.get(entity.id()).is_some());
                }
            });
        });
        group.bench_function(format!("{entity_count}_entities_sparse_wide"), |bencher| {
            let (world, entities) = setup_wide::<(
                WideSparse<0>,
                WideSparse<1>,
                WideSparse<2>,
                WideSparse<3>,
                WideSparse<4>,
                WideSparse<5>,
            )>(entity_count);
            let query = world.query::<(
                &WideSparse<0>,
                &WideSparse<1>,
                &WideSparse<2>,
                &WideSparse<3>,
                &WideSparse<4>,
                &WideSparse<5>,
            )>().unwrap();

            bencher.iter(|| {
                for entity in &entities {
                    assert!(query.get(entity.id()).is_some());
                }
            });
        });
    }

    group.finish();
}

pub fn world_query_iter(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("world_query_iter");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    for entity_count in RANGE.map(|i| i * 10_000) {
        group.bench_function(format!("{entity_count}_entities_table"), |bencher| {
            let (world, _) = setup::<Table>(entity_count);
            let query = world.query::<&Table>().unwrap();

            bencher.iter(|| {
                let mut count = 0;
                for (_id, comp) in query.iter() {
                    core::hint::black_box(comp);
                    count += 1;
                    core::hint::black_box(count);
                }
                assert_eq!(core::hint::black_box(count), entity_count);
            });
        });
        group.bench_function(format!("{entity_count}_entities_sparse"), |bencher| {
            let (world, _) = setup::<Sparse>(entity_count);
            let query = world.query::<&Sparse>().unwrap();

            bencher.iter(|| {
                let mut count = 0;
                for (_id, comp) in query.iter() {
                    core::hint::black_box(comp);
                    count += 1;
                    core::hint::black_box(count);
                }
                assert_eq!(core::hint::black_box(count), entity_count);
            });
        });
    }

    group.finish();
}

pub fn world_query_for_each(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("world_query_for_each");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    for entity_count in RANGE.map(|i| i * 10_000) {
        group.bench_function(format!("{entity_count}_entities_table"), |bencher| {
            let (world, _) = setup::<Table>(entity_count);
            let query = world.query::<&Table>().unwrap();

            bencher.iter(|| {
                let mut count = 0;
                query.iter().for_each(|(_id, comp)| {
                    core::hint::black_box(comp);
                    count += 1;
                    core::hint::black_box(count);
                });
                assert_eq!(core::hint::black_box(count), entity_count);
            });
        });
        group.bench_function(format!("{entity_count}_entities_sparse"), |bencher| {
            let (world, _) = setup::<Sparse>(entity_count);
            let query = world.query::<&Sparse>().unwrap();

            bencher.iter(|| {
                let mut count = 0;
                query.iter().for_each(|(_id, comp)| {
                    core::hint::black_box(comp);
                    count += 1;
                    core::hint::black_box(count);
                });
                assert_eq!(core::hint::black_box(count), entity_count);
            });
        });
    }

    group.finish();
}

pub fn query_get(criterion: &mut Criterion) {
    use rand::seq::SliceRandom;

    let mut group = criterion.benchmark_group("query_get");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    for entity_count in RANGE.map(|i| i * 10_000) {
        group.bench_function(format!("{entity_count}_entities_table"), |bencher| {
            let mut world = World::new();
            let mut entities: Vec<_> = world
                .spawn_batch(std::iter::repeat_n((Table::default(),), entity_count as usize))
                .collect();
            use rand::SeedableRng;
            let mut rng = chacha20::ChaCha8Rng::seed_from_u64(42);
            entities.shuffle(&mut rng);

            let mut schedule = gizmo_core::system::Schedule::new();
            let entities_clone = entities.clone();
            schedule.add_di_system(move |query: gizmo_core::query::Query<&Table>| {
                let mut count = 0;
                for comp in entities_clone.iter().filter_map(|&e| query.get(e.id())) {
                    core::hint::black_box(comp);
                    count += 1;
                    core::hint::black_box(count);
                }
                assert_eq!(core::hint::black_box(count), entity_count);
            });
            schedule.build();
            bencher.iter(|| {
                schedule.run(&mut world, 0.0);
            });
        });
        group.bench_function(format!("{entity_count}_entities_sparse"), |bencher| {
            let mut world = World::new();
            let mut entities: Vec<_> = world
                .spawn_batch(std::iter::repeat_n((Sparse::default(),), entity_count as usize))
                .collect();
            use rand::SeedableRng;
            let mut rng = chacha20::ChaCha8Rng::seed_from_u64(42);
            entities.shuffle(&mut rng);

            let mut schedule = gizmo_core::system::Schedule::new();
            let entities_clone = entities.clone();
            schedule.add_di_system(move |query: gizmo_core::query::Query<&Sparse>| {
                let mut count = 0;
                for comp in entities_clone.iter().filter_map(|&e| query.get(e.id())) {
                    core::hint::black_box(comp);
                    count += 1;
                    core::hint::black_box(count);
                }
                assert_eq!(core::hint::black_box(count), entity_count);
            });
            schedule.build();
            bencher.iter(|| {
                schedule.run(&mut world, 0.0);
            });
        });
    }

    group.finish();
}

pub fn query_get_components_mut_2(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("world_query_get_components_mut_2");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    for entity_count in RANGE.map(|i| i * 10_000) {
        group.bench_function(format!("2_components_{entity_count}_entities"), |bencher| {
            let (mut world, entities) = setup_wide::<(WideTable<0>, WideTable<1>)>(entity_count);
            let mut query = world.query_mut::<(Mut<WideTable<0>>, Mut<WideTable<1>>)>().unwrap();
            bencher.iter(|| {
                for entity in &entities {
                    assert!(query.get_mut(entity.id()).is_some());
                }
            });
        });
    }

    group.finish();
}

pub fn query_get_components_mut_5(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("world_query_get_components_mut_5");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    for entity_count in RANGE.map(|i| i * 10_000) {
        group.bench_function(format!("5_components_{entity_count}_entities"), |bencher| {
            let (mut world, entities) = setup_wide::<(WideTable<0>, WideTable<1>, WideTable<2>, WideTable<3>, WideTable<4>)>(entity_count);
            let mut query = world.query_mut::<(Mut<WideTable<0>>, Mut<WideTable<1>>, Mut<WideTable<2>>, Mut<WideTable<3>>, Mut<WideTable<4>>)>().unwrap();
            bencher.iter(|| {
                for entity in &entities {
                    assert!(query.get_mut(entity.id()).is_some());
                }
            });
        });
    }

    group.finish();
}

pub fn query_get_components_mut_10(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("world_query_get_components_mut_10");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    for entity_count in RANGE.map(|i| i * 10_000) {
        group.bench_function(format!("10_components_{entity_count}_entities"), |bencher| {
            let (mut world, entities) = setup_wide::<(WideTable<0>, WideTable<1>, WideTable<2>, WideTable<3>, WideTable<4>, WideTable<5>, WideTable<6>, WideTable<7>, WideTable<8>, WideTable<9>)>(entity_count);
            let mut query = world.query_mut::<(Mut<WideTable<0>>, Mut<WideTable<1>>, Mut<WideTable<2>>, Mut<WideTable<3>>, Mut<WideTable<4>>, Mut<WideTable<5>>, Mut<WideTable<6>>, Mut<WideTable<7>>, Mut<WideTable<8>>, Mut<WideTable<9>>)>().unwrap();
            bencher.iter(|| {
                for entity in &entities {
                    assert!(query.get_mut(entity.id()).is_some());
                }
            });
        });
    }

    group.finish();
}
