use criterion::{Criterion, BatchSize};
use gizmo_core::world::World;
use super::common::*;

// 4. Spawn Batch vs Loop
pub fn bench_spawn_batch(c: &mut Criterion) {
    c.bench_function("spawn_batch_10k", |b| {
        b.iter_batched(
            World::new,
            |mut world| {
                let iter = (0..10_000).map(|_| {
                    (
                        Transform(Mat4::ONE),
                        Position(Vec3::ONE),
                        Rotation(Vec3::ONE),
                        Velocity(Vec3::ONE),
                    )
                });
                // Exhaust the iterator to actually spawn
                let _ = world.spawn_batch(iter).count();
            },
            BatchSize::LargeInput,
        );
    });

    c.bench_function("spawn_loop_10k", |b| {
        b.iter_batched(
            World::new,
            |mut world| {
                for _ in 0..10_000 {
                    world.spawn_bundle((
                        Transform(Mat4::ONE),
                        Position(Vec3::ONE),
                        Rotation(Vec3::ONE),
                        Velocity(Vec3::ONE),
                    ));
                }
            },
            BatchSize::LargeInput,
        );
    });
}
