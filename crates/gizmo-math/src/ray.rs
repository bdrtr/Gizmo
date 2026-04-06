use glam::Vec3;
pub struct Ray {
    pub origin: Vec3,
    pub direction: Vec3, // Normalize edilmiş olmalı
}

impl Ray {
    pub fn new(origin: Vec3, direction: Vec3) -> Self {
        Self { origin, direction: direction.normalize() }
    }

    /// Bir AABB (Axis-Aligned Bounding Box) kutusuyla kesişim testi yapar (Slab Algorithm).
    /// Kesişiyorsa t_near mesafesini döner, kesişmiyorsa None döner.
    pub fn intersect_aabb(&self, min: Vec3, max: Vec3) -> Option<f32> {
        let mut tmin = (min.x - self.origin.x) / self.direction.x;
        let mut tmax = (max.x - self.origin.x) / self.direction.x;

        if tmin > tmax {
            std::mem::swap(&mut tmin, &mut tmax);
        }

        let mut tymin = (min.y - self.origin.y) / self.direction.y;
        let mut tymax = (max.y - self.origin.y) / self.direction.y;

        if tymin > tymax {
            std::mem::swap(&mut tymin, &mut tymax);
        }

        if (tmin > tymax) || (tymin > tmax) {
            return None;
        }

        if tymin > tmin {
            tmin = tymin;
        }

        if tymax < tmax {
            tmax = tymax;
        }

        let mut tzmin = (min.z - self.origin.z) / self.direction.z;
        let mut tzmax = (max.z - self.origin.z) / self.direction.z;

        if tzmin > tzmax {
            std::mem::swap(&mut tzmin, &mut tzmax);
        }

        if (tmin > tzmax) || (tzmin > tmax) {
            return None;
        }

        if tzmin > tmin {
            tmin = tzmin;
        }

        if tzmax < tmax {
            tmax = tzmax;
        }

        // tmin < 0 ise kutunun içindeyiz, bu yüzden pozitif bir mesafe olan tmax'ı dönebiliriz.
        // Ancak bizim işimiz için dışarıdan vurmayı (tmin > 0) ölçmek yeterli.
        if tmax < 0.0 {
            return None; // Kutu kameranın arkasında kaldı
        }

        let t = if tmin < 0.0 { tmax } else { tmin };
        Some(t)
    }
}
