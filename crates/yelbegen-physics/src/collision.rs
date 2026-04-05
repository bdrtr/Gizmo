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

#[derive(Debug, Clone)]
pub struct CollisionManifold {
    pub is_colliding: bool,
    pub normal: Vec3,
    pub penetration: f32,
    pub contact_points: Vec<Vec3>, // Çoklu temas noktaları (Multi-point contact)
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

        // Çarpışma noktasını tahmini olarak iki objenin birbirine değdiği ara yüzeyden alıyoruz
        // AABB-AABB çarpışması bir "volume" olduğu için en basit centroid (merkez) alımını yaparız:
        let mut contact_point = pos_a + (diff * 0.5);
        // Tam değdiği yüzeye kilitleyelim:
        if normal.x != 0.0 { contact_point.x = pos_a.x + normal.x * a_ex.x; }
        if normal.y != 0.0 { contact_point.y = pos_a.y + normal.y * a_ex.y; }
        if normal.z != 0.0 { contact_point.z = pos_a.z + normal.z * a_ex.z; }

        CollisionManifold { is_colliding: true, normal, penetration: p, contact_points: vec![contact_point] }
    } else {
        CollisionManifold { is_colliding: false, normal: Vec3::ZERO, penetration: 0.0, contact_points: vec![] }
    }
}

pub fn check_sphere_sphere_manifold(pos_a: Vec3, s_a: &Sphere, pos_b: Vec3, s_b: &Sphere) -> CollisionManifold {
    let diff = pos_b - pos_a;
    let dist_sq = diff.length_squared();
    let sum_r = s_a.radius + s_b.radius;

    if dist_sq < sum_r * sum_r {
        let dist = dist_sq.sqrt();
        let penetration = sum_r - dist;
        let normal = if dist > 0.0001 {
            diff / dist
        } else {
            Vec3::new(0.0, 1.0, 0.0) // Eger merkezleri tamamen ic ice gecmisse rastgele yukari it
        };

        // Kürenin çarpışma noktası her zaman kendi yüzeyindedir
        let contact_point = pos_a + (normal * (s_a.radius - penetration * 0.5));

        CollisionManifold { is_colliding: true, normal, penetration, contact_points: vec![contact_point] }
    } else {
        CollisionManifold { is_colliding: false, normal: Vec3::ZERO, penetration: 0.0, contact_points: vec![] }
    }
}

pub fn check_sphere_aabb_manifold(pos_s: Vec3, sphere: &Sphere, pos_aabb: Vec3, aabb: &Aabb) -> CollisionManifold {
    // 1. Sphere merkezinin AABB kutusuna olan en yakin noktasini bul
    let mut closest_point = pos_s;

    let min_b = pos_aabb - aabb.half_extents;
    let max_b = pos_aabb + aabb.half_extents;

    closest_point.x = closest_point.x.max(min_b.x).min(max_b.x);
    closest_point.y = closest_point.y.max(min_b.y).min(max_b.y);
    closest_point.z = closest_point.z.max(min_b.z).min(max_b.z);

    // 2. Kure bu en yakin noktaya ne kadar yakin?
    let diff = closest_point - pos_s;
    let dist_sq = diff.length_squared();

    if dist_sq < sphere.radius * sphere.radius {
        // Çarpışma var!
        let dist = dist_sq.sqrt();
        
        let (normal, penetration) = if dist > 0.0001 {
            // Normal disari dogru
            let n = diff / dist;
            (n, sphere.radius - dist)
        } else {
            // Sphere tam AABB icine girmisse en cok hangi eksenden cikabilir?
            // Kaba bir tahmin: AABB'den disari it.
            let diff_center = pos_aabb - pos_s;
            let n = diff_center.normalize();
            (n * -1.0, sphere.radius) // Rastgele disari
        };

        CollisionManifold { is_colliding: true, normal, penetration, contact_points: vec![closest_point] }
    } else {
        CollisionManifold { is_colliding: false, normal: Vec3::ZERO, penetration: 0.0, contact_points: vec![] }
    }
}
