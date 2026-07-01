use criterion::Criterion;
use gizmo_core::{
    component::Component,
    world::World,
};
use super::common::*;

pub fn empty_commands(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("empty_commands");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    group.bench_function("0_entities", |bencher| {
        let mut world = World::new();
        let queue = gizmo_core::commands::CommandQueue::new();

        bencher.iter(|| {
            queue.apply(&mut world);
        });
    });

    group.finish();
}

pub fn spawn_commands(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("spawn_commands");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    for entity_count in [100, 1_000, 10_000] {
        group.bench_function(format!("{entity_count}_entities"), |bencher| {
            let mut world = World::new();
            let queue = gizmo_core::commands::CommandQueue::new();

            bencher.iter(|| {
                for i in 0..entity_count {
                    queue.push(move |world| {
                        let entity = world.spawn();
                        if i % 2 == 0 { world.add_component(entity, TestA(0.0)); }
                        if i % 3 == 0 { world.add_component(entity, TestB(0.0)); }
                        if i % 4 == 0 { world.add_component(entity, TestC(0.0)); }
                        if i % 5 == 0 { world.despawn(entity); }
                    });
                }
                queue.apply(&mut world);
            });
        });
    }

    group.finish();
}

pub fn nonempty_spawn_commands(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("nonempty_spawn_commands");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    for entity_count in [100, 1_000, 10_000] {
        group.bench_function(format!("{entity_count}_entities"), |bencher| {
            let mut world = World::new();
            let queue = gizmo_core::commands::CommandQueue::new();

            bencher.iter(|| {
                for i in 0..entity_count {
                    if core::hint::black_box(i % 2 == 0) {
                        queue.push(|world| {
                            let e = world.spawn();
                            world.add_component(e, TestA(0.0));
                        });
                    }
                }
                queue.apply(&mut world);
            });
        });
    }

    group.finish();
}

#[derive(Default, Clone, Copy)]
struct TestMatrix([[f32; 4]; 4]);
impl Component for TestMatrix {}

#[derive(Default, Clone, Copy)]
struct TestVec3([f32; 3]);
impl Component for TestVec3 {}

pub fn insert_commands(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("insert_commands");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    let entity_count = 10_000;
    group.bench_function("insert", |bencher| {
        let mut world = World::new();
        let mut entities = Vec::new();
        for _ in 0..entity_count {
            entities.push(world.spawn());
        }
        let queue = gizmo_core::commands::CommandQueue::new();

        bencher.iter(|| {
            for entity in &entities {
                let e = *entity;
                queue.push(move |world| {
                    world.add_bundle(e, (TestMatrix::default(), TestVec3::default()));
                });
            }
            queue.apply(&mut world);
        });
    });

    group.finish();
}

pub fn fake_commands(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("fake_commands");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    for command_count in [100, 1_000, 10_000] {
        group.bench_function(format!("{command_count}_commands"), |bencher| {
            let mut world = World::new();
            let queue = gizmo_core::commands::CommandQueue::new();

            bencher.iter(|| {
                for i in 0..command_count {
                    if core::hint::black_box(i % 2 == 0) {
                        queue.push(|world| { core::hint::black_box(world); });
                    } else {
                        queue.push(|world| { core::hint::black_box(world); });
                    }
                }
                queue.apply(&mut world);
            });
        });
    }

    group.finish();
}

fn sized_commands_impl<T: Default + Send + Sync + 'static>(criterion: &mut Criterion, name: &str) {
    let mut group = criterion.benchmark_group(name);
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    for command_count in [100, 1_000, 10_000] {
        group.bench_function(format!("{command_count}_commands"), |bencher| {
            let mut world = World::new();
            let queue = gizmo_core::commands::CommandQueue::new();

            bencher.iter(|| {
                for _ in 0..command_count {
                    let t = T::default();
                    queue.push(move |world| {
                        core::hint::black_box(t);
                        core::hint::black_box(world);
                    });
                }
                queue.apply(&mut world);
            });
        });
    }

    group.finish();
}

pub fn zero_sized_commands(criterion: &mut Criterion) {
    sized_commands_impl::<()>(criterion, "sized_commands_0_bytes");
}

pub fn medium_sized_commands(criterion: &mut Criterion) {
    sized_commands_impl::<(u32, u32, u32)>(criterion, "sized_commands_12_bytes");
}

#[derive(Clone, Copy)]
struct LargeStruct([u64; 64]);

impl Default for LargeStruct {
    fn default() -> Self {
        Self([0; 64])
    }
}

pub fn large_sized_commands(criterion: &mut Criterion) {
    sized_commands_impl::<LargeStruct>(criterion, "sized_commands_512_bytes");
}
