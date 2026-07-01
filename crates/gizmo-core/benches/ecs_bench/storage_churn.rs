use criterion::Criterion;
use gizmo_core::{
    component::Component,
    world::World,
};
use super::common::*;

pub fn bench_insert_remove_sparseset(c: &mut Criterion) {
    let mut world = World::new();
    let mut entities = Vec::with_capacity(10_000);

    for _ in 0..10_000 {
        entities.push(world.spawn_bundle(A(0.0)));
    }

    c.bench_function("insert_remove_sparseset", |b| {
        b.iter(|| {
            for entity in &entities {
                world.add_component(*entity, B(0.0));
            }
            for entity in &entities {
                world.remove_component::<B>(*entity);
            }
        });
    });
}

// 2. Batch Operations Benchmark (Archetype Migration O(1))
pub fn bench_insert_remove_batch(c: &mut Criterion) {
    let mut world = World::new();
    let mut entities = Vec::with_capacity(10_000);

    for _ in 0..10_000 {
        entities.push(world.spawn_bundle(A(0.0)));
    }

    c.bench_function("insert_remove_batch", |b| {
        b.iter(|| {
            // O(1) Arch lookup
            world.insert_batch(&entities, Velocity(Vec3::ZERO));
            world.remove_batch::<Velocity>(&entities);
        });
    });
}

// 3. Heavyweight Nested Bundle
#[derive(Clone, Copy)]
struct F<const N: usize>(Mat4);
impl<const N: usize> Component for F<N> {}

pub fn bench_heavyweight_bundle(c: &mut Criterion) {
    let mut world = World::new();
    let mut entities = Vec::with_capacity(10_000);

    for _ in 0..10_000 {
        entities.push(world.spawn_bundle(A(0.0)));
    }

    c.bench_function("insert_remove_heavy_bundle", |b| {
        b.iter(|| {
            // O(1) Migration for 7 components at once
            for entity in &entities {
                world.add_bundle(*entity, (
                    F::<1>(Mat4::ONE),
                    F::<2>(Mat4::ONE),
                    F::<3>(Mat4::ONE),
                    F::<4>(Mat4::ONE),
                    F::<5>(Mat4::ONE),
                    F::<6>(Mat4::ONE),
                    F::<7>(Mat4::ONE),
                ));
            }

            for entity in &entities {
                world.remove_bundle::<(F<1>, F<2>, F<3>, F<4>, F<5>, F<6>, F<7>)>(*entity);
            }
        });
    });
}
