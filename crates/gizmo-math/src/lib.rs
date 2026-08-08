#![warn(missing_docs)]
//! (`missing_docs` is a RATCHET, not a suggestion. The CI lint gate runs with `-D warnings`,
//! so every public item in this crate must carry a doc comment or the build fails. This crate
//! is Stage A — the dependency-light core that goes to 1.x first — and its documented surface
//! is part of that promise. Do not silence this with `#[allow]`; write the doc.)

//! # Gizmo Math
//!
//! Houses Gizmo Engine's fundamental maths infrastructure and its render/physics data types.
//!
//! ## Conventions
//! - **Coordinate System**: Right-Handed (RH).
//! - **Up Axis**: Y-Up (0.0, 1.0, 0.0).
//! - **Forward Axis**: -Z (the camera always looks towards the negative Z axis).
//! - **Matrix Layout**: Column-Major (compatible with the glam architecture).
//!
//! Houses `Mat3`, structured for normal-matrix computations, and the size-optimised
//! `Aabb`, `Frustum`, `Ray` structures for 3D space-bounds computations.
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
//! Currently pinned to the `0.32` line.

/// Axis-aligned bounding boxes — the engine's universal "cheap bound".
///
/// Lives down here in `gizmo-math` because the same [`Aabb`] type is used by both the
/// physics and the rendering crates rather than belonging to any one consumer.
pub mod aabb;

/// **Experimental** Q16.16 fixed-point arithmetic ([`Fp32`], [`FpVec3`]). **The
/// simulation does not use it.**
///
/// Physics state (transforms, velocities, the solver) runs entirely on `glam`/`f32`;
/// the determinism guarantee this engine actually ships is *same-platform* replay and
/// rollback bit-equality, not cross-platform bit-exactness. This module is groundwork
/// for a possible future lock-step / cross-platform-deterministic mode and is currently
/// referenced by nothing outside `gizmo-math` itself. It has unit tests but no coverage
/// from the physics soak/determinism gates — treat it as a sketch, not production maths.
pub mod fixed;

/// View-frustum plane extraction and volume classification for culling.
///
/// Produces six half-spaces from a `Projection × View` matrix (Gribb–Hartmann) and
/// classifies an [`Aabb`] as `Inside` / `Partial` / `Outside`.
/// The three-way answer is the point: a hierarchy node that comes back `Inside` lets
/// the whole subtree skip plane tests. Assumes WGPU/Vulkan/D3D NDC depth (Z ∈ [0, 1]).
///
/// The plane normals point *into* the frustum, so a visible point has a non-negative
/// [`Plane::signed_distance`] against all six.
pub mod frustum;

/// Ray casting against analytic primitives (AABB, OBB, triangle).
///
/// This is the *rendering/tooling* ray — `Vec3A`-based, with an NDC unprojection
/// constructor for camera picking. The physics scene queries use their own `Ray` type in
/// `gizmo-physics-core` (`Vec3`-based, different degenerate-direction fallback); the
/// two are deliberately separate and must not be assumed interchangeable.
pub mod ray;

/// Plücker (6-D "spatial") vectors, matrices and inertias for Featherstone-style
/// articulated-body dynamics.
///
/// **Consumed only by the experimental `experimental-multibody` feature of
/// `gizmo-physics-rigid`** (the ABA solver). The mainline rigid-body pipeline —
/// broadphase, narrowphase, TGS-Soft sequential impulses — never touches these types,
/// so changes here cannot perturb the `headless_stress_test` determinism gate.
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
        assert!(safe_recip(1.0, f32::MIN_POSITIVE).is_none_or(|r| r.is_finite()));
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
