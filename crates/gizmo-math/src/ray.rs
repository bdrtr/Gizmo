use glam::{Quat, Vec3};
pub struct Ray {
    pub origin: Vec3,
    pub direction: Vec3, // Normalize edilmiş olmalı
}

impl Ray {
    pub fn new(origin: Vec3, direction: Vec3) -> Self {
        Self {
            origin,
            direction: direction.normalize(),
        }
    }

    /// Bir AABB (Axis-Aligned Bounding Box) kutusuyla kesişim testi yapar (Slab Algorithm).
    /// Kesişiyorsa t_near mesafesini döner, kesişmiyorsa None döner.
    pub fn intersect_aabb(&self, min: Vec3, max: Vec3) -> Option<f32> {
        let inv_dir = Vec3::new(
            if self.direction.x.abs() > 1e-8 {
                1.0 / self.direction.x
            } else {
                f32::MAX.copysign(self.direction.x)
            },
            if self.direction.y.abs() > 1e-8 {
                1.0 / self.direction.y
            } else {
                f32::MAX.copysign(self.direction.y)
            },
            if self.direction.z.abs() > 1e-8 {
                1.0 / self.direction.z
            } else {
                f32::MAX.copysign(self.direction.z)
            },
        );

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

    /// Bir OBB (Oriented Bounding Box) kutusuyla kesişim testi yapar.
    /// Kesişiyorsa t_near mesafesini döner, kesişmiyorsa None döner.
    pub fn intersect_obb(&self, center: Vec3, half_extents: Vec3, rotation: Quat) -> Option<f32> {
        let inv_rot = rotation.inverse();

        // Işını OBB'nin yerel uzayına çeviriyoruz
        let local_origin = inv_rot * (self.origin - center);
        let local_direction = inv_rot * self.direction;

        let local_ray = Ray {
            origin: local_origin,
            direction: local_direction, // Rotasyon vektör boyunu değiştirmediği için hala mormalize edilmiş haldedir
        };

        // Yerel koordinatlarda AABB testi
        local_ray.intersect_aabb(-half_extents, half_extents)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ray_intersect_aabb_hit() {
        let ray = Ray::new(Vec3::new(0.0, 0.0, -5.0), Vec3::new(0.0, 0.0, 1.0));
        let min = Vec3::new(-1.0, -1.0, -1.0);
        let max = Vec3::new(1.0, 1.0, 1.0);

        let t = ray.intersect_aabb(min, max);
        assert!(t.is_some());
        assert_eq!(t.unwrap(), 4.0); // Hits the front face at z = -1, origin is -5, distance is 4
    }

    #[test]
    fn test_ray_intersect_aabb_miss() {
        let ray = Ray::new(Vec3::new(0.0, 5.0, -5.0), Vec3::new(0.0, 0.0, 1.0));
        let min = Vec3::new(-1.0, -1.0, -1.0);
        let max = Vec3::new(1.0, 1.0, 1.0);

        let t = ray.intersect_aabb(min, max);
        assert!(t.is_none());
    }

    #[test]
    fn test_ray_intersect_aabb_inside() {
        let ray = Ray::new(Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0));
        let min = Vec3::new(-1.0, -1.0, -1.0);
        let max = Vec3::new(1.0, 1.0, 1.0);

        let t = ray.intersect_aabb(min, max);
        assert!(t.is_some());
        assert_eq!(t.unwrap(), 1.0); // Inside the box, hits the back face at z = 1, distance is 1
    }

    #[test]
    fn test_ray_intersect_aabb_behind() {
        let ray = Ray::new(Vec3::new(0.0, 0.0, 5.0), Vec3::new(0.0, 0.0, 1.0));
        let min = Vec3::new(-1.0, -1.0, -1.0);
        let max = Vec3::new(1.0, 1.0, 1.0);

        let t = ray.intersect_aabb(min, max);
        assert!(t.is_none()); // The box is strictly behind the ray origin
    }
}
