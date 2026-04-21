use crate::aabb::Aabb;
use glam::{Mat4, Vec3A, Vec4, Vec4Swizzles};

// ---------------------------------------------------------------------------
// Intersection result
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intersection {
    /// Completely outside the frustum — safe to cull.
    Outside,
    /// Partially overlapping — cannot cull, must recurse or draw.
    Partial,
    /// Completely inside the frustum — children can skip plane tests.
    Inside,
}

// ---------------------------------------------------------------------------
// Plane
// ---------------------------------------------------------------------------

/// A normalized half-space plane: `normal · X + distance = 0`.
/// Points where `normal · X + distance > 0` are on the "positive" side.
#[derive(Debug, Clone, Copy)]
pub struct Plane {
    /// Unit-length outward normal.
    pub normal: Vec3A,
    /// Signed distance from the origin along the normal.
    pub distance: f32,
}

impl Plane {
    /// Constructs a plane from raw (possibly unnormalized) coefficients `(x, y, z, w)`.
    /// Normalizes so that `normal` is always unit-length.
    /// Falls back to `+Z, d=0` if the normal is degenerate.
    #[inline]
    pub fn from_coefficients(x: f32, y: f32, z: f32, w: f32) -> Self {
        let len_sq = x * x + y * y + z * z;
        if len_sq < 1e-10 {
            return Self { normal: Vec3A::Z, distance: 0.0 };
        }
        let inv_len = len_sq.sqrt().recip();
        Self {
            normal: Vec3A::new(x * inv_len, y * inv_len, z * inv_len),
            distance: w * inv_len,
        }
    }

    /// Constructs a plane directly from a `Vec4` row of a VP matrix.
    #[inline]
    fn from_vec4(v: Vec4) -> Self {
        Self::from_coefficients(v.x, v.y, v.z, v.w)
    }

    /// Signed distance from the plane to `pt`.
    /// Positive = in front of (positive side of) the plane.
    #[inline]
    pub fn signed_distance(self, pt: Vec3A) -> f32 {
        self.normal.dot(pt) + self.distance
    }

    /// Returns the "positive vertex" of the AABB — the corner furthest along
    /// the plane normal. Used in conservative AABB–frustum tests.
    #[inline]
    fn positive_vertex(self, aabb: Aabb) -> Vec3A {
        Vec3A::select(self.normal.cmpgt(Vec3A::ZERO), aabb.max, aabb.min)
    }

    /// Returns the "negative vertex" of the AABB — the corner furthest
    /// against the plane normal.
    #[inline]
    fn negative_vertex(self, aabb: Aabb) -> Vec3A {
        Vec3A::select(self.normal.cmpgt(Vec3A::ZERO), aabb.min, aabb.max)
    }
}

// ---------------------------------------------------------------------------
// Frustum
// ---------------------------------------------------------------------------

/// Six-plane view frustum extracted from a combined Projection × View matrix.
///
/// Plane extraction follows Gribb & Hartmann (2001) and works for both
/// right-handed and left-handed projection conventions.
///
/// NDC convention: WGPU / Vulkan / DX12 (Z ∈ [0, 1]).
/// For OpenGL (Z ∈ [−1, 1]) swap the near-plane extraction (see comments).
#[derive(Debug, Clone, Copy)]
pub struct Frustum {
    /// `[left, right, bottom, top, near, far]` — all outward-facing.
    planes: [Plane; 6],
}

impl Frustum {
    // Plane index constants for clarity
    const LEFT: usize   = 0;
    const RIGHT: usize  = 1;
    const BOTTOM: usize = 2;
    const TOP: usize    = 3;
    const NEAR: usize   = 4;
    const FAR: usize    = 5;

    /// Extracts the frustum planes from a Projection × View (VP) matrix.
    ///
    /// Uses the Gribb–Hartmann method: each plane is a linear combination of
    /// the matrix rows, requiring no trigonometry and working with any
    /// well-formed projection.
    ///
    /// **NDC assumed:** Z ∈ [0, 1] (WGPU, Vulkan, DirectX).  
    /// For OpenGL Z ∈ [−1, 1], change the near plane to `r3 + r2`.
    #[inline]
    pub fn from_matrix(vp: &Mat4) -> Self {
        let r0 = vp.row(0); // X column in clip space
        let r1 = vp.row(1); // Y column
        let r2 = vp.row(2); // Z column
        let r3 = vp.row(3); // W column (homogeneous)

        Self {
            planes: [
                Plane::from_vec4(r3 + r0), // Left:   w + x ≥ 0
                Plane::from_vec4(r3 - r0), // Right:  w - x ≥ 0
                Plane::from_vec4(r3 + r1), // Bottom: w + y ≥ 0
                Plane::from_vec4(r3 - r1), // Top:    w - y ≥ 0
                Plane::from_vec4(r2),      // Near:   z ≥ 0        (Z∈[0,1])
                // Near (OpenGL Z∈[−1,1]): Plane::from_vec4(r3 + r2)
                Plane::from_vec4(r3 - r2), // Far:    w - z ≥ 0
            ],
        }
    }

    /// Returns the six frustum planes as a slice.
    #[inline]
    pub fn planes(&self) -> &[Plane; 6] {
        &self.planes
    }

    // -----------------------------------------------------------------------
    // Sphere test
    // -----------------------------------------------------------------------

    /// Tests whether a sphere (center + radius) is visible.
    ///
    /// Culls if the sphere center is more than `radius` behind any plane.
    /// This is a conservative test — no false negatives, some false positives
    /// near corners.
    #[inline]
    pub fn intersects_sphere(&self, center: impl Into<Vec3A>, radius: f32) -> bool {
        let c = center.into();
        self.planes.iter().all(|p| p.signed_distance(c) >= -radius)
    }

    // -----------------------------------------------------------------------
    // AABB tests
    // -----------------------------------------------------------------------

    /// Full AABB–frustum classification: `Outside`, `Partial`, or `Inside`.
    ///
    /// Uses the positive/negative vertex (p/n-vertex) method — 2 dot products
    /// per plane instead of 8 corner transforms.
    ///
    /// - **Outside**: positive vertex is behind any plane → fully culled.
    /// - **Inside**: negative vertex is in front of every plane → fully contained.
    /// - **Partial**: otherwise.
    #[inline]
    pub fn test_aabb(&self, aabb: Aabb) -> Intersection {
        if aabb.is_empty() {
            return Intersection::Outside;
        }

        let mut all_inside = true;

        for plane in &self.planes {
            // p-vertex: the corner most along the normal.
            // If even this corner is behind the plane → completely outside.
            if plane.signed_distance(plane.positive_vertex(aabb)) < 0.0 {
                return Intersection::Outside;
            }
            // n-vertex: the corner most against the normal.
            // If it is behind the plane → not fully inside.
            if plane.signed_distance(plane.negative_vertex(aabb)) < 0.0 {
                all_inside = false;
            }
        }

        if all_inside { Intersection::Inside } else { Intersection::Partial }
    }

    /// Returns `true` if the AABB is at least partially visible (not culled).
    #[inline]
    pub fn intersects_aabb(&self, aabb: Aabb) -> bool {
        self.test_aabb(aabb) != Intersection::Outside
    }

    // -----------------------------------------------------------------------
    // BVH / hierarchical culling helpers
    // -----------------------------------------------------------------------

    /// Faster AABB visibility test that skips planes that the parent already
    /// passed (used in BVH traversal with plane masking).
    ///
    /// `plane_mask` is a bitmask of the 6 planes to test (bit `i` = plane `i`).
    /// Planes already known to fully contain the parent can be masked out.
    ///
    /// Returns `(Intersection, out_mask)` where `out_mask` can be passed to
    /// child nodes — planes where the child is fully inside are cleared.
    #[inline]
    pub fn test_aabb_masked(&self, aabb: Aabb, plane_mask: u8) -> (Intersection, u8) {
        if aabb.is_empty() {
            return (Intersection::Outside, 0);
        }

        let mut all_inside = true;
        let mut out_mask = 0u8;

        for (i, plane) in self.planes.iter().enumerate() {
            let bit = 1u8 << i;
            if plane_mask & bit == 0 {
                // Parent was already fully inside this plane; skip.
                continue;
            }
            if plane.signed_distance(plane.positive_vertex(aabb)) < 0.0 {
                return (Intersection::Outside, 0);
            }
            if plane.signed_distance(plane.negative_vertex(aabb)) < 0.0 {
                all_inside = false;
                out_mask |= bit; // Still need to test children against this plane.
            }
            // else: fully inside this plane — don't propagate to children.
        }

        let result = if all_inside { Intersection::Inside } else { Intersection::Partial };
        (result, out_mask)
    }

    /// Full plane mask — test all 6 planes (use as the root mask for BVH traversal).
    pub const FULL_MASK: u8 = 0b0011_1111;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::{Frustum, Intersection, Plane};
    use crate::aabb::Aabb;
    use glam::{Mat4, Vec3, Vec3A};

    // Shared camera setup: eye at Z=8, looking towards -Z.
    fn make_frustum() -> Frustum {
        let view = Mat4::look_at_rh(Vec3::new(0.0, 0.0, 8.0), Vec3::ZERO, Vec3::Y);
        let proj = Mat4::perspective_rh(std::f32::consts::FRAC_PI_4, 1.0, 0.1, 100.0);
        Frustum::from_matrix(&(proj * view))
    }

    // --- Plane tests ---

    #[test]
    fn plane_signed_distance() {
        // XY plane (normal +Z, d = 0)
        let p = Plane::from_coefficients(0.0, 0.0, 1.0, 0.0);
        assert!((p.signed_distance(Vec3A::new(0.0, 0.0, 5.0)) - 5.0).abs() < 1e-5);
        assert!((p.signed_distance(Vec3A::new(0.0, 0.0, -3.0)) + 3.0).abs() < 1e-5);
    }

    #[test]
    fn plane_degenerate_normal() {
        // Zero normal should not panic, falls back to Z
        let p = Plane::from_coefficients(0.0, 0.0, 0.0, 1.0);
        assert!((p.normal - Vec3A::Z).length() < 1e-5);
    }

    // --- Visibility ---

    #[test]
    fn aabb_fully_inside() {
        let frustum = make_frustum();
        let cube = Aabb::new(Vec3::splat(-0.5), Vec3::splat(0.5));
        assert_eq!(frustum.test_aabb(cube), Intersection::Inside);
    }

    #[test]
    fn aabb_behind_camera_outside() {
        let frustum = make_frustum();
        // Camera at Z=8 looking toward -Z; Z > 8 is behind near plane.
        let behind = Aabb::new(Vec3::new(-1.0, -1.0, 10.0), Vec3::new(1.0, 1.0, 12.0));
        assert_eq!(frustum.test_aabb(behind), Intersection::Outside);
    }

    #[test]
    fn aabb_beyond_far_plane_outside() {
        let frustum = make_frustum();
        // Far plane at depth 100 from camera (Z = 8 − 100 = −92).
        let far = Aabb::new(Vec3::new(-1.0, -1.0, -105.0), Vec3::new(1.0, 1.0, -95.0));
        assert_eq!(frustum.test_aabb(far), Intersection::Outside);
    }

    #[test]
    fn aabb_enclosing_frustum_is_partial() {
        let frustum = make_frustum();
        let huge = Aabb::new(Vec3::splat(-1000.0), Vec3::splat(1000.0));
        assert_eq!(frustum.test_aabb(huge), Intersection::Partial);
    }

    #[test]
    fn aabb_degenerate_point_inside() {
        let frustum = make_frustum();
        let pt = Aabb::new(Vec3::ZERO, Vec3::ZERO);
        assert_eq!(frustum.test_aabb(pt), Intersection::Inside);
    }

    #[test]
    fn aabb_degenerate_point_outside() {
        let frustum = make_frustum();
        let pt = Aabb::new(Vec3::splat(1000.0), Vec3::splat(1000.0));
        assert_eq!(frustum.test_aabb(pt), Intersection::Outside);
    }

    #[test]
    fn aabb_empty_is_outside() {
        let frustum = make_frustum();
        assert_eq!(frustum.test_aabb(Aabb::empty()), Intersection::Outside);
    }

    #[test]
    fn intersects_aabb_convenience() {
        let frustum = make_frustum();
        let inside = Aabb::new(Vec3::splat(-0.5), Vec3::splat(0.5));
        let outside = Aabb::new(Vec3::new(-1.0, -1.0, 10.0), Vec3::new(1.0, 1.0, 12.0));
        assert!(frustum.intersects_aabb(inside));
        assert!(!frustum.intersects_aabb(outside));
    }

    // --- Sphere tests ---

    #[test]
    fn sphere_inside_frustum() {
        let frustum = make_frustum();
        assert!(frustum.intersects_sphere(Vec3::ZERO, 0.5));
    }

    #[test]
    fn sphere_outside_frustum() {
        let frustum = make_frustum();
        // Center well behind near plane, radius too small to reach
        assert!(!frustum.intersects_sphere(Vec3::new(0.0, 0.0, 50.0), 0.1));
    }

    #[test]
    fn sphere_straddles_plane() {
        let frustum = make_frustum();
        // Large sphere centered behind camera but radius large enough to overlap
        assert!(frustum.intersects_sphere(Vec3::new(0.0, 0.0, 9.0), 5.0));
    }

    // --- Masked test ---

    #[test]
    fn masked_test_skip_all_planes() {
        let frustum = make_frustum();
        // mask = 0: skip all planes → result is always Inside
        let outside = Aabb::new(Vec3::splat(1000.0), Vec3::splat(2000.0));
        let (result, _) = frustum.test_aabb_masked(outside, 0);
        assert_eq!(result, Intersection::Inside, "zero mask skips all tests");
    }

    #[test]
    fn masked_test_full_mask_matches_unmasked() {
        let frustum = make_frustum();
        let cube = Aabb::new(Vec3::splat(-0.5), Vec3::splat(0.5));
        let unmasked = frustum.test_aabb(cube);
        let (masked, _) = frustum.test_aabb_masked(cube, Frustum::FULL_MASK);
        assert_eq!(unmasked, masked);
    }
}