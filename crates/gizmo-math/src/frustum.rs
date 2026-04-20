use crate::aabb::Aabb;
use glam::{Mat4, Vec3};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intersection {
    Outside,
    Partial,
    Inside,
}

#[derive(Debug, Clone, Copy)]
pub struct Plane {
    pub normal: Vec3,
    pub distance: f32, // Orijinden uzaklık
}

impl Plane {
    pub fn new(x: f32, y: f32, z: f32, w: f32) -> Self {
        let length = (x * x + y * y + z * z).sqrt();
        if length < 1e-10 {
            return Self {
                normal: Vec3::Z,
                distance: 0.0,
            };
        }
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

#[derive(Debug, Clone, Copy)]
pub struct Frustum {
    pub planes: [Plane; 6],
}

impl Frustum {
    /// Projection * View matrisinden 6 adet Plane çıkarır.
    pub fn from_matrix(vp: &Mat4) -> Self {
        let r0 = vp.row(0);
        let r1 = vp.row(1);
        let r2 = vp.row(2);
        let r3 = vp.row(3);

        // Left Plane
        let left = Plane::new(r3.x + r0.x, r3.y + r0.y, r3.z + r0.z, r3.w + r0.w);
        // Right Plane
        let right = Plane::new(r3.x - r0.x, r3.y - r0.y, r3.z - r0.z, r3.w - r0.w);
        // Bottom Plane
        let bottom = Plane::new(r3.x + r1.x, r3.y + r1.y, r3.z + r1.z, r3.w + r1.w);
        // Top Plane
        let top = Plane::new(r3.x - r1.x, r3.y - r1.y, r3.z - r1.z, r3.w - r1.w);

        // WGPU / Vulkan / DX NDC: Z ∈ [0, 1]. OpenGL için near = r3 + r2, ...
        let near_wgpu = Plane::new(r2.x, r2.y, r2.z, r2.w);
        // Far Plane
        let far = Plane::new(r3.x - r2.x, r3.y - r2.y, r3.z - r2.z, r3.w - r2.w);

        Self {
            planes: [left, right, bottom, top, near_wgpu, far],
        }
    }

    /// Bir kürenin (Sphere) frustum ile kesişip kesişmediğini hızlıca test eder.
    pub fn intersects_sphere(&self, center: Vec3, radius: f32) -> bool {
        self.planes.iter().all(|p| p.distance_to_point(center) >= -radius)
    }

    /// Bir AABB objesinin (Bounding Box) Frustum durumunu detaylı şekilde hesaplar.
    pub fn test_aabb(&self, aabb: &Aabb) -> Intersection {
        let mut all_inside = true;
        for plane in &self.planes {
            let px = if plane.normal.x > 0.0 { aabb.max.x } else { aabb.min.x };
            let py = if plane.normal.y > 0.0 { aabb.max.y } else { aabb.min.y };
            let pz = if plane.normal.z > 0.0 { aabb.max.z } else { aabb.min.z };
            let p_vertex = Vec3::new(px, py, pz);

            let nx = if plane.normal.x < 0.0 { aabb.max.x } else { aabb.min.x };
            let ny = if plane.normal.y < 0.0 { aabb.max.y } else { aabb.min.y };
            let nz = if plane.normal.z < 0.0 { aabb.max.z } else { aabb.min.z };
            let n_vertex = Vec3::new(nx, ny, nz);

            if plane.distance_to_point(p_vertex) < 0.0 {
                return Intersection::Outside;
            }
            if plane.distance_to_point(n_vertex) < 0.0 {
                all_inside = false;
            }
        }
        
        if all_inside {
            Intersection::Inside
        } else {
            Intersection::Partial
        }
    }

    /// AABB objesinin Frustum dahilinde görünür olup olmadığını kontrol eder.
    pub fn intersects_aabb(&self, aabb: &Aabb) -> bool {
        self.test_aabb(aabb) != Intersection::Outside
    }
}

#[cfg(test)]
mod tests {
    use super::Frustum;
    use crate::aabb::Aabb;
    use glam::{Mat4, Vec3};

    #[test]
    fn frustum_intersects_aabb_in_front_of_camera() {
        let view = Mat4::look_at_rh(Vec3::new(0.0, 0.0, 8.0), Vec3::ZERO, Vec3::Y);
        let proj = Mat4::perspective_rh(std::f32::consts::FRAC_PI_4, 1.0, 0.1, 100.0);
        let vp = proj * view;
        let frustum = Frustum::from_matrix(&vp);

        let unit_cube = Aabb::new(Vec3::splat(-0.5), Vec3::splat(0.5));
        assert_eq!(
            frustum.test_aabb(&unit_cube),
            super::Intersection::Inside,
            "origin cube should be fully inside the camera frustum"
        );
    }

    #[test]
    fn frustum_rejects_aabb_behind_camera() {
        let view = Mat4::look_at_rh(Vec3::new(0.0, 0.0, 8.0), Vec3::ZERO, Vec3::Y);
        let proj = Mat4::perspective_rh(std::f32::consts::FRAC_PI_4, 1.0, 0.1, 100.0);
        let vp = proj * view;
        let frustum = Frustum::from_matrix(&vp);

        // Camera is at Z=8, looking towards Z=0 (-Z direction).
        // Anything with Z > 8 is strictly behind the near plane.
        let behind = Aabb::new(Vec3::new(-1.0, -1.0, 10.0), Vec3::new(1.0, 1.0, 12.0));
        assert_eq!(
            frustum.test_aabb(&behind),
            super::Intersection::Outside,
            "AABB directly behind camera should be culled (Outside)"
        );
    }

    #[test]
    fn frustum_rejects_aabb_beyond_far_plane() {
        let view = Mat4::look_at_rh(Vec3::new(0.0, 0.0, 8.0), Vec3::ZERO, Vec3::Y);
        let proj = Mat4::perspective_rh(std::f32::consts::FRAC_PI_4, 1.0, 0.1, 100.0);
        let vp = proj * view;
        let frustum = Frustum::from_matrix(&vp);

        // Camera is at Z=8, looking at -Z direction. Far plane is at 8 - 100 = -92.
        // AABB at Z=-100 is strictly beyond the limits of the far plane. 
        let far_away = Aabb::new(Vec3::new(-1.0, -1.0, -105.0), Vec3::new(1.0, 1.0, -95.0));
        assert_eq!(
            frustum.test_aabb(&far_away),
            super::Intersection::Outside,
            "AABB strictly beyond far plane should be culled (Outside)"
        );
    }

    #[test]
    fn frustum_edge_cases() {
        let view = Mat4::look_at_rh(Vec3::new(0.0, 0.0, 8.0), Vec3::ZERO, Vec3::Y);
        let proj = Mat4::perspective_rh(std::f32::consts::FRAC_PI_4, 1.0, 0.1, 100.0);
        let vp = proj * view;
        let frustum = Frustum::from_matrix(&vp);

        // 1. AABB Enclosing the entire camera and scene
        let huge = Aabb::new(Vec3::splat(-1000.0), Vec3::splat(1000.0));
        assert_eq!(
            frustum.test_aabb(&huge),
            super::Intersection::Partial,
            "Huge AABB enclosing frustum should be evaluated as Partial"
        );

        // 2. Degenerate Point AABB exactly at the origin (Inside)
        let pt_inside = Aabb::new(Vec3::ZERO, Vec3::ZERO);
        assert_eq!(
            frustum.test_aabb(&pt_inside),
            super::Intersection::Inside,
            "Degenerate point AABB at origin should be Inside"
        );

        // 3. Degenerate Point AABB far outside the frustum (Outside)
        let pt_outside = Aabb::new(Vec3::splat(1000.0), Vec3::splat(1000.0));
        assert_eq!(
            frustum.test_aabb(&pt_outside),
            super::Intersection::Outside,
            "Degenerate point AABB outside frustum should be Outside"
        );
    }
}
