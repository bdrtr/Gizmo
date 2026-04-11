use crate::aabb::Aabb;
use glam::{Mat4, Vec3};

#[derive(Debug, Clone, Copy)]
pub struct Plane {
    pub normal: Vec3,
    pub distance: f32, // Orijinden uzaklık
}

impl Plane {
    pub fn new(x: f32, y: f32, z: f32, w: f32) -> Self {
        let length = (x * x + y * y + z * z).sqrt();
        Self {
            normal: Vec3::new(x / length, y / length, z / length),
            distance: w / length,
        }
    }

    /// Eğer uzaklık > 0 ise noktanın "önündedir"
    pub fn distance_to_point(&self, pt: Vec3) -> f32 {
        self.normal.dot(pt) + self.distance
    }
}

pub struct Frustum {
    pub planes: [Plane; 6],
}

impl Frustum {
    /// Projection * View matrisinden 6 adet Plane çıkarır.
    pub fn from_matrix(vp: &Mat4) -> Self {
        // glam::Mat4 uses x_axis, y_axis, z_axis, w_axis as columns.
        let m11 = vp.x_axis.x;
        let m12 = vp.y_axis.x;
        let m13 = vp.z_axis.x;
        let m14 = vp.w_axis.x;
        let m21 = vp.x_axis.y;
        let m22 = vp.y_axis.y;
        let m23 = vp.z_axis.y;
        let m24 = vp.w_axis.y;
        let m31 = vp.x_axis.z;
        let m32 = vp.y_axis.z;
        let m33 = vp.z_axis.z;
        let m34 = vp.w_axis.z;
        let m41 = vp.x_axis.w;
        let m42 = vp.y_axis.w;
        let m43 = vp.z_axis.w;
        let m44 = vp.w_axis.w;

        // Left Plane
        let left = Plane::new(m41 + m11, m42 + m12, m43 + m13, m44 + m14);
        // Right Plane
        let right = Plane::new(m41 - m11, m42 - m12, m43 - m13, m44 - m14);
        // Bottom Plane
        let bottom = Plane::new(m41 + m21, m42 + m22, m43 + m23, m44 + m24);
        // Top Plane
        let top = Plane::new(m41 - m21, m42 - m22, m43 - m23, m44 - m24);

        let near_wgpu = Plane::new(m31, m32, m33, m34);
        let far = Plane::new(m41 - m31, m42 - m32, m43 - m33, m44 - m34);

        Self {
            planes: [left, right, bottom, top, near_wgpu, far],
        }
    }

    /// Küresel (AABB) bir objenin dışarıda kalıp kalmadığını test eder.
    pub fn contains_aabb(&self, aabb: &Aabb) -> bool {
        for plane in &self.planes {
            let px = if plane.normal.x > 0.0 {
                aabb.max.x
            } else {
                aabb.min.x
            };
            let py = if plane.normal.y > 0.0 {
                aabb.max.y
            } else {
                aabb.min.y
            };
            let pz = if plane.normal.z > 0.0 {
                aabb.max.z
            } else {
                aabb.min.z
            };

            if plane.distance_to_point(Vec3::new(px, py, pz)) < 0.0 {
                return false;
            }
        }
        true
    }
}
