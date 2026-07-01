use criterion::Criterion;
use gizmo_core::{
    component::Component,
    world::World,
};
use super::common::*;

fn flip_coin() -> bool {
    rand::random::<bool>()
}

// Ensure Table and Sparse take <const X: usize = 0> as per the prompt?
// Wait, Table<X> doesn't exist. There's Table(f32) and WideTable<X>(f32). I will reuse WideTable<X>!
fn spawn_empty_frag_archetype_wide<T: Component + Default>(world: &mut World) {
    for i in 0..10000 { // Reduced to 10k to prevent OOM / taking too long in CI
        let e = world.spawn();
        if flip_coin() {
            world.add_component(e, WideTable::<1>(0.0));
        }
        if flip_coin() {
            world.add_component(e, WideTable::<2>(0.0));
        }
        if flip_coin() {
            world.add_component(e, WideTable::<3>(0.0));
        }
        if flip_coin() {
            world.add_component(e, WideTable::<4>(0.0));
        }
        if flip_coin() {
            world.add_component(e, WideTable::<5>(0.0));
        }
        if flip_coin() {
            world.add_component(e, WideTable::<6>(0.0));
        }
        if flip_coin() {
            world.add_component(e, WideTable::<7>(0.0));
        }
        if flip_coin() {
            world.add_component(e, WideTable::<8>(0.0));
        }
        if flip_coin() {
            world.add_component(e, WideTable::<9>(0.0));
        }
        if flip_coin() {
            world.add_component(e, WideTable::<10>(0.0));
        }
        if flip_coin() {
            world.add_component(e, WideTable::<11>(0.0));
        }
        if flip_coin() {
            world.add_component(e, WideTable::<12>(0.0));
        }
        world.add_component(e, T::default());

        if i != 0 {
            world.despawn(e);
        }
    }
}

pub fn iter_frag_empty(c: &mut Criterion) {
    let mut group = c.benchmark_group("iter_fragmented_empty");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    group.bench_function("foreach_table", |b| {
        let mut world = World::new();
        spawn_empty_frag_archetype_wide::<Table>(&mut world);

        let mut schedule = gizmo_core::system::Schedule::new();
        fn iter_table(query: gizmo_core::query::Query<&Table >) {
            let mut res = 0;
            // Iterate over entities
            for (e, t) in query.iter() {
                res += e;
                core::hint::black_box(t);
            }
            core::hint::black_box(res);
        }
        schedule.add_di_system(iter_table);
        schedule.build();

        b.iter(|| {
            schedule.run(&mut world, 0.0);
        });
    });

    group.bench_function("foreach_sparse", |b| {
        let mut world = World::new();
        spawn_empty_frag_archetype_wide::<Sparse>(&mut world);

        let mut schedule = gizmo_core::system::Schedule::new();
        fn iter_sparse(query: gizmo_core::query::Query<&Sparse >) {
            let mut res = 0;
            // Iterate over entities
            for (e, t) in query.iter() {
                res += e;
                core::hint::black_box(t);
            }
            core::hint::black_box(res);
        }
        schedule.add_di_system(iter_sparse);
        schedule.build();

        b.iter(|| {
            schedule.run(&mut world, 0.0);
        });
    });
    group.finish();
}
