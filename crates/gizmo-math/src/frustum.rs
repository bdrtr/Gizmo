use crate::aabb::Aabb;
use glam::{DVec4, Mat4, Vec3A, Vec4};

// ---------------------------------------------------------------------------
// Intersection result
// ---------------------------------------------------------------------------

/// Result of classifying a volume against a frustum.
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
///
/// One plane is not normalized: [`Plane::UNBOUNDED`], which bounds nothing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plane {
    /// Unit-length plane normal — zero for [`Plane::UNBOUNDED`].
    pub normal: Vec3A,
    /// Signed distance from the origin along the normal.
    pub distance: f32,
}

impl Plane {
    /// A plane that bounds nothing: every point is on its positive side (zero normal, distance
    /// `+∞`). What a plane comes out as when its coefficients do not say where it is — see
    /// [`from_coefficients`](Self::from_coefficients) and [`Frustum::from_matrix`].
    ///
    /// A culling test must never invent a boundary. The fallback used to be `+Z, d = 0` — a real
    /// plane through the origin — so a frustum whose far plane came out degenerate culled every
    /// object with `z < 0`, which is half of any world.
    pub const UNBOUNDED: Self = Self { normal: Vec3A::ZERO, distance: f32::INFINITY };

    /// Constructs a plane from raw (possibly unnormalized) coefficients `(x, y, z, w)`.
    /// Normalizes so that `normal` is always unit-length.
    ///
    /// A zero (or non-finite) normal names no plane, and gives [`Plane::UNBOUNDED`]. Only that:
    /// a *short* normal is still a direction, and normalizing it in `f64` loses nothing. (This
    /// used to treat any `|normal|² < 1e-10` as degenerate — an absolute threshold on a quantity
    /// whose scale is the caller's. A view-projection's far plane has `|normal| = n / (f − n)`,
    /// which for a 0.1 m near and a 20 km far is 5·10⁻⁶: under the threshold, so that frustum
    /// lost its far plane to the fallback.)
    #[inline]
    pub fn from_coefficients(x: f32, y: f32, z: f32, w: f32) -> Self {
        Self::from_dvec4(Vec4::new(x, y, z, w).as_dvec4())
    }

    fn from_dvec4(c: DVec4) -> Self {
        let len = c.truncate().length();
        if !len.is_finite() || len == 0.0 || !c.w.is_finite() {
            return Self::UNBOUNDED;
        }
        Self {
            normal: Vec3A::new((c.x / len) as f32, (c.y / len) as f32, (c.z / len) as f32),
            distance: (c.w / len) as f32,
        }
    }

    /// The plane `a + sign·b` of two rows of an `f32` view-projection, or [`Plane::UNBOUNDED`]
    /// when the rows do not determine it.
    ///
    /// The rows' entries carry `f32` rounding, up to `2⁻²⁴` of each. When `a` and `b` nearly
    /// cancel, what is left of the normal can be mostly that rounding: a perspective's far plane
    /// is `row3 − row2`, two rows equal to within `n / (f − n)`. So the normal is kept only when it
    /// is [`RESOLVED`](Self::RESOLVED) times longer than the rounding it could hold — direction
    /// known to 0.1 % — and otherwise the plane culls nothing. That is conservative by
    /// construction (the GPU clips at that plane anyway); a plane known to a few percent is not:
    /// tilted by a degree over a 20 km far plane, it cuts hundreds of metres into the frustum.
    fn between(a: DVec4, b: DVec4, sign: f64) -> Self {
        /// Unit roundoff of `f32`.
        const ROUNDING: f64 = 1.0 / 16_777_216.0;
        let c = a + b * sign;
        let doubt = ROUNDING * (a.truncate().length() + b.truncate().length());
        if c.truncate().length() <= Self::RESOLVED * doubt {
            return Self::UNBOUNDED;
        }
        Self::from_dvec4(c)
    }

    /// How many times longer than its possible rounding a plane's normal must be for
    /// [`Frustum::from_matrix`] to use it. See [`between`](Self::between).
    const RESOLVED: f64 = 1000.0;

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
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frustum {
    /// `[left, right, bottom, top, near, far]` — normals point into the frustum, so a
    /// point inside is on the positive side of all six.
    pub planes: [Plane; 6],
}

impl Frustum {
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

        // In `f64`, each from the rows it is a sum of: the far plane is a difference of two rows
        // that can agree to within a few millionths, and `Plane::between` decides whether what
        // is left of them is a plane at all.
        let [r0, r1, r2, r3] = [r0, r1, r2, r3].map(Vec4::as_dvec4);
        Self {
            planes: [
                Plane::between(r3, r0, 1.0),  // Left:   w + x ≥ 0
                Plane::between(r3, r0, -1.0), // Right:  w - x ≥ 0
                Plane::between(r3, r1, 1.0),  // Bottom: w + y ≥ 0
                Plane::between(r3, r1, -1.0), // Top:    w - y ≥ 0
                Plane::between(DVec4::ZERO, r2, 1.0), // Near: z ≥ 0 (Z∈[0,1])
                Plane::between(r3, r2, -1.0), // Far:    w - z ≥ 0
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
    // OBB tests
    // -----------------------------------------------------------------------

    /// Returns `true` if an Oriented Bounding Box (OBB) intersects the frustum.
    ///
    /// The OBB is defined by its center, half-extents (from center to faces),
    /// and its rotation quaternion. This is a fast conservative test based on
    /// projecting the OBB extents onto each frustum plane normal.
    #[inline]
    pub fn intersects_obb(
        &self,
        center: impl Into<Vec3A>,
        half_extents: impl Into<Vec3A>,
        rotation: glam::Quat,
    ) -> bool {
        self.test_obb(center, half_extents, rotation) != Intersection::Outside
    }

    /// Full OBB–frustum classification: `Outside`, `Partial`, or `Inside`.
    #[inline]
    pub fn test_obb(
        &self,
        center: impl Into<Vec3A>,
        half_extents: impl Into<Vec3A>,
        rotation: glam::Quat,
    ) -> Intersection {
        let center = center.into();
        let extents = half_extents.into();

        // Extract local axes of the OBB from the rotation quaternion
        let rot_mat = glam::Mat3A::from_quat(rotation);
        let axis_x = rot_mat.x_axis;
        let axis_y = rot_mat.y_axis;
        let axis_z = rot_mat.z_axis;

        let mut all_inside = true;

        for plane in &self.planes {
            // Compute the projected "radius" of the OBB along the plane's normal
            let r = extents.x * plane.normal.dot(axis_x).abs()
                + extents.y * plane.normal.dot(axis_y).abs()
                + extents.z * plane.normal.dot(axis_z).abs();

            let d = plane.signed_distance(center);

            // If the OBB's center is further behind the plane than its projected radius,
            // the OBB is completely outside the frustum.
            if d < -r {
                return Intersection::Outside;
            }
            // If the center is within the radius distance of the plane, it crosses the boundary.
            if d < r {
                all_inside = false;
            }
        }

        if all_inside {
            Intersection::Inside
        } else {
            Intersection::Partial
        }
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

        if all_inside {
            Intersection::Inside
        } else {
            Intersection::Partial
        }
    }

    /// Returns `true` if the AABB is at least partially visible (not culled).
    ///
    /// Only the p-vertex test — the one that can say "outside". It used to go through
    /// [`test_aabb`](Self::test_aabb), which also tests every plane's n-vertex to tell `Inside`
    /// from `Partial`, an answer this question throws away: twice the plane work for the same
    /// bool, for the camera and each of four shadow cascades, per mesh, per frame. Same decision
    /// bit for bit (NaN included): the p-vertex comparison is unchanged, only the discarded half is
    /// gone. Not a measured frame-time win on its own — the draw path's cost was elsewhere (see
    /// `MotionHistory`) — just less work for the same answer.
    #[inline]
    pub fn intersects_aabb(&self, aabb: Aabb) -> bool {
        if aabb.is_empty() {
            return false;
        }
        // `!any(< 0)`, not `all(>= 0)`: the same predicate `test_aabb` uses, so a NaN distance
        // decides the same way it did.
        !self
            .planes
            .iter()
            .any(|plane| plane.signed_distance(plane.positive_vertex(aabb)) < 0.0)
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

        let result = if all_inside {
            Intersection::Inside
        } else {
            Intersection::Partial
        };
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
        // A zero normal names no plane: it bounds nothing rather than falling back to a real one.
        let p = Plane::from_coefficients(0.0, 0.0, 0.0, 1.0);
        assert_eq!(p, Plane::UNBOUNDED);
        for at in [Vec3A::new(0.0, 0.0, -1.0e6), Vec3A::new(3.0, -2.0, 1.0e6), Vec3A::ZERO] {
            assert!(p.signed_distance(at) >= 0.0);
        }
    }

    #[test]
    fn a_short_normal_is_still_a_plane() {
        // The far plane of a 0.1 m / 20 km perspective has |normal| ≈ 5e-6. It used to be taken
        // for degenerate (|n|² < 1e-10) and replaced by `+Z, d = 0`.
        let p = Plane::from_coefficients(0.0, 0.0, -5.0e-6, 0.1);
        assert!((p.normal - Vec3A::NEG_Z).length() < 1e-6, "{p:?}");
        assert!((p.distance - 20_000.0).abs() < 1.0, "{p:?}");
    }

    /// A camera with a 0.1 m near plane and a 20 km far plane sees what is in front of it.
    ///
    /// Its far plane is `row3 − row2` of the view-projection, two rows equal to within
    /// `n / (f − n) = 5·10⁻⁶`. That fell under the old absolute degeneracy threshold and became
    /// `+Z, d = 0`, so the frustum culled every box with `z < 0`: in `gizmo`'s own frame, 14 611 of
    /// a static city's 14 620 objects, all in plain view. Found measuring that city's frame cost
    /// (`a_static_city_frame_full_cpu_cost`, 2026-11-18).
    #[test]
    fn a_wide_depth_range_keeps_what_is_in_front() {
        let cam = Vec3::new(0.0, 60.0, 80.0);
        let forward = Vec3::new(0.0, -0.12, -1.0).normalize();
        for (near, far) in [(0.1, 20_000.0), (0.01, 100_000.0), (0.5, 20_000.0), (0.1, 2_000.0)] {
            let vp = Mat4::perspective_rh(1.2, 16.0 / 9.0, near, far) * Mat4::look_to_rh(cam, forward, Vec3::Y);
            let f = Frustum::from_matrix(&vp);
            for ahead in [10.0f32, 100.0, 1_000.0, 0.9 * far] {
                let at = cam + forward * ahead;
                let b = Aabb::new(at - Vec3::splat(1.0), at + Vec3::splat(1.0));
                assert!(f.intersects_aabb(b), "near {near}, far {far}: a box {ahead} m ahead was culled");
            }
            // And behind, beside and above it, still out.
            for off in [cam - forward * 50.0, cam + Vec3::new(5_000.0, 0.0, -10.0), cam + Vec3::new(0.0, 5_000.0, -10.0)] {
                let b = Aabb::new(off - Vec3::splat(1.0), off + Vec3::splat(1.0));
                assert!(!f.intersects_aabb(b), "near {near}, far {far}: a box at {off} was kept");
            }
        }
    }

    /// Where the depth range is narrow enough for the far plane to be known, it still culls.
    #[test]
    fn a_narrow_depth_range_still_culls_past_the_far_plane() {
        let vp = Mat4::perspective_rh(1.0, 1.0, 0.1, 100.0) * Mat4::look_to_rh(Vec3::ZERO, Vec3::NEG_Z, Vec3::Y);
        let f = Frustum::from_matrix(&vp);
        assert_ne!(f.planes()[5], Plane::UNBOUNDED);
        assert!(f.intersects_aabb(Aabb::new(Vec3::new(-1.0, -1.0, -91.0), Vec3::new(1.0, 1.0, -89.0))));
        assert!(!f.intersects_aabb(Aabb::new(Vec3::new(-1.0, -1.0, -151.0), Vec3::new(1.0, 1.0, -149.0))));
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
        // A point (min == max) has zero volume (is_degenerate() == true) but is
        // not strictly empty (is_empty() == false). Thus, the intersection test
        // proceeds and correctly identifies it as inside the frustum.
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

