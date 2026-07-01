use criterion::Criterion;
use gizmo_core::{
    component::Component,
    world::World,
};

pub fn world_despawn(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("despawn_world");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    #[derive(Clone, Copy)]
    struct LocalA(crate::Mat4);
    impl Component for LocalA {}

    #[derive(Clone, Copy)]
    struct LocalB([f32; 4]);
    impl Component for LocalB {}

    for entity_count in [1, 100, 10_000] {
        group.bench_function(format!("{entity_count}_entities"), |bencher| {
            bencher.iter_batched_ref(
                || {
                    let mut world = World::new();
                    let mut entities = Vec::with_capacity(entity_count);
                    for _ in 0..entity_count {
                        let e = world.spawn();
                        world.add_bundle(e, (LocalA(crate::Mat4::ZERO), LocalB([0.0; 4])));
                        entities.push(e);
                    }
                    (world, entities)
                },
                |(world, entities)| {
                    entities.iter().for_each(|e| {
                        world.despawn(*e);
                    });
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

pub fn world_despawn_recursive(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("despawn_world_recursive");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    #[derive(Clone, Copy)]
    struct LocalA(crate::Mat4);
    impl Component for LocalA {}

    #[derive(Clone, Copy)]
    struct LocalB([f32; 4]);
    impl Component for LocalB {}

    for entity_count in [1, 100, 10_000] {
        group.bench_function(format!("{entity_count}_entities"), |bencher| {
            bencher.iter_batched_ref(
                || {
                    let mut world = World::new();
                    use gizmo_core::hierarchy::HierarchyExt;
                    let mut parent_ents = Vec::with_capacity(entity_count);
                    for _ in 0..entity_count {
                        let parent = world.spawn();
                        world.add_bundle(parent, (LocalA(crate::Mat4::ZERO), LocalB([0.0; 4])));
                        let child = world.spawn();
                        world.add_bundle(child, (LocalA(crate::Mat4::ZERO), LocalB([0.0; 4])));
                        world.add_child(parent, child);
                        parent_ents.push(parent);
                    }
                    (world, parent_ents)
                },
                |(world, parent_ents)| {
                    use gizmo_core::hierarchy::HierarchyExt;
                    parent_ents.iter().for_each(|e| {
                        world.despawn_recursive(*e);
                    });
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

pub fn world_spawn(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("spawn_world");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    #[derive(Clone, Copy)]
    struct LocalA(crate::Mat4);
    impl Component for LocalA {}

    #[derive(Clone, Copy)]
    struct LocalB([f32; 4]);
    impl Component for LocalB {}

    for entity_count in [1, 100, 10_000] {
        group.bench_function(format!("{entity_count}_entities"), |bencher| {
            bencher.iter_batched_ref(
                World::new,
                |world| {
                    for _ in 0..entity_count {
                        let e = world.spawn();
                        world.add_bundle(e, (LocalA(crate::Mat4::ZERO), LocalB([0.0; 4])));
                    }
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

pub fn world_spawn_batch(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("spawn_world_batch");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    #[derive(Clone, Copy)]
    struct LocalA(crate::Mat4);
    impl Component for LocalA {}

    #[derive(Clone, Copy)]
    struct LocalB([f32; 4]);
    impl Component for LocalB {}

    for batch_count in [1, 100, 1000, 10_000] {
        group.bench_function(format!("{batch_count}_entities"), |bencher| {
            bencher.iter_batched_ref(
                World::new,
                |world| {
                    for _ in 0..(10_000 / batch_count) {
                        let _ = world.spawn_batch(
                            std::iter::repeat_n((LocalA(crate::Mat4::ZERO), LocalB([0.0; 4])), batch_count as usize),
                        ).count();
                    }
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}
