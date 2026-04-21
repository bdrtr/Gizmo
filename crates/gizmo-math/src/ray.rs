use glam::{Quat, Vec3, Vec3A};

#[derive(Debug, Clone, Copy)]
pub struct Ray {
    pub origin: Vec3A,
    pub direction: Vec3A, // Normalize edilmiş olmalı
}

impl Ray {
    #[inline]
    pub fn new(origin: impl Into<Vec3A>, direction: impl Into<Vec3A>) -> Self {
        let dir = direction.into().normalize();
        debug_assert!(dir.is_finite(), "Ray direction must be non-zero");
        Self {
            origin: origin.into(),
            direction: dir,
        }
    }

    /// Işının uzayda `t` uzaklığındaki ulaştığı (çarpıştığı) kesin noktayı hesaplar.
    #[inline]
    pub fn at(self, t: f32) -> Vec3A {
        self.origin + self.direction * t
    }

    /// Bir eksen kısıtlı boundary kutusuyla kesişim testi yapar (Slab Algorithm).
    /// Kesişiyorsa t_near mesafesini döner, kesişmiyorsa None döner.
    #[inline]
    pub(crate) fn intersect_bounds(self, min: Vec3A, max: Vec3A) -> Option<f32> {
        let inv_dir = self.direction.recip();

        let t0 = (min - self.origin) * inv_dir;
        let t1 = (max - self.origin) * inv_dir;

        let tmin_vec = t0.min(t1);
        let tmax_vec = t0.max(t1);

        let tmin = tmin_vec.x.max(tmin_vec.y).max(tmin_vec.z);
        let tmax = tmax_vec.x.min(tmax_vec.y).min(tmax_vec.z);

        if tmin <= tmax && tmax > 0.0 {
            Some(if tmin > 0.0 { tmin } else { tmax })
        } else {
            None
        }
    }

    /// Bir Aabb nesnesiyle (Axis-Aligned Bounding Box) doğrudan kesişim testi yapar.
    #[inline]
    pub fn intersect_aabb(self, aabb: crate::aabb::Aabb) -> Option<f32> {
        self.intersect_bounds(aabb.min, aabb.max)
    }

    /// Möller–Trumbore algoritması kullanarak bir üçgenle hassas kesişim (Mesh Raycasting) testi yapar.
    /// Kesişiyorsa t_near mesafesini döner, aksi halde None döner.
    #[inline]
    pub fn intersect_triangle(self, v0: impl Into<Vec3A>, v1: impl Into<Vec3A>, v2: impl Into<Vec3A>) -> Option<f32> {
        let v0 = v0.into();
        let v1 = v1.into();
        let v2 = v2.into();
        let edge1 = v1 - v0;
        let edge2 = v2 - v0;
        
        let h = self.direction.cross(edge2);
        let a = edge1.dot(h);
        
        // Culling backfaces and parallel rays
        if a.abs() < 1e-8 {
            return None;
        }
        
        let f = 1.0 / a;
        let s = self.origin - v0;
        let u = f * s.dot(h);
        
        if !(0.0..=1.0).contains(&u) {
            return None;
        }
        
        let q = s.cross(edge1);
        let v = f * self.direction.dot(q);
        
        if v < 0.0 || u + v > 1.0 {
            return None;
        }
        
        let t = f * edge2.dot(q);
        
        if t > 1e-8 {
            Some(t)
        } else {
            None
        }
    }

    /// Bir OBB (Oriented Bounding Box) kutusuyla kesişim testi yapar.
    /// Kesişiyorsa t_near mesafesini döner, kesişmiyorsa None döner.
    #[inline]
    pub fn intersect_obb(self, center: impl Into<Vec3A>, half_extents: impl Into<Vec3A>, rotation: Quat) -> Option<f32> {
        let c = center.into();
        let he = half_extents.into();
        let inv_rot = rotation.inverse();

        // Işını OBB'nin yerel uzayına çeviriyoruz. 
        // Quat * Vec3A dönüşümü bulunmadığı için Vec3 üzerinden yapıp Vec3A'ya cast etmeliyiz.
        let local_origin = Vec3A::from(inv_rot * Vec3::from(self.origin - c));
        let local_direction = Vec3A::from(inv_rot * Vec3::from(self.direction));

        // Quat dönüşümü uzunluğu korusa da, `Ray` constructor ile garantili normalize ediyoruz.
        let local_ray = Ray::new(local_origin, local_direction);

        // Yerel koordinatlarda AABB testi
        local_ray.intersect_bounds(-he, he)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ray_intersect_aabb_hit() {
        let ray = Ray::new(Vec3::new(0.0, 0.0, -5.0), Vec3::new(0.0, 0.0, 1.0));
        let aabb = crate::aabb::Aabb::new(Vec3::new(-1.0, -1.0, -1.0), Vec3::new(1.0, 1.0, 1.0));

        let t = ray.intersect_aabb(aabb);
        assert!(t.is_some());
        assert!((t.unwrap() - 4.0).abs() < 1e-5); // Hits the front face at z = -1, origin is -5, distance is 4
    }

    #[test]
    fn test_ray_intersect_aabb_miss() {
        let ray = Ray::new(Vec3::new(0.0, 5.0, -5.0), Vec3::new(0.0, 0.0, 1.0));
        let aabb = crate::aabb::Aabb::new(Vec3::new(-1.0, -1.0, -1.0), Vec3::new(1.0, 1.0, 1.0));

        let t = ray.intersect_aabb(aabb);
        assert!(t.is_none());
    }

    #[test]
    fn test_ray_intersect_aabb_inside() {
        let ray = Ray::new(Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0));
        let aabb = crate::aabb::Aabb::new(Vec3::new(-1.0, -1.0, -1.0), Vec3::new(1.0, 1.0, 1.0));

        let t = ray.intersect_aabb(aabb);
        assert!(t.is_some());
        assert!((t.unwrap() - 1.0).abs() < 1e-5); // Inside the box, hits the back face at z = 1, distance is 1
    }

    #[test]
    fn test_ray_intersect_aabb_behind() {
        let ray = Ray::new(Vec3::new(0.0, 0.0, 5.0), Vec3::new(0.0, 0.0, 1.0));
        let aabb = crate::aabb::Aabb::new(Vec3::new(-1.0, -1.0, -1.0), Vec3::new(1.0, 1.0, 1.0));

        let t = ray.intersect_aabb(aabb);
        assert!(t.is_none()); // The box is strictly behind the ray origin
    }

    #[test]
    fn test_ray_intersect_obb() {
        let origin = Vec3::new(0.0, 0.0, -5.0);
        let direction = Vec3::new(0.0, 0.0, 1.0);
        let ray = Ray::new(origin, direction);

        let obb_center = Vec3::new(0.0, 0.0, 0.0);
        let obb_extents = Vec3::new(1.0, 1.0, 1.0);
        
        // 45 degrees rotated around Y
        let rot = Quat::from_rotation_y(std::f32::consts::FRAC_PI_4);
        
        let t = ray.intersect_obb(obb_center, obb_extents, rot);
        assert!(t.is_some());
        
        // Since OBB is rotated by 45 degrees, ray hits the tilted face earlier.
        // Unrotated distance is 4.0. With 45 degree tilt, the half-diagonal length is sqrt(2), so front face is at -sqrt(2).
        // 5.0 - 1.414 = approx 3.585
        assert!((t.unwrap() - (5.0 - std::f32::consts::SQRT_2)).abs() < 1e-4);
    }

    #[test]
    fn test_ray_parallel_hit() {
        // Parallel ray moving exactly along the Y-axis (Direction X and Z are exactly ZERO)
        let ray = Ray::new(Vec3::new(0.0, -5.0, 0.0), Vec3::Y);
        let aabb = crate::aabb::Aabb::new(Vec3::new(-1.0, -1.0, -1.0), Vec3::new(1.0, 1.0, 1.0));

        // This hit should register and perfectly calculate intersections without Div-By-Zero NaNs
        let t = ray.intersect_aabb(aabb);
        assert!(t.is_some());
        assert!((t.unwrap() - 4.0).abs() < 1e-5);
    }

    #[test]
    fn test_ray_parallel_miss() {
        // Parallel ray moving exactly along the Y-axis but offset completely outside the AABB X-range
        let ray_miss = Ray::new(Vec3::new(5.0, -5.0, 0.0), Vec3::Y);
        let aabb = crate::aabb::Aabb::new(Vec3::new(-1.0, -1.0, -1.0), Vec3::new(1.0, 1.0, 1.0));

        let t_miss = ray_miss.intersect_aabb(aabb);
        assert!(t_miss.is_none());
    }
}
