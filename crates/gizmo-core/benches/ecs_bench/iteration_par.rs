use criterion::Criterion;
use gizmo_core::{
    world::World,
    query::Mut,
};
use super::common::*;

// 5. Heavy Compute
pub fn bench_heavy_compute(c: &mut Criterion) {
    let mut world = World::new();
    world.spawn_batch((0..1_000).map(|_| {
        (
            Transform(Mat4::ONE),
            Position(Vec3::ONE),
            Rotation(Vec3::ONE),
            Velocity(Vec3::ONE),
        )
    })).count();

    c.bench_function("heavy_compute_par", |b| {
        b.iter(|| {
            let mut query = world.query_mut::<(Mut<Position>, Mut<Transform>)>().unwrap();
            query.par_for_each_mut(|(_id, (mut pos, mut mat))| {
                for _ in 0..100 {
                    // simulate inverse matrix
                    mat.0.0[0] *= 0.99;
                }
                pos.0.0[0] *= mat.0.0[0];
            });
        });
    });
}

pub fn bench_par_cache_locality_loss(c: &mut Criterion) {
    let mut world = World::new();

    let mut v = vec![];
    for _ in 0..10_000 {
        world.spawn_bundle((A(0.0), B(0.0)));
        v.push(world.spawn_bundle(A(0.0)));
    }

    use rand::seq::SliceRandom;
    use rand::SeedableRng;
    let mut rng = rand::rngs::StdRng::seed_from_u64(42);
    v.shuffle(&mut rng);

    for e in v.into_iter() {
        world.despawn(e);
    }

    let mut query = world.query_mut::<(Mut<A>, &B)>().unwrap();
    query.iter_mut().for_each(|_| {}); // Warmup

    c.bench_function("par_cache_locality_loss", |b| {
        b.iter(|| {
            query.par_for_each_mut(|(_id, (mut v1, v2))| {
                v1.0 += v2.0;
            });
        });
    });
}
