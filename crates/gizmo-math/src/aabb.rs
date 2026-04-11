use glam::{Mat4, Vec3, Vec4};
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
            min: Vec3::new(f32::MAX, f32::MAX, f32::MAX),
            max: Vec3::new(f32::MIN, f32::MIN, f32::MIN),
        }
    }

    pub fn extend(&mut self, pt: Vec3) {
        self.min.x = self.min.x.min(pt.x);
        self.min.y = self.min.y.min(pt.y);
        self.min.z = self.min.z.min(pt.z);
        self.max.x = self.max.x.max(pt.x);
        self.max.y = self.max.y.max(pt.y);
        self.max.z = self.max.z.max(pt.z);
    }

    /// OBB köşelerini hesaplayıp tekrar AABB'ye çevirerek dönüşüm uygular
    pub fn transform(&self, mat: &Mat4) -> Self {
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
            let transformed_pt = *mat * Vec4::new(corner.x, corner.y, corner.z, 1.0);
            transformed_aabb.extend(Vec3::new(
                transformed_pt.x / transformed_pt.w,
                transformed_pt.y / transformed_pt.w,
                transformed_pt.z / transformed_pt.w,
            ));
        }

        transformed_aabb
    }

    pub fn intersects(&self, other: &Self) -> bool {
        self.min.x <= other.max.x
            && self.max.x >= other.min.x
            && self.min.y <= other.max.y
            && self.max.y >= other.min.y
            && self.min.z <= other.max.z
            && self.max.z >= other.min.z
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
}
