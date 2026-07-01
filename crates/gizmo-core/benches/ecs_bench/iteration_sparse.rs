use criterion::Criterion;
use gizmo_core::{
    world::World,
    query::Mut,
};
use super::common::*;

pub fn bench_sparse_iter(c: &mut Criterion) {
    let mut world = World::new();

    world.spawn_batch((0..10_000).map(|_| {
        (
            Transform(Mat4::ONE),
            SparsePos(Vec3::ZERO),
            Rotation(Vec3::ZERO),
            SparseVel(Vec3::ONE),
        )
    })).count();

    let mut query = world.query_mut::<(&SparseVel, Mut<SparsePos>)>().unwrap();
    query.iter_mut().for_each(|_| {}); // Warmup

    c.bench_function("sparse_iter", |b| {
        b.iter(|| {
            query.iter_mut().for_each(|(_id, (velocity, mut position))| {
                position.0.0[0] += velocity.0.0[0];
                position.0.0[1] += velocity.0.0[1];
                position.0.0[2] += velocity.0.0[2];
            });
        });
    });
}

pub fn bench_wide_sparse_iter(c: &mut Criterion) {
    let mut world = World::new();

    world.spawn_batch((0..10_000).map(|_| {
        (
            Transform(Mat4::ONE),
            Rotation(Vec3::ZERO),
            SparsePosWide::<0>(Vec3::ZERO),
            SparseVelWide::<0>(Vec3::ONE),
            SparsePosWide::<1>(Vec3::ZERO),
            SparseVelWide::<1>(Vec3::ONE),
            SparsePosWide::<2>(Vec3::ZERO),
            SparseVelWide::<2>(Vec3::ONE),
            SparsePosWide::<3>(Vec3::ZERO),
            SparseVelWide::<3>(Vec3::ONE),
            SparsePosWide::<4>(Vec3::ZERO),
            SparseVelWide::<4>(Vec3::ONE),
        )
    })).count();

    let mut query = world.query_mut::<(
        &SparseVelWide<0>,
        Mut<SparsePosWide<0>>,
        &SparseVelWide<1>,
        Mut<SparsePosWide<1>>,
        &SparseVelWide<2>,
        Mut<SparsePosWide<2>>,
        &SparseVelWide<3>,
        Mut<SparsePosWide<3>>,
        &SparseVelWide<4>,
        Mut<SparsePosWide<4>>,
    )>().unwrap();

    query.iter_mut().for_each(|_| {}); // Warmup

    c.bench_function("wide_sparse_iter", |b| {
        b.iter(|| {
            query.iter_mut().for_each(|(_id, (v0, mut p0, v1, mut p1, v2, mut p2, v3, mut p3, v4, mut p4))| {
                p0.0.0[0] += v0.0.0[0];
                p0.0.0[1] += v0.0.0[1];
                p0.0.0[2] += v0.0.0[2];

                p1.0.0[0] += v1.0.0[0];
                p1.0.0[1] += v1.0.0[1];
                p1.0.0[2] += v1.0.0[2];

                p2.0.0[0] += v2.0.0[0];
                p2.0.0[1] += v2.0.0[1];
                p2.0.0[2] += v2.0.0[2];

                p3.0.0[0] += v3.0.0[0];
                p3.0.0[1] += v3.0.0[1];
                p3.0.0[2] += v3.0.0[2];

                p4.0.0[0] += v4.0.0[0];
                p4.0.0[1] += v4.0.0[1];
                p4.0.0[2] += v4.0.0[2];
            });
        });
    });
}

pub fn bench_sparse_simple_iter(c: &mut Criterion) {
    let mut world = World::new();

    world.spawn_batch((0..10_000).map(|_| {
        (
            Transform(Mat4::ONE),
            SparsePos(Vec3::ZERO),
            Rotation(Vec3::ZERO),
            SparseVel(Vec3::ONE),
        )
    })).count();

    let mut query = world.query_mut::<(&SparseVel, Mut<SparsePos>)>().unwrap();
    query.iter_mut().for_each(|_| {}); // Warmup

    c.bench_function("sparse_simple_iter", |b| {
        b.iter(|| {
            for (_id, (velocity, mut position)) in query.iter_mut() {
                position.0.0[0] += velocity.0.0[0];
                position.0.0[1] += velocity.0.0[1];
                position.0.0[2] += velocity.0.0[2];
            }
        });
    });
}

pub fn bench_wide_sparse_simple_iter(c: &mut Criterion) {
    let mut world = World::new();

    world.spawn_batch((0..10_000).map(|_| {
        (
            Transform(Mat4::ONE),
            Rotation(Vec3::ZERO),
            SparsePosWide::<0>(Vec3::ZERO),
            SparseVelWide::<0>(Vec3::ONE),
            SparsePosWide::<1>(Vec3::ZERO),
            SparseVelWide::<1>(Vec3::ONE),
            SparsePosWide::<2>(Vec3::ZERO),
            SparseVelWide::<2>(Vec3::ONE),
            SparsePosWide::<3>(Vec3::ZERO),
            SparseVelWide::<3>(Vec3::ONE),
            SparsePosWide::<4>(Vec3::ZERO),
            SparseVelWide::<4>(Vec3::ONE),
        )
    })).count();

    let mut query = world.query_mut::<(
        &SparseVelWide<0>,
        Mut<SparsePosWide<0>>,
        &SparseVelWide<1>,
        Mut<SparsePosWide<1>>,
        &SparseVelWide<2>,
        Mut<SparsePosWide<2>>,
        &SparseVelWide<3>,
        Mut<SparsePosWide<3>>,
        &SparseVelWide<4>,
        Mut<SparsePosWide<4>>,
    )>().unwrap();

    query.iter_mut().for_each(|_| {}); // Warmup

    c.bench_function("wide_sparse_simple_iter", |b| {
        b.iter(|| {
            for (_id, (v0, mut p0, v1, mut p1, v2, mut p2, v3, mut p3, v4, mut p4)) in query.iter_mut() {
                p0.0.0[0] += v0.0.0[0];
                p0.0.0[1] += v0.0.0[1];
                p0.0.0[2] += v0.0.0[2];

                p1.0.0[0] += v1.0.0[0];
                p1.0.0[1] += v1.0.0[1];
                p1.0.0[2] += v1.0.0[2];

                p2.0.0[0] += v2.0.0[0];
                p2.0.0[1] += v2.0.0[1];
                p2.0.0[2] += v2.0.0[2];

                p3.0.0[0] += v3.0.0[0];
                p3.0.0[1] += v3.0.0[1];
                p3.0.0[2] += v3.0.0[2];

                p4.0.0[0] += v4.0.0[0];
                p4.0.0[1] += v4.0.0[1];
                p4.0.0[2] += v4.0.0[2];
            }
        });
    });
}
