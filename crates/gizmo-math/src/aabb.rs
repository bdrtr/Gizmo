use std::f32;
use crate::vec3::Vec3;
use crate::mat4::Mat4;

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
            let transformed_pt = *mat * crate::vec4::Vec4::new(corner.x, corner.y, corner.z, 1.0);
            transformed_aabb.extend(Vec3::new(
                transformed_pt.x / transformed_pt.w,
                transformed_pt.y / transformed_pt.w,
                transformed_pt.z / transformed_pt.w,
            ));
        }

        transformed_aabb
    }
}
