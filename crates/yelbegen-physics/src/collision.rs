use yelbegen_math::Vec3;
use crate::shape::{Aabb, Sphere};

// Hızlı ve Kirli Algılamalar (Broad Phase testleri için muazzam optimize)

#[inline]
pub fn test_aabb_aabb(pos_a: Vec3, aabb_a: &Aabb, pos_b: Vec3, aabb_b: &Aabb) -> bool {
    let min_a = pos_a - aabb_a.half_extents;
    let max_a = pos_a + aabb_a.half_extents;
    
    let min_b = pos_b - aabb_b.half_extents;
    let max_b = pos_b + aabb_b.half_extents;

    (min_a.x <= max_b.x && max_a.x >= min_b.x) &&
    (min_a.y <= max_b.y && max_a.y >= min_b.y) &&
    (min_a.z <= max_b.z && max_a.z >= min_b.z)
}

#[inline]
pub fn test_sphere_sphere(pos_a: Vec3, s_a: &Sphere, pos_b: Vec3, s_b: &Sphere) -> bool {
    let dist_sq = pos_a.distance_squared(pos_b);
    let radius_sum = s_a.radius + s_b.radius;
    dist_sq <= (radius_sum * radius_sum)
}

#[derive(Debug, Clone, Copy)]
pub struct CollisionManifold {
    pub is_colliding: bool,
    pub normal: Vec3,       // A'yı B'den iten kuvvet yönü
    pub penetration: f32,   // Iki nesne ne kadar birbirine geçti? (Pos düzeltmesi için)
}

pub fn check_aabb_aabb_manifold(pos_a: Vec3, aabb_a: &Aabb, pos_b: Vec3, aabb_b: &Aabb) -> CollisionManifold {
    let diff = pos_b - pos_a; 
    
    let a_ex = aabb_a.half_extents;
    let b_ex = aabb_b.half_extents;

    let x_overlap = a_ex.x + b_ex.x - diff.x.abs();
    let y_overlap = a_ex.y + b_ex.y - diff.y.abs();
    let z_overlap = a_ex.z + b_ex.z - diff.z.abs();

    if x_overlap > 0.0 && y_overlap > 0.0 && z_overlap > 0.0 {
        // En az örtüşen eksen, çarpışma yüzeyimizdir (Normal vektor)
        let mut normal = Vec3::ZERO;
        let p;

        if x_overlap < y_overlap && x_overlap < z_overlap {
            normal.x = if diff.x > 0.0 { 1.0 } else { -1.0 };
            p = x_overlap;
        } else if y_overlap < z_overlap {
            normal.y = if diff.y > 0.0 { 1.0 } else { -1.0 };
            p = y_overlap;
        } else {
            normal.z = if diff.z > 0.0 { 1.0 } else { -1.0 };
            p = z_overlap;
        }

        CollisionManifold { is_colliding: true, normal, penetration: p }
    } else {
        CollisionManifold { is_colliding: false, normal: Vec3::ZERO, penetration: 0.0 }
    }
}
