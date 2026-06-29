//! # Gizmo Math
//!
//! Gizmo Engine'nin temel matematik altyapısını ve render/fizik veri tiplerini barındırır.
//!
//! ## Konvansiyonlar (Conventions)
//! - **Koordinat Sistemi**: Sağ-Elli (Right-Handed, RH).
//! - **Yukarı Ekseni**: Y-Up (0.0, 1.0, 0.0).
//! - **İleri Ekseni**: -Z (Kamera her zaman eksi Z eksenine doğru bakar).
//! - **Matris Düzeni**: Column-Major (glam mimarisi ile uyumlu).
//!
//! Normal matrisi hesaplamaları için yapılandırılmış `Mat3`, ve 3B uzay sınırları
//! hesaplamaları için boyut optimize edilmiş `Aabb`, `Frustum`, `Ray` yapıları barındırır.
//!
//! ## Public dependency: `glam`
//!
//! This crate **re-exports `glam`** ([`Vec2`], [`Vec3`], [`Vec3A`], [`Vec4`],
//! [`Mat3`], [`Mat4`], [`Quat`], [`EulerRot`]) as the engine-wide vector-math
//! vocabulary. `glam` is therefore an **official, intentional public dependency**:
//! these types appear directly in the public API of every Gizmo crate that does
//! math, and forcing callers through newtype wrappers would add no value.
//!
//! Consequence for semver: a `glam` **major** version bump is a breaking change
//! for `gizmo-math` (and thus a deliberate, documented `gizmo-math` bump).
//! Currently pinned to the `0.29` line.

pub mod aabb;
pub mod fixed;
pub mod frustum;
pub mod ray;
pub mod spatial;

/// The engine's vector-math vocabulary, re-exported **directly from `glam`**
/// (see the crate-level "Public dependency" note). This is the single source of
/// truth: `gizmo-math` does not depend on `bevy_math` for these types, so no
/// `bevy_reflect` is pulled into the Stage A production dependency tree.
pub use glam::{EulerRot, Mat3, Mat4, Quat, Vec2, Vec3, Vec3A, Vec4};

pub use aabb::Aabb;
pub use fixed::{Fp32, FpVec3};
pub use frustum::{Frustum, Intersection, Plane};
pub use ray::Ray;

/// Below this magnitude a denominator is treated as zero (degenerate) by [`safe_recip`].
pub const DEGENERATE_EPS: f32 = 1e-20;

/// Guarded reciprocal-divide for geometry code. Returns `Some(num / den)` when `den` is
/// safely non-zero, or `None` when `|den| < DEGENERATE_EPS` (a degenerate configuration —
/// collinear/coplanar simplex, zero-area triangle, parallel axes). Use it instead of a
/// bare `num / den` so a degenerate input yields a handled `None` rather than a NaN/inf
/// that silently poisons everything downstream (the GJK distance bug class).
#[inline]
pub fn safe_recip(num: f32, den: f32) -> Option<f32> {
    if den.abs() < DEGENERATE_EPS {
        None
    } else {
        Some(num / den)
    }
}

/// Normalizes `v`, returning `fallback` when `v` is too short to have a stable direction
/// (degenerate / zero vector). A guarded wrapper over glam's `try_normalize` so callers
/// don't reinvent the zero-length check (and never emit a NaN direction).
#[inline]
pub fn safe_normalize_or(v: Vec3, fallback: Vec3) -> Vec3 {
    v.try_normalize().unwrap_or(fallback)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_recip_guards_degenerate_denominator() {
        assert_eq!(safe_recip(1.0, 4.0), Some(0.25));
        assert_eq!(safe_recip(1.0, 0.0), None);
        assert_eq!(safe_recip(1.0, DEGENERATE_EPS * 0.5), None); // below threshold → None
        // never produces a non-finite result
        assert!(safe_recip(1.0, f32::MIN_POSITIVE).map_or(true, |r| r.is_finite()));
    }

    #[test]
    fn safe_normalize_or_handles_zero_vector() {
        let n = safe_normalize_or(Vec3::ZERO, Vec3::X);
        assert_eq!(n, Vec3::X, "zero vector must use the fallback, not NaN");
        let n2 = safe_normalize_or(Vec3::new(0.0, 3.0, 0.0), Vec3::X);
        assert!((n2 - Vec3::Y).length() < 1e-6);
    }

    #[test]
    fn ray_intersects_aabb_inside_frustum() {
        // Frustum: Camera at (0, 0, 5), looking at -Z (RH geometry)
        let view = Mat4::look_at_rh(Vec3::new(0.0, 0.0, 5.0), Vec3::ZERO, Vec3::Y);
        let proj = Mat4::perspective_rh(std::f32::consts::FRAC_PI_4, 1.0, 0.1, 100.0);
        let vp = proj * view;
        let frustum = Frustum::from_matrix(&vp);

        // Center AABB at origin
        let aabb = Aabb::new(Vec3::splat(-1.0), Vec3::splat(1.0));

        // Ensure AABB is cleanly within the camera frustum limits
        assert_eq!(frustum.test_aabb(aabb), Intersection::Inside);

        // Ray shooting exactly down the -Z axis from the camera position targeting the object
        let ray = Ray::new(Vec3::new(0.0, 0.0, 5.0), Vec3::new(0.0, 0.0, -1.0));

        // Math simulation verification: It should collide with the box
        let t = ray.intersect_aabb(aabb);
        assert!(t.is_some());

        let intersection_distance = t.unwrap();
        // Distance from camera Z=5 to AABB Front-Face Z=1 requires a travel distance of precisely 4 units
        assert!((intersection_distance - 4.0).abs() < 1e-5);
    }

    #[test]
    fn aabb_transform_then_frustum_cull() {
        // Frustum: Camera at (0, 0, 5), looking at -Z (RH bounds)
        let view = Mat4::look_at_rh(Vec3::new(0.0, 0.0, 5.0), Vec3::ZERO, Vec3::Y);
        let proj = Mat4::perspective_rh(std::f32::consts::FRAC_PI_4, 1.0, 0.1, 100.0);
        let vp = proj * view;
        let frustum = Frustum::from_matrix(&vp);

        // Default Unit AABB representing a local unscaled Model
        let local_aabb = Aabb::new(Vec3::splat(-0.5), Vec3::splat(0.5));

        // Scene Step 1: Object is pushed into the active view frustum
        let inside_mat = Mat4::from_translation(Vec3::new(0.0, 0.0, -10.0));
        let world_aabb_inside = local_aabb.transform(&inside_mat);
        assert_eq!(frustum.test_aabb(world_aabb_inside), Intersection::Inside);

        // Scene Step 2: Object is rotated and pushed way outside to the right of the visible frustum limits
        let outside_mat = Mat4::from_translation(Vec3::new(100.0, 0.0, 0.0));
        let world_aabb_outside = local_aabb.transform(&outside_mat);
        assert_eq!(frustum.test_aabb(world_aabb_outside), Intersection::Outside);
    }
}
