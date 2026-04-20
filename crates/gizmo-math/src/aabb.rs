use glam::{Mat4, Vec3};
use std::f32;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    pub fn new(min: Vec3, max: Vec3) -> Self {
        Self { min, max }
    }

    pub fn empty() -> Self {
        Self {
            min: Vec3::splat(f32::INFINITY),
            max: Vec3::splat(f32::NEG_INFINITY),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.min.x > self.max.x || self.min.y > self.max.y || self.min.z > self.max.z
    }

    pub fn is_valid(&self) -> bool {
        !self.is_empty()
    }

    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    pub fn half_extents(&self) -> Vec3 {
        (self.max - self.min) * 0.5
    }

    pub fn size(&self) -> Vec3 {
        self.max - self.min
    }

    pub fn contains_point(&self, pt: Vec3) -> bool {
        pt.x >= self.min.x && pt.x <= self.max.x &&
        pt.y >= self.min.y && pt.y <= self.max.y &&
        pt.z >= self.min.z && pt.z <= self.max.z
    }

    pub fn extend(&mut self, pt: Vec3) {
        self.min = self.min.min(pt);
        self.max = self.max.max(pt);
    }

    pub fn merge(&self, other: &Self) -> Self {
        Self {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
        }
    }

    /// OBB köşelerini hesaplayıp tekrar AABB'ye çevirerek dönüşüm uygular
    pub fn transform(&self, mat: &Mat4) -> Self {
        if self.is_empty() {
            return *self;
        }

        let corners = [
            Vec3::new(self.min.x, self.min.y, self.min.z),
            Vec3::new(self.max.x, self.min.y, self.min.z),
            Vec3::new(self.min.x, self.max.y, self.min.z),
            Vec3::new(self.max.x, self.max.y, self.min.z),
            Vec3::new(self.min.x, self.min.y, self.max.z),
            Vec3::new(self.max.x, self.min.y, self.max.z),
            Vec3::new(self.min.x, self.max.y, self.max.z),
            Vec3::new(self.max.x, self.max.y, self.max.z),
        ];

        let mut transformed_aabb = Self::empty();
        for corner in corners.iter() {
            transformed_aabb.extend(mat.transform_point3(*corner));
        }

        transformed_aabb
    }

    /// Checks if this AABB intersects with another. 
    /// Note: This is an inclusive intersection, meaning if the edges of two AABBs touch (min == max), it will return `true`.
    /// For exclusive intersection where touching boundaries do not count, use `intersects_exclusive`.
    pub fn intersects(&self, other: &Self) -> bool {
        if self.is_empty() || other.is_empty() {
            return false;
        }
        self.min.x <= other.max.x
            && self.max.x >= other.min.x
            && self.min.y <= other.max.y
            && self.max.y >= other.min.y
            && self.min.z <= other.max.z
            && self.max.z >= other.min.z
    }

    /// Checks if this AABB intersects with another exclusively. (Edge-touching does not count as intersection).
    pub fn intersects_exclusive(&self, other: &Self) -> bool {
        if self.is_empty() || other.is_empty() {
            return false;
        }
        self.min.x < other.max.x
            && self.max.x > other.min.x
            && self.min.y < other.max.y
            && self.max.y > other.min.y
            && self.min.z < other.max.z
            && self.max.z > other.min.z
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_aabb_intersects_overlapping() {
        let a = Aabb::new(Vec3::new(0.0, 0.0, 0.0), Vec3::new(2.0, 2.0, 2.0));
        let b = Aabb::new(Vec3::new(1.0, 1.0, 1.0), Vec3::new(3.0, 3.0, 3.0));
        assert!(a.intersects(&b));
        assert!(b.intersects(&a));
    }

    #[test]
    fn test_aabb_intersects_disjoint() {
        let a = Aabb::new(Vec3::new(0.0, 0.0, 0.0), Vec3::new(2.0, 2.0, 2.0));
        let b = Aabb::new(Vec3::new(3.0, 3.0, 3.0), Vec3::new(5.0, 5.0, 5.0));
        assert!(!a.intersects(&b));
        assert!(!b.intersects(&a));
    }

    #[test]
    fn test_aabb_intersects_edge_touching() {
        let a = Aabb::new(Vec3::new(0.0, 0.0, 0.0), Vec3::new(2.0, 2.0, 2.0));
        let b = Aabb::new(Vec3::new(2.0, 0.0, 0.0), Vec3::new(4.0, 2.0, 2.0));
        assert!(a.intersects(&b)); // Edge-touching is considered intersection here
    }

    #[test]
    fn test_aabb_extend() {
        let mut a = Aabb::new(Vec3::new(0.0, 0.0, 0.0), Vec3::new(2.0, 2.0, 2.0));
        a.extend(Vec3::new(3.0, -1.0, 1.0));
        assert_eq!(a.min, Vec3::new(0.0, -1.0, 0.0));
        assert_eq!(a.max, Vec3::new(3.0, 2.0, 2.0));
    }

    #[test]
    fn test_aabb_transform_identity() {
        let a = Aabb::new(Vec3::new(1.0, 2.0, 3.0), Vec3::new(4.0, 5.0, 6.0));
        let result = a.transform(&Mat4::IDENTITY);
        assert!((result.min - a.min).length() < 1e-5);
        assert!((result.max - a.max).length() < 1e-5);
    }

    #[test]
    fn test_aabb_empty() {
        let mut a = Aabb::empty();
        assert!(a.is_empty());
        assert!(!a.is_valid());

        // Test extending an empty AABB
        let pt = Vec3::new(-5.0, 10.0, 3.0);
        a.extend(pt);
        assert!(!a.is_empty());
        assert!(a.is_valid());
        assert_eq!(a.min, pt);
        assert_eq!(a.max, pt);

        // Extend again
        let pt2 = Vec3::new(5.0, -10.0, 6.0);
        a.extend(pt2);
        assert_eq!(a.min, Vec3::new(-5.0, -10.0, 3.0));
        assert_eq!(a.max, Vec3::new(5.0, 10.0, 6.0));
    }
}
