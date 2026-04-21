use glam::{Mat4, Vec3A, Vec4Swizzles};
use std::f32;

/// Axis-Aligned Bounding Box (AABB) represented by min/max corners.
///
/// Uses `Vec3A` (16-byte aligned SIMD vector) for performance.
/// An "empty" AABB has min = +INF, max = -INF and represents no volume.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb {
    pub min: Vec3A,
    pub max: Vec3A,
}

impl Aabb {
    // -----------------------------------------------------------------------
    // Constructors
    // -----------------------------------------------------------------------

    /// Creates an AABB from explicit min/max corners.
    /// Caller is responsible for ensuring `min <= max` on all axes.
    #[inline]
    pub fn new(min: impl Into<Vec3A>, max: impl Into<Vec3A>) -> Self {
        Self {
            min: min.into(),
            max: max.into(),
        }
    }

    /// Creates the canonical empty/invalid AABB (min = +INF, max = -INF).
    #[inline]
    pub fn empty() -> Self {
        Self {
            min: Vec3A::splat(f32::INFINITY),
            max: Vec3A::splat(f32::NEG_INFINITY),
        }
    }

    /// Creates an AABB from a center point and half-extents.
    #[inline]
    pub fn from_center_half_extents(center: impl Into<Vec3A>, half_extents: impl Into<Vec3A>) -> Self {
        let c = center.into();
        let h = half_extents.into();
        Self {
            min: c - h,
            max: c + h,
        }
    }

    /// Creates an AABB that contains all the provided points.
    /// Returns `Aabb::empty()` if the iterator is empty.
    #[inline]
    pub fn from_points(points: impl IntoIterator<Item = impl Into<Vec3A>>) -> Self {
        let mut aabb = Self::empty();
        for p in points {
            aabb.extend(p);
        }
        aabb
    }

    // -----------------------------------------------------------------------
    // State queries
    // -----------------------------------------------------------------------

    /// Returns `true` if the AABB has no valid volume (min > max on any axis).
    #[inline]
    pub fn is_empty(self) -> bool {
        self.min.x > self.max.x || self.min.y > self.max.y || self.min.z > self.max.z
    }

    /// Returns `true` if the AABB has a valid volume.
    #[inline]
    pub fn is_valid(self) -> bool {
        !self.is_empty()
    }

    // -----------------------------------------------------------------------
    // Geometric properties
    // -----------------------------------------------------------------------

    /// Returns the center of the AABB.
    #[inline]
    pub fn center(self) -> Vec3A {
        (self.min + self.max) * 0.5
    }

    /// Returns the half-extents (half the size on each axis).
    #[inline]
    pub fn half_extents(self) -> Vec3A {
        (self.max - self.min) * 0.5
    }

    /// Returns the full size (extent) on each axis.
    #[inline]
    pub fn size(self) -> Vec3A {
        self.max - self.min
    }

    /// Returns the total volume of the AABB.
    /// Returns `0.0` for an empty AABB.
    #[inline]
    pub fn volume(self) -> f32 {
        if self.is_empty() {
            return 0.0;
        }
        let s = self.size();
        s.x * s.y * s.z
    }

    /// Returns the surface area of the AABB.
    /// Useful for SAH-based BVH construction.
    /// Returns `0.0` for an empty AABB.
    #[inline]
    pub fn surface_area(self) -> f32 {
        if self.is_empty() {
            return 0.0;
        }
        let s = self.size();
        2.0 * (s.x * s.y + s.y * s.z + s.z * s.x)
    }

    /// Returns the length of the diagonal of the AABB.
    #[inline]
    pub fn diagonal(self) -> f32 {
        self.size().length()
    }

    // -----------------------------------------------------------------------
    // Point / containment queries
    // -----------------------------------------------------------------------

    /// Returns `true` if the point lies inside or on the boundary of this AABB.
    #[inline]
    pub fn contains_point(self, pt: impl Into<Vec3A>) -> bool {
        let p = pt.into();
        p.cmpge(self.min).all() && p.cmple(self.max).all()
    }

    /// Returns `true` if `other` is entirely contained within this AABB.
    #[inline]
    pub fn contains_aabb(self, other: Self) -> bool {
        if self.is_empty() || other.is_empty() {
            return false;
        }
        other.min.cmpge(self.min).all() && other.max.cmple(self.max).all()
    }

    /// Returns the closest point on (or inside) the AABB to the given point.
    #[inline]
    pub fn closest_point(self, pt: impl Into<Vec3A>) -> Vec3A {
        pt.into().clamp(self.min, self.max)
    }

    /// Returns the squared distance from the point to the AABB surface.
    /// Returns `0.0` if the point is inside the AABB.
    #[inline]
    pub fn distance_sq_to_point(self, pt: impl Into<Vec3A>) -> f32 {
        let p = pt.into();
        let closest = self.closest_point(p);
        (p - closest).length_squared()
    }

    /// Returns the distance from the point to the AABB surface.
    /// Returns `0.0` if the point is inside the AABB.
    #[inline]
    pub fn distance_to_point(self, pt: impl Into<Vec3A>) -> f32 {
        self.distance_sq_to_point(pt).sqrt()
    }

    // -----------------------------------------------------------------------
    // Intersection / overlap queries
    // -----------------------------------------------------------------------

    /// Returns `true` if this AABB overlaps `other`.
    /// Edge/face-touching counts as intersection.
    #[inline]
    pub fn intersects(self, other: Self) -> bool {
        if self.is_empty() || other.is_empty() {
            return false;
        }
        self.min.cmple(other.max).all() && self.max.cmpge(other.min).all()
    }

    /// Returns `true` if this AABB strictly overlaps `other`.
    /// Edge/face-touching does NOT count as intersection.
    #[inline]
    pub fn intersects_exclusive(self, other: Self) -> bool {
        if self.is_empty() || other.is_empty() {
            return false;
        }
        self.min.cmplt(other.max).all() && self.max.cmpgt(other.min).all()
    }

    /// Returns the intersection volume as an `Aabb`, or `None` if they do not overlap.
    /// Edge/face-touching yields a degenerate AABB (zero volume on at least one axis).
    #[inline]
    pub fn intersection(self, other: Self) -> Option<Self> {
        if !self.intersects(other) {
            return None;
        }
        Some(Self {
            min: self.min.max(other.min),
            max: self.max.min(other.max),
        })
    }

    // -----------------------------------------------------------------------
    // Modification / combination
    // -----------------------------------------------------------------------

    /// Expands this AABB to include the given point.
    #[inline]
    pub fn extend(&mut self, pt: impl Into<Vec3A>) {
        let p = pt.into();
        self.min = self.min.min(p);
        self.max = self.max.max(p);
    }

    /// Returns a new AABB that is the union of `self` and `other`.
    #[inline]
    pub fn merge(self, other: Self) -> Self {
        Self {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
        }
    }

    /// Returns a new AABB expanded (inflated) outward by `amount` on every face.
    /// A negative `amount` shrinks the AABB. If shrinking causes inversion, returns empty.
    #[inline]
    pub fn expand(self, amount: f32) -> Self {
        let delta = Vec3A::splat(amount);
        let new_min = self.min - delta;
        let new_max = self.max + delta;
        if new_min.cmpgt(new_max).any() {
            Self::empty()
        } else {
            Self { min: new_min, max: new_max }
        }
    }

    // -----------------------------------------------------------------------
    // Transformation
    // -----------------------------------------------------------------------

    /// Transforms this AABB by a `Mat4`, returning a new world-space AABB.
    ///
    /// Uses Arvo's method: applies the 3×3 rotation/scale part column-by-column,
    /// so only 3 iterations instead of 8 corner transforms.
    ///
    /// Reference: James Arvo, "Transforming Axis-Aligned Bounding Boxes",
    /// Graphics Gems, 1990.
    #[inline]
    pub fn transform(self, mat: &Mat4) -> Self {
        if self.is_empty() {
            return self;
        }

        // Translation goes directly into both min and max.
        let translation = Vec3A::from(mat.w_axis.xyz());
        let mut new_min = translation;
        let mut new_max = translation;

        // For each column of the 3×3 upper-left submatrix, accumulate the
        // interval contribution to [new_min, new_max].
        let cols = [
            Vec3A::from(mat.x_axis.xyz()),
            Vec3A::from(mat.y_axis.xyz()),
            Vec3A::from(mat.z_axis.xyz()),
        ];
        let bounds = [self.min, self.max];

        for (col_idx, col) in cols.iter().enumerate() {
            // Pick which original bound (min or max) contributes positively.
            for axis in 0..3 {
                let col_val = col[axis];
                let (lo, hi) = if col_val >= 0.0 {
                    (bounds[0][col_idx] * col_val, bounds[1][col_idx] * col_val)
                } else {
                    (bounds[1][col_idx] * col_val, bounds[0][col_idx] * col_val)
                };
                new_min[axis] += lo;
                new_max[axis] += hi;
            }
        }

        Self { min: new_min, max: new_max }
    }

    // -----------------------------------------------------------------------
    // Corners
    // -----------------------------------------------------------------------

    /// Returns all 8 corners of the AABB in a fixed order.
    /// Order: --- -+- +-- ++- --+ -++ +-+ +++  (xyz sign pattern)
    #[inline]
    pub fn corners(self) -> [Vec3A; 8] {
        let (mn, mx) = (self.min, self.max);
        [
            Vec3A::new(mn.x, mn.y, mn.z),
            Vec3A::new(mn.x, mx.y, mn.z),
            Vec3A::new(mx.x, mn.y, mn.z),
            Vec3A::new(mx.x, mx.y, mn.z),
            Vec3A::new(mn.x, mn.y, mx.z),
            Vec3A::new(mn.x, mx.y, mx.z),
            Vec3A::new(mx.x, mn.y, mx.z),
            Vec3A::new(mx.x, mx.y, mx.z),
        ]
    }
}

// ---------------------------------------------------------------------------
// Trait implementations
// ---------------------------------------------------------------------------

impl Default for Aabb {
    #[inline]
    fn default() -> Self {
        Self::empty()
    }
}

impl std::fmt::Display for Aabb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Aabb(min: [{:.3}, {:.3}, {:.3}], max: [{:.3}, {:.3}, {:.3}])",
            self.min.x, self.min.y, self.min.z,
            self.max.x, self.max.y, self.max.z,
        )
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    const EPS: f32 = 1e-5;

    fn approx_eq(a: Vec3A, b: Vec3A) -> bool {
        (a - b).length() < EPS
    }

    // --- Constructors ---

    #[test]
    fn test_empty() {
        let a = Aabb::empty();
        assert!(a.is_empty());
        assert!(!a.is_valid());
    }

    #[test]
    fn test_from_center_half_extents() {
        let a = Aabb::from_center_half_extents(Vec3::ZERO, Vec3::ONE);
        assert!(approx_eq(a.min, Vec3A::splat(-1.0)));
        assert!(approx_eq(a.max, Vec3A::splat(1.0)));
        assert!(approx_eq(a.center(), Vec3A::ZERO));
    }

    #[test]
    fn test_from_points_empty() {
        let a = Aabb::from_points(std::iter::empty::<Vec3A>());
        assert!(a.is_empty());
    }

    #[test]
    fn test_from_points() {
        let pts = [
            Vec3A::new(1.0, -2.0, 3.0),
            Vec3A::new(-1.0, 4.0, 0.0),
        ];
        let a = Aabb::from_points(pts);
        assert!(approx_eq(a.min, Vec3A::new(-1.0, -2.0, 0.0)));
        assert!(approx_eq(a.max, Vec3A::new(1.0, 4.0, 3.0)));
    }

    // --- Geometric properties ---

    #[test]
    fn test_center_size_half_extents() {
        let a = Aabb::new(Vec3::new(0.0, 0.0, 0.0), Vec3::new(4.0, 6.0, 8.0));
        assert!(approx_eq(a.center(), Vec3A::new(2.0, 3.0, 4.0)));
        assert!(approx_eq(a.size(), Vec3A::new(4.0, 6.0, 8.0)));
        assert!(approx_eq(a.half_extents(), Vec3A::new(2.0, 3.0, 4.0)));
    }

    #[test]
    fn test_volume() {
        let a = Aabb::new(Vec3::ZERO, Vec3::new(2.0, 3.0, 4.0));
        assert!((a.volume() - 24.0).abs() < EPS);
        assert!((Aabb::empty().volume()).abs() < EPS);
    }

    #[test]
    fn test_surface_area() {
        let a = Aabb::new(Vec3::ZERO, Vec3::new(1.0, 2.0, 3.0));
        // 2*(1*2 + 2*3 + 3*1) = 2*(2+6+3) = 22
        assert!((a.surface_area() - 22.0).abs() < EPS);
    }

    // --- Containment ---

    #[test]
    fn test_contains_point() {
        let a = Aabb::new(Vec3::ZERO, Vec3::ONE);
        assert!(a.contains_point(Vec3A::splat(0.5)));
        assert!(a.contains_point(Vec3A::ZERO));   // on boundary
        assert!(a.contains_point(Vec3A::ONE));    // on boundary
        assert!(!a.contains_point(Vec3A::splat(1.1)));
        assert!(!a.contains_point(Vec3A::splat(-0.1)));
    }

    #[test]
    fn test_contains_aabb() {
        let outer = Aabb::new(Vec3::ZERO, Vec3::splat(10.0));
        let inner = Aabb::new(Vec3::ONE, Vec3::splat(9.0));
        assert!(outer.contains_aabb(inner));
        assert!(!inner.contains_aabb(outer));
    }

    #[test]
    fn test_closest_point_inside() {
        let a = Aabb::new(Vec3::ZERO, Vec3::ONE);
        let p = Vec3A::splat(0.5);
        assert!(approx_eq(a.closest_point(p), p)); // inside → same point
    }

    #[test]
    fn test_closest_point_outside() {
        let a = Aabb::new(Vec3::ZERO, Vec3::ONE);
        let p = Vec3A::new(2.0, 0.5, 0.5);
        assert!(approx_eq(a.closest_point(p), Vec3A::new(1.0, 0.5, 0.5)));
    }

    #[test]
    fn test_distance_sq_to_point() {
        let a = Aabb::new(Vec3::ZERO, Vec3::ONE);
        assert!((a.distance_sq_to_point(Vec3A::splat(0.5))).abs() < EPS); // inside
        assert!((a.distance_sq_to_point(Vec3A::new(2.0, 0.5, 0.5)) - 1.0).abs() < EPS);
    }

    // --- Intersection ---

    #[test]
    fn test_intersects_overlapping() {
        let a = Aabb::new(Vec3::ZERO, Vec3::splat(2.0));
        let b = Aabb::new(Vec3::ONE, Vec3::splat(3.0));
        assert!(a.intersects(b));
        assert!(b.intersects(a));
    }

    #[test]
    fn test_intersects_disjoint() {
        let a = Aabb::new(Vec3::ZERO, Vec3::splat(1.0));
        let b = Aabb::new(Vec3::splat(2.0), Vec3::splat(3.0));
        assert!(!a.intersects(b));
    }

    #[test]
    fn test_intersects_edge_touching() {
        let a = Aabb::new(Vec3::ZERO, Vec3::splat(1.0));
        let b = Aabb::new(Vec3::new(1.0, 0.0, 0.0), Vec3::new(2.0, 1.0, 1.0));
        assert!(a.intersects(b));          // inclusive: edge touch = true
        assert!(!a.intersects_exclusive(b)); // exclusive: edge touch = false
    }

    #[test]
    fn test_intersects_empty() {
        let a = Aabb::new(Vec3::ZERO, Vec3::ONE);
        let e = Aabb::empty();
        assert!(!a.intersects(e));
        assert!(!e.intersects(a));
    }

    #[test]
    fn test_intersection_volume() {
        let a = Aabb::new(Vec3::ZERO, Vec3::splat(2.0));
        let b = Aabb::new(Vec3::ONE, Vec3::splat(3.0));
        let i = a.intersection(b).expect("should intersect");
        assert!(approx_eq(i.min, Vec3A::ONE));
        assert!(approx_eq(i.max, Vec3A::splat(2.0)));
    }

    #[test]
    fn test_intersection_none() {
        let a = Aabb::new(Vec3::ZERO, Vec3::ONE);
        let b = Aabb::new(Vec3::splat(2.0), Vec3::splat(3.0));
        assert!(a.intersection(b).is_none());
    }

    // --- Modification ---

    #[test]
    fn test_extend() {
        let mut a = Aabb::new(Vec3::ZERO, Vec3::splat(2.0));
        a.extend(Vec3A::new(3.0, -1.0, 1.0));
        assert!(approx_eq(a.min, Vec3A::new(0.0, -1.0, 0.0)));
        assert!(approx_eq(a.max, Vec3A::new(3.0, 2.0, 2.0)));
    }

    #[test]
    fn test_extend_empty() {
        let mut a = Aabb::empty();
        let pt = Vec3A::new(-5.0, 10.0, 3.0);
        a.extend(pt);
        assert!(approx_eq(a.min, pt));
        assert!(approx_eq(a.max, pt));

        let pt2 = Vec3A::new(5.0, -10.0, 6.0);
        a.extend(pt2);
        assert!(approx_eq(a.min, Vec3A::new(-5.0, -10.0, 3.0)));
        assert!(approx_eq(a.max, Vec3A::new(5.0, 10.0, 6.0)));
    }

    #[test]
    fn test_merge() {
        let a = Aabb::new(Vec3::ZERO, Vec3::ONE);
        let b = Aabb::new(Vec3::splat(-1.0), Vec3::splat(0.5));
        let m = a.merge(b);
        assert!(approx_eq(m.min, Vec3A::splat(-1.0)));
        assert!(approx_eq(m.max, Vec3A::ONE));
    }

    #[test]
    fn test_expand() {
        let a = Aabb::new(Vec3::ZERO, Vec3::splat(2.0));
        let expanded = a.expand(1.0);
        assert!(approx_eq(expanded.min, Vec3A::splat(-1.0)));
        assert!(approx_eq(expanded.max, Vec3A::splat(3.0)));

        let shrunk = a.expand(-0.5);
        assert!(approx_eq(shrunk.min, Vec3A::splat(0.5)));
        assert!(approx_eq(shrunk.max, Vec3A::splat(1.5)));

        // Over-shrink → empty
        let over = a.expand(-10.0);
        assert!(over.is_empty());
    }

    // --- Transform ---

    #[test]
    fn test_transform_identity() {
        let a = Aabb::new(Vec3::new(1.0, 2.0, 3.0), Vec3::new(4.0, 5.0, 6.0));
        let result = a.transform(&Mat4::IDENTITY);
        assert!(approx_eq(result.min, a.min));
        assert!(approx_eq(result.max, a.max));
    }

    #[test]
    fn test_transform_translation() {
        let a = Aabb::new(Vec3::ZERO, Vec3::ONE);
        let t = Mat4::from_translation(glam::Vec3::new(1.0, 2.0, 3.0));
        let result = a.transform(&t);
        assert!(approx_eq(result.min, Vec3A::new(1.0, 2.0, 3.0)));
        assert!(approx_eq(result.max, Vec3A::new(2.0, 3.0, 4.0)));
    }

    #[test]
    fn test_transform_uniform_scale() {
        let a = Aabb::new(Vec3::new(-1.0, -1.0, -1.0), Vec3::ONE);
        let s = Mat4::from_scale(glam::Vec3::splat(2.0));
        let result = a.transform(&s);
        assert!(approx_eq(result.min, Vec3A::splat(-2.0)));
        assert!(approx_eq(result.max, Vec3A::splat(2.0)));
    }

    #[test]
    fn test_transform_rotation_90_deg() {
        // Rotating a unit AABB 90° around Z: (1,1,1) → still (1,1,1) by symmetry
        let a = Aabb::new(Vec3::ZERO, Vec3::ONE);
        let r = Mat4::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let result = a.transform(&r);
        // The result should still contain the origin and be approximately unit-sized
        assert!(result.is_valid());
        assert!((result.surface_area() - a.surface_area()).abs() < 1e-4);
    }

    #[test]
    fn test_transform_empty() {
        let e = Aabb::empty();
        let result = e.transform(&Mat4::IDENTITY);
        assert!(result.is_empty());
    }

    // --- Corners ---

    #[test]
    fn test_corners_count_and_bounds() {
        let a = Aabb::new(Vec3::ZERO, Vec3::ONE);
        let corners = a.corners();
        assert_eq!(corners.len(), 8);
        for c in corners {
            assert!(a.contains_point(c));
        }
    }

    // --- Traits ---

    #[test]
    fn test_default_is_empty() {
        let a = Aabb::default();
        assert!(a.is_empty());
    }

    #[test]
    fn test_display() {
        let a = Aabb::new(Vec3::ZERO, Vec3::ONE);
        let s = format!("{}", a);
        assert!(s.contains("min") && s.contains("max"));
    }
}