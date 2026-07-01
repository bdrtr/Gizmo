use criterion::Criterion;
use gizmo_core::{
    world::World,
    query::{Query, Mut},
};
use super::common::*;

pub fn bench_simple_iter(c: &mut Criterion) {
    let mut world = World::new();

    world.spawn_batch((0..10_000).map(|_| {
        (
            Transform(Mat4::ONE),
            Position(Vec3::ZERO), // Replace Vec3::X with ZERO since my mock struct doesn't have X
            Rotation(Vec3::ZERO),
            Velocity(Vec3::ONE),
        )
    })).count();

    let mut query = world.query_mut::<(&Velocity, Mut<Position>)>().unwrap();
    query.iter_mut().for_each(|_| {}); // Warmup

    c.bench_function("simple_iter", |b| {
        b.iter(|| {
            for (_id, (velocity, mut position)) in query.iter_mut() {
                position.0.0[0] += velocity.0.0[0];
                position.0.0[1] += velocity.0.0[1];
                position.0.0[2] += velocity.0.0[2];
            }
        });
    });
}

pub fn bench_contiguous_iter(c: &mut Criterion) {
    let mut world = World::new();

    world.spawn_batch((0..10_000).map(|_| {
        (
            Transform(Mat4::ONE),
            Position(Vec3::ZERO),
            Rotation(Vec3::ZERO),
            Velocity(Vec3::ONE),
        )
    })).count();

    let mut query = world.query_mut::<(&Velocity, Mut<Position>)>().unwrap();
    query.iter_mut().for_each(|_| {}); // Warmup

    c.bench_function("contiguous_iter", |b| {
        b.iter(|| {
            let iter = query.iter_chunks_mut();
            for (_ids, (velocity_slice, position_slice)) in iter {
                assert!(velocity_slice.len() == position_slice.len());
                for (v, p) in velocity_slice.iter().zip(position_slice.iter_mut()) {
                    p.0.0[0] += v.0.0[0];
                    p.0.0[1] += v.0.0[1];
                    p.0.0[2] += v.0.0[2];
                }
            }
        });
    });
}

pub fn bench_contiguous_iter_avx2(c: &mut Criterion) {
    if !std::is_x86_feature_detected!("avx2") {
        return;
    }

    let mut world = World::new();

    world.spawn_batch((0..10_000).map(|_| {
        (
            Transform(Mat4::ONE),
            Position(Vec3::ZERO),
            Rotation(Vec3::ZERO),
            Velocity(Vec3::ONE),
        )
    })).count();

    let mut query = world.query_mut::<(&Velocity, Mut<Position>)>().unwrap();
    query.iter_mut().for_each(|_| {}); // Warmup

    #[target_feature(enable = "avx2")]
    unsafe fn exec(position: &mut [Position], velocity: &[Velocity]) {
        assert!(position.len() == velocity.len());
        for i in 0..position.len() {
            position[i].0.0[0] += velocity[i].0.0[0];
            position[i].0.0[1] += velocity[i].0.0[1];
            position[i].0.0[2] += velocity[i].0.0[2];
        }
    }

    c.bench_function("contiguous_iter_avx2", |b| {
        b.iter(|| {
            let iter = query.iter_chunks_mut();
            for (_ids, (velocity_slice, position_slice)) in iter {
                unsafe {
                    exec(position_slice, velocity_slice);
                }
            }
        });
    });
}

pub fn bench_for_each_iter(c: &mut Criterion) {
    let mut world = World::new();

    world.spawn_batch((0..10_000).map(|_| {
        (
            Transform(Mat4::ONE),
            Position(Vec3::ZERO),
            Rotation(Vec3::ZERO),
            Velocity(Vec3::ONE),
        )
    })).count();

    let mut query = world.query_mut::<(&Velocity, Mut<Position>)>().unwrap();
    query.iter_mut().for_each(|_| {}); // Warmup

    c.bench_function("for_each_iter", |b| {
        b.iter(|| {
            query.iter_mut().for_each(|(_id, (velocity, mut position))| {
                position.0.0[0] += velocity.0.0[0];
                position.0.0[1] += velocity.0.0[1];
                position.0.0[2] += velocity.0.0[2];
            });
        });
    });
}

pub fn bench_cache_locality_loss(c: &mut Criterion) {
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

    c.bench_function("cache_locality_loss", |b| {
        b.iter(|| {
            query.iter_mut().for_each(|(_id, (mut v1, v2))| {
                v1.0 += v2.0;
            });
        });
    });
}

pub fn bench_bypass_change_detection(c: &mut Criterion) {
    let mut world = World::new();

    world.spawn_batch((0..10_000).map(|_| {
        (
            Transform(Mat4::ONE),
            Position(Vec3::ZERO),
            Rotation(Vec3::ZERO),
            Velocity(Vec3::ONE),
        )
    })).count();

    let mut query = world.query_mut::<(&Velocity, Mut<Position>)>().unwrap();
    query.iter_mut().for_each(|_| {}); // Warmup

    c.bench_function("bypass_change_detection", |b| {
        b.iter(|| {
            for (_id, (velocity, mut position)) in query.iter_mut() {
                let p = position.bypass_change_detection();
                p.0.0[0] += velocity.0.0[0];
                p.0.0[1] += velocity.0.0[1];
                p.0.0[2] += velocity.0.0[2];
            }
        });
    });
}

pub fn bench_system_iter(c: &mut Criterion) {
    let mut world = World::new();

    world.spawn_batch((0..10_000).map(|_| {
        (
            Transform(Mat4::ONE),
            Position(Vec3::ZERO),
            Rotation(Vec3::ZERO),
            Velocity(Vec3::ONE),
        )
    })).count();

    fn query_system(mut query: Query<(&Velocity, Mut<Position>)>) {
        for (_id, (velocity, mut position)) in query.iter_mut() {
            position.0.0[0] += velocity.0.0[0];
            position.0.0[1] += velocity.0.0[1];
            position.0.0[2] += velocity.0.0[2];
        }
    }

    use gizmo_core::system::IntoSystem;
    let mut system = query_system.into_system();

    // Warmup
    system.run(&world, 0.0);

    c.bench_function("system_iter", |b| {
        b.iter(|| {
            system.run(&world, 0.0);
        });
    });
}
