use gizmo_math::Vec3;
use crate::shape::{Aabb, Sphere, Capsule};

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
    pub contact_points: Vec<Vec3>,
}

pub fn check_aabb_aabb_manifold(pos_a: Vec3, aabb_a: &Aabb, pos_b: Vec3, aabb_b: &Aabb) -> CollisionManifold {
    let diff = pos_b - pos_a; 
    
    let a_ex = aabb_a.half_extents;
    let b_ex = aabb_b.half_extents;

    let x_overlap = a_ex.x + b_ex.x - diff.x.abs();
    let y_overlap = a_ex.y + b_ex.y - diff.y.abs();
    let z_overlap = a_ex.z + b_ex.z - diff.z.abs();

    if x_overlap > 0.0 && y_overlap > 0.0 && z_overlap > 0.0 {
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

        let mut contact_point = pos_a + (diff * 0.5);
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
            Vec3::new(0.0, 1.0, 0.0)
        };

        let contact_point = pos_a + (normal * (s_a.radius - penetration * 0.5));

        CollisionManifold { is_colliding: true, normal, penetration, contact_points: vec![contact_point] }
    } else {
        CollisionManifold { is_colliding: false, normal: Vec3::ZERO, penetration: 0.0, contact_points: vec![] }
    }
}

pub fn check_sphere_aabb_manifold(pos_s: Vec3, sphere: &Sphere, pos_aabb: Vec3, aabb: &Aabb) -> CollisionManifold {
    let mut closest_point = pos_s;

    let min_b = pos_aabb - aabb.half_extents;
    let max_b = pos_aabb + aabb.half_extents;

    closest_point.x = closest_point.x.max(min_b.x).min(max_b.x);
    closest_point.y = closest_point.y.max(min_b.y).min(max_b.y);
    closest_point.z = closest_point.z.max(min_b.z).min(max_b.z);

    let diff = closest_point - pos_s;
    let dist_sq = diff.length_squared();

    if dist_sq < sphere.radius * sphere.radius {
        let dist = dist_sq.sqrt();
        
        let (normal, penetration) = if dist > 0.0001 {
            let n = diff / dist;
            (n, sphere.radius - dist)
        } else {
            let diff_center = pos_aabb - pos_s;
            let n = diff_center.normalize();
            (n * -1.0, sphere.radius)
        };

        CollisionManifold { is_colliding: true, normal, penetration, contact_points: vec![closest_point] }
    } else {
        CollisionManifold { is_colliding: false, normal: Vec3::ZERO, penetration: 0.0, contact_points: vec![] }
    }
}

// ======================== KAPSÜL ÇARPIŞMA FONKSİYONLARI ========================

/// İki çizgi segmenti arasındaki en yakın noktaları bulur.
/// Döndürür: (t_a, t_b) — her parametrik t [0,1] aralığındadır.
/// p_a + t_a * d_a ve p_b + t_b * d_b en yakın noktaları verir.
fn closest_points_on_segments(
    p_a: Vec3, d_a: Vec3, // Segment A: başlangıç + yön (uç - başlangıç)
    p_b: Vec3, d_b: Vec3, // Segment B
) -> (f32, f32) {
    let r = p_a - p_b;
    let a = d_a.dot(d_a); // ||d_a||^2
    let e = d_b.dot(d_b); // ||d_b||^2
    let f = d_b.dot(r);

    if a < 1e-6 && e < 1e-6 {
        return (0.0, 0.0); // İki nokta
    }
    
    let (s, t);
    if a < 1e-6 {
        s = 0.0;
        t = (f / e).clamp(0.0, 1.0);
    } else {
        let c = d_a.dot(r);
        if e < 1e-6 {
            t = 0.0;
            s = (-c / a).clamp(0.0, 1.0);
        } else {
            let b = d_a.dot(d_b);
            let denom = a * e - b * b;
            
            s = if denom.abs() > 1e-6 {
                ((b * f - c * e) / denom).clamp(0.0, 1.0)
            } else {
                0.0
            };
            
            t = ((b * s + f) / e).clamp(0.0, 1.0);
        }
    }
    (s, t)
}

/// Kapsül-Kapsül çarpışma manifold'u.
/// Her kapsülün merkez segmenti bulunur, en yakın noktalar hesaplanır,
/// sonra iki küre çarpışmasına indirgenir.
pub fn check_capsule_capsule_manifold(
    pos_a: Vec3, rot_a: gizmo_math::Quat, cap_a: &Capsule,
    pos_b: Vec3, rot_b: gizmo_math::Quat, cap_b: &Capsule,
) -> CollisionManifold {
    // A kapsülünün dünya koordinatlarındaki üst ve alt merkezi
    let a_top = pos_a + rot_a.mul_vec3(Vec3::new(0.0, cap_a.half_height, 0.0));
    let a_bot = pos_a + rot_a.mul_vec3(Vec3::new(0.0, -cap_a.half_height, 0.0));
    let b_top = pos_b + rot_b.mul_vec3(Vec3::new(0.0, cap_b.half_height, 0.0));
    let b_bot = pos_b + rot_b.mul_vec3(Vec3::new(0.0, -cap_b.half_height, 0.0));

    let (t_a, t_b) = closest_points_on_segments(a_bot, a_top - a_bot, b_bot, b_top - b_bot);
    
    let closest_a = a_bot + (a_top - a_bot) * t_a;
    let closest_b = b_bot + (b_top - b_bot) * t_b;

    // İki küre çarpışmasına indirge
    check_sphere_sphere_manifold(
        closest_a, &Sphere { radius: cap_a.radius },
        closest_b, &Sphere { radius: cap_b.radius },
    )
}

/// Kapsül-Küre çarpışma manifold'u.
pub fn check_capsule_sphere_manifold(
    pos_cap: Vec3, rot_cap: gizmo_math::Quat, cap: &Capsule,
    pos_sphere: Vec3, sphere: &Sphere,
) -> CollisionManifold {
    let cap_top = pos_cap + rot_cap.mul_vec3(Vec3::new(0.0, cap.half_height, 0.0));
    let cap_bot = pos_cap + rot_cap.mul_vec3(Vec3::new(0.0, -cap.half_height, 0.0));
    
    // Kürenin merkezinin segmente en yakın noktası
    let seg = cap_top - cap_bot;
    let seg_len_sq = seg.length_squared();
    let t = if seg_len_sq > 1e-6 {
        ((pos_sphere - cap_bot).dot(seg) / seg_len_sq).clamp(0.0, 1.0)
    } else {
        0.5
    };
    let closest = cap_bot + seg * t;

    check_sphere_sphere_manifold(
        closest, &Sphere { radius: cap.radius },
        pos_sphere, sphere,
    )
}

/// Kapsül-AABB çarpışma manifold'u.
pub fn check_capsule_aabb_manifold(
    pos_cap: Vec3, rot_cap: gizmo_math::Quat, cap: &Capsule,
    pos_aabb: Vec3, aabb: &Aabb,
) -> CollisionManifold {
    let cap_top = pos_cap + rot_cap.mul_vec3(Vec3::new(0.0, cap.half_height, 0.0));
    let cap_bot = pos_cap + rot_cap.mul_vec3(Vec3::new(0.0, -cap.half_height, 0.0));
    
    // AABB'ye en yakın kapsül segmenti noktasını bul
    // Basit yaklaşım: segmentteki birkaç noktayı test et, en yakın olanı seç
    let min_b = pos_aabb - aabb.half_extents;
    let max_b = pos_aabb + aabb.half_extents;
    
    let mut best_dist_sq = f32::MAX;
    let mut best_cap_point = pos_cap;
    
    // Segmentten 5 nokta sample'la
    for i in 0..=4 {
        let t = i as f32 / 4.0;
        let p = cap_bot + (cap_top - cap_bot) * t;
        
        // Bu noktanın AABB'ye en yakın noktası
        let clamped = Vec3::new(
            p.x.max(min_b.x).min(max_b.x),
            p.y.max(min_b.y).min(max_b.y),
            p.z.max(min_b.z).min(max_b.z),
        );
        let dist_sq = (clamped - p).length_squared();
        if dist_sq < best_dist_sq {
            best_dist_sq = dist_sq;
            best_cap_point = p;
        }
    }

    // En yakın noktadan Sphere-AABB çarpışmasına indirge
    check_sphere_aabb_manifold(best_cap_point, &Sphere { radius: cap.radius }, pos_aabb, aabb)
}

