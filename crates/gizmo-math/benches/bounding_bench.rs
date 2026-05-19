use bevy_math::{
    bounding::{Aabb3d, BoundingSphere, BoundingVolume},
    prelude::*,
    Vec3, Vec3A, Quat, Isometry3d,
};
use core::hint::black_box;
use criterion::{criterion_group, criterion_main, Criterion};
use rand::{
    distributions::{Distribution, Standard, Uniform},
    rngs::StdRng,
    Rng, SeedableRng,
};

macro_rules! bench {
    ($name:expr) => {
        $name
    };
}

struct PointCloud {
    points: Vec<Vec3A>,
    isometry: Isometry3d,
}

impl PointCloud {
    fn aabb(&self) -> Aabb3d {
        // In bevy_math 0.15, from_point_cloud expects Isometry3d and point iter
        Aabb3d::from_point_cloud(self.isometry, self.points.iter().map(|p| Vec3::from(*p)))
    }

    fn sphere(&self) -> BoundingSphere {
        // In bevy_math 0.15, from_point_cloud expects Isometry3d and slice, so we map to Vec3
        let vec3_points: Vec<Vec3> = self.points.iter().map(|p| Vec3::from(*p)).collect();
        BoundingSphere::from_point_cloud(self.isometry, &vec3_points)
    }
}

fn bounding(c: &mut Criterion) {
    let mut rng1 = StdRng::seed_from_u64(123);
    let mut rng2 = StdRng::seed_from_u64(456);

    let point_clouds = Uniform::<usize>::new(black_box(3), black_box(30))
        .sample_iter(&mut rng1)
        .take(black_box(1000))
        .map(|num_points| PointCloud {
            points: Standard
                .sample_iter(&mut rng2)
                .take(num_points)
                .map(|p: [f32; 3]| Vec3A::from_array(p))
                .collect::<Vec<Vec3A>>(),
            isometry: Isometry3d::new(
                Vec3::new(rng2.gen(), rng2.gen(), rng2.gen()),
                Quat::from_array(rng2.gen()),
            ),
        })
        .collect::<Vec<_>>();

    c.bench_function(bench!("bounding"), |b| {
        b.iter(|| {
            let aabb = point_clouds
                .iter()
                .map(PointCloud::aabb)
                .reduce(|l, r| l.merge(&r));

            let sphere = point_clouds
                .iter()
                .map(PointCloud::sphere)
                .reduce(|l, r| l.merge(&r));

            black_box(aabb);
            black_box(sphere);
        });
    });
}

criterion_group!(benches, bounding);
criterion_main!(benches);
