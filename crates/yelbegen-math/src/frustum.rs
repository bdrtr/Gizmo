use crate::vec3::Vec3;
use crate::mat4::Mat4;
use crate::aabb::Aabb;

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
        // Note: m_ij notation typical in frustum extraction corresponds to row i, col j
        // In column-major Mat4, mat.cols[c].x is m_1{c+1}
        let m11 = vp.cols[0].x; let m12 = vp.cols[1].x; let m13 = vp.cols[2].x; let m14 = vp.cols[3].x;
        let m21 = vp.cols[0].y; let m22 = vp.cols[1].y; let m23 = vp.cols[2].y; let m24 = vp.cols[3].y;
        let m31 = vp.cols[0].z; let m32 = vp.cols[1].z; let m33 = vp.cols[2].z; let m34 = vp.cols[3].z;
        let m41 = vp.cols[0].w; let m42 = vp.cols[1].w; let m43 = vp.cols[2].w; let m44 = vp.cols[3].w;

        // Left Plane
        let left = Plane::new(m41 + m11, m42 + m12, m43 + m13, m44 + m14);
        // Right Plane
        let right = Plane::new(m41 - m11, m42 - m12, m43 - m13, m44 - m14);
        // Bottom Plane
        let bottom = Plane::new(m41 + m21, m42 + m22, m43 + m23, m44 + m24);
        // Top Plane
        let top = Plane::new(m41 - m21, m42 - m22, m43 - m23, m44 - m24);
        // Near Plane
        let _near = Plane::new(m41 + m31, m42 + m32, m43 + m33, m44 + m34); // Note: For WebGPU / API depth [0, 1] usually just m31, m32... But we'll use standard m4+m3 for [-1, 1]. In WGPU it depends on configuration. YELBEGEN engine defaults to wgpu depth format. Let's just store classic.`m31`, `m32`, `m33`, `m34` for Near if depth is 0..1
        // Actually wgpu is 0..1 depth, Near plane is:
        let near_wgpu = Plane::new(m31, m32, m33, m34);
        // Far Plane
        let far = Plane::new(m41 - m31, m42 - m32, m43 - m33, m44 - m34);

        Self {
            planes: [left, right, bottom, top, near_wgpu, far],
        }
    }

    /// Küresel (AABB) bir objenin dışarıda kalıp kalmadığını test eder.
    pub fn contains_aabb(&self, aabb: &Aabb) -> bool {
        for plane in &self.planes {
            // AABB'nin plane'e en yakın (veya test edilen) noktasını buluyoruz (Positive Vertex)
            let px = if plane.normal.x > 0.0 { aabb.max.x } else { aabb.min.x };
            let py = if plane.normal.y > 0.0 { aabb.max.y } else { aabb.min.y };
            let pz = if plane.normal.z > 0.0 { aabb.max.z } else { aabb.min.z };

            // Eğer "P" noktası bile düzlemin arkasında ise (-), tüm AABB dışarıdadır!
            if plane.distance_to_point(Vec3::new(px, py, pz)) < 0.0 {
                return false;
            }
        }
        true
    }
}
