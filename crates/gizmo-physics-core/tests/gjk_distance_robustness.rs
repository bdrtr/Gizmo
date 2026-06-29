//! Regression (audit 2026-06-29): `Gjk::distance` must stay accurate when one shape
//! is a thin, high-aspect-ratio box (a tiny face next to huge side extents) and the
//! other is very close to that face.
//!
//! The support of such a box jumps between far corners (±10 on the long axes), which
//! used to make the distance sub-algorithm's simplex reduction degenerate to a NaN
//! barycentre and then cycle back to a far corner, returning ~14.14 instead of the
//! true ~0.01 separation. (This is what let a Mach-scale CCD bullet tunnel through a
//! thick wall: the speculative contact read a bogus 14 m gap and bailed.)

use gizmo_math::{Quat, Vec3};
use gizmo_physics_core::{Collider, Gjk};

fn gap(sphere: &Collider, sphere_pos: Vec3, boxc: &Collider, box_a_first: bool) -> f32 {
    let support = |dir: Vec3| {
        if box_a_first {
            Gjk::support_point(&boxc.shape, Vec3::ZERO, Quat::IDENTITY, dir)
                - Gjk::support_point(&sphere.shape, sphere_pos, Quat::IDENTITY, -dir)
        } else {
            Gjk::support_point(&sphere.shape, sphere_pos, Quat::IDENTITY, dir)
                - Gjk::support_point(&boxc.shape, Vec3::ZERO, Quat::IDENTITY, -dir)
        }
    };
    Gjk::distance(support).expect("distance").0
}

#[test]
fn distance_sphere_near_thin_box_is_accurate() {
    let radius = 0.099_517_87;
    let half_x = 0.299_569_7;
    let sphere = Collider::sphere(radius);
    let boxc = Collider::box_collider(Vec3::new(half_x, 10.0, 10.0)); // thin in X, huge in Y/Z

    // Sweep the sphere centre toward the box's -X face; the true shape-shape gap is
    // |cx| - half_x - radius. Check both Minkowski orderings (pipeline pairs can be
    // (box, sphere) or (sphere, box)) and with the kind of tiny FP noise in Y/Z that
    // a real simulation accumulates — that noise is what triggered the old failure.
    for &cx in &[-8.0_f32, -1.0, -0.5, -0.41, -0.4090705, -0.42, -0.45, -0.6] {
        for &(ny, nz) in &[(0.0f32, 0.0f32), (6.666938e-6, 6.666937e-6), (-3e-6, 5e-6)] {
            let pos = Vec3::new(cx, ny, nz);
            let expected = (cx.abs() - half_x - radius).max(0.0);
            for &box_first in &[true, false] {
                let g = gap(&sphere, pos, &boxc, box_first);
                assert!(
                    (g - expected).abs() < 0.02,
                    "cx={cx} noise=({ny},{nz}) box_first={box_first}: \
                     gap={g}, expected≈{expected} (degenerate far-corner regression)"
                );
            }
        }
    }
}

#[test]
fn distance_far_apart_sphere_box_matches_analytic() {
    // A plain sanity case: well-separated, no degeneracy. Must be exact.
    let sphere = Collider::sphere(0.25);
    let boxc = Collider::box_collider(Vec3::new(0.5, 0.5, 0.5));
    let g = gap(&sphere, Vec3::new(-5.0, 0.0, 0.0), &boxc, true);
    let expected = 5.0 - 0.5 - 0.25; // 4.25
    assert!((g - expected).abs() < 1e-3, "gap={g}, expected {expected}");
}
