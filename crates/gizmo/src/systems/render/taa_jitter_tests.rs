//! What [`jitter_projection`] has to be true of: the sub-pixel offset must land as the SAME
//! NDC shift at every depth. It did not for an orthographic camera, and the symptom was a
//! static scene that shimmered — see the function's own header for the measurement.

use super::jitter_projection;
use crate::math::{Mat4, Vec3, Vec4};

/// Where a view-space point ends up in NDC, under `proj`.
fn ndc(proj: Mat4, view_point: Vec3) -> (f32, f32) {
    let clip = proj * Vec4::new(view_point.x, view_point.y, view_point.z, 1.0);
    (clip.x / clip.w, clip.y / clip.w)
}

/// The jitter is a *sub-pixel* offset, so it must be the same shift at 2 m and at 200 m.
/// Depth-dependence is precisely the defect: on an orthographic camera the offset used to
/// scale with view depth, which is a several-pixel wobble rather than a sub-pixel one.
#[test]
fn orthographic_jitter_is_the_same_shift_at_every_depth() {
    let proj = Mat4::orthographic_rh(-4.0, 4.0, -3.0, 3.0, 0.1, 500.0);
    let (jx, jy) = (0.0011, -0.0007);
    let jittered = jitter_projection(proj, jx, jy);

    for depth in [-2.0f32, -20.0, -200.0] {
        let p = Vec3::new(1.0, -0.5, depth);
        let (x0, y0) = ndc(proj, p);
        let (x1, y1) = ndc(jittered, p);
        assert!(
            (x1 - x0 - jx).abs() < 1e-6 && (y1 - y0 - jy).abs() < 1e-6,
            "ortografik jitter {depth} m derinlikte kaydı: Δ=({}, {}) beklenen ({jx}, {jy})",
            x1 - x0,
            y1 - y0
        );
    }
}

/// The perspective path is the one that already worked; it is pinned so the fix cannot
/// have moved it.
#[test]
fn perspective_jitter_is_the_same_shift_at_every_depth() {
    let proj = Mat4::perspective_rh(1.0, 16.0 / 9.0, 0.1, 500.0);
    let (jx, jy) = (0.0011, -0.0007);
    let jittered = jitter_projection(proj, jx, jy);

    for depth in [-2.0f32, -20.0, -200.0] {
        let p = Vec3::new(1.0, -0.5, depth);
        let (x0, y0) = ndc(proj, p);
        let (x1, y1) = ndc(jittered, p);
        assert!(
            (x1 - x0 - jx).abs() < 1e-6 && (y1 - y0 - jy).abs() < 1e-6,
            "perspektif jitter {depth} m derinlikte kaydı: Δ=({}, {})",
            x1 - x0,
            y1 - y0
        );
    }
}

/// Zero jitter must leave the matrix alone, whichever branch it takes — a frame with TAA
/// disabled and one with a zero offset have to rasterise identically.
#[test]
fn zero_jitter_changes_nothing() {
    for proj in [
        Mat4::orthographic_rh(-4.0, 4.0, -3.0, 3.0, 0.1, 500.0),
        Mat4::perspective_rh(1.0, 16.0 / 9.0, 0.1, 500.0),
    ] {
        assert_eq!(jitter_projection(proj, 0.0, 0.0), proj);
    }
}
