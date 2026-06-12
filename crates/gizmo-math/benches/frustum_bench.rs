use criterion::{black_box, criterion_group, criterion_main, Criterion};
use gizmo_math::frustum::{Frustum, Plane};
use glam::{Quat, Vec3A};

pub fn intersects_obb(c: &mut Criterion) {
    let mut group = c.benchmark_group("frustum_intersects");

    // Center and half_extents for OBB tests
    let center = Vec3A::ZERO;
    let half_extents = Vec3A::new(0.5, 0.5, 0.5);

    let rotation = Quat::from_rotation_y(std::f32::consts::FRAC_PI_4);
    let identity_rotation = Quat::IDENTITY;

    let sphere_center = Vec3A::new(1.0, 0.5, 0.0);
    let sphere_radius = 1.5;

    let frustum = Frustum {
        planes: [
            Plane::from_coefficients(-0.9701, -0.2425, -0.0000, 0.7276),
            Plane::from_coefficients(-0.0000, 1.0000, -0.0000, 1.0000),
            Plane::from_coefficients(-0.0000, -0.2425, -0.9701, 0.7276),
            Plane::from_coefficients(-0.0000, -1.0000, -0.0000, 1.0000),
            Plane::from_coefficients(-0.0000, -0.2425, 0.9701, 0.7276),
            Plane::from_coefficients(0.9701, -0.2425, -0.0000, 0.7276),
        ],
    };

    assert!(frustum.intersects_sphere(sphere_center, sphere_radius));
    group.bench_function("frustum_intersects_sphere", |b| {
        b.iter(|| black_box(frustum.intersects_sphere(black_box(sphere_center), black_box(sphere_radius))));
    });

    assert!(frustum.intersects_obb(center, half_extents, rotation));
    group.bench_function("frustum_intersects_obb", |b| {
        b.iter(|| {
            black_box(frustum.intersects_obb(
                black_box(center),
                black_box(half_extents),
                black_box(rotation),
            ))
        });
    });

    assert!(frustum.intersects_obb(center, half_extents, identity_rotation));
    group.bench_function("frustum_intersects_obb_identity", |b| {
        b.iter(|| {
            black_box(frustum.intersects_obb(
                black_box(center),
                black_box(half_extents),
                black_box(identity_rotation),
            ))
        });
    });

    group.finish();
}

criterion_group!(benches, intersects_obb);
criterion_main!(benches);
