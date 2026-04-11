use crate::shape::{Aabb, Capsule, Sphere};
use gizmo_math::Vec3;

// Hızlı ve Kirli Algılamalar (Broad Phase testleri için muazzam optimize)

#[inline]
pub fn test_aabb_aabb(pos_a: Vec3, aabb_a: &Aabb, pos_b: Vec3, aabb_b: &Aabb) -> bool {
    let min_a = pos_a - aabb_a.half_extents;
    let max_a = pos_a + aabb_a.half_extents;

    let min_b = pos_b - aabb_b.half_extents;
    let max_b = pos_b + aabb_b.half_extents;

    (min_a.x <= max_b.x && max_a.x >= min_b.x)
        && (min_a.y <= max_b.y && max_a.y >= min_b.y)
        && (min_a.z <= max_b.z && max_a.z >= min_b.z)
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
    pub contact_points: Vec<(Vec3, f32)>,
}

pub fn check_aabb_aabb_manifold(
    pos_a: Vec3,
    aabb_a: &Aabb,
    pos_b: Vec3,
    aabb_b: &Aabb,
) -> CollisionManifold {
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
        if normal.x != 0.0 {
            contact_point.x = pos_a.x + normal.x * a_ex.x;
        }
        if normal.y != 0.0 {
            contact_point.y = pos_a.y + normal.y * a_ex.y;
        }
        if normal.z != 0.0 {
            contact_point.z = pos_a.z + normal.z * a_ex.z;
        }

        CollisionManifold {
            is_colliding: true,
            normal,
            penetration: p,
            contact_points: vec![(contact_point, p)],
        }
    } else {
        CollisionManifold {
            is_colliding: false,
            normal: Vec3::ZERO,
            penetration: 0.0,
            contact_points: vec![],
        }
    }
}

pub fn check_sphere_sphere_manifold(
    pos_a: Vec3,
    s_a: &Sphere,
    pos_b: Vec3,
    s_b: &Sphere,
) -> CollisionManifold {
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

        CollisionManifold {
            is_colliding: true,
            normal,
            penetration,
            contact_points: vec![(contact_point, penetration)],
        }
    } else {
        CollisionManifold {
            is_colliding: false,
            normal: Vec3::ZERO,
            penetration: 0.0,
            contact_points: vec![],
        }
    }
}

pub fn check_sphere_aabb_manifold(
    pos_s: Vec3,
    sphere: &Sphere,
    pos_aabb: Vec3,
    aabb: &Aabb,
) -> CollisionManifold {
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

        CollisionManifold {
            is_colliding: true,
            normal,
            penetration,
            contact_points: vec![(closest_point, penetration)],
        }
    } else {
        CollisionManifold {
            is_colliding: false,
            normal: Vec3::ZERO,
            penetration: 0.0,
            contact_points: vec![],
        }
    }
}

pub fn check_sphere_obb_manifold(
    pos_s: Vec3,
    sphere: &Sphere,
    pos_obb: Vec3,
    rot_obb: gizmo_math::Quat,
    obb: &Aabb,
) -> CollisionManifold {
    // Küre merkezini OBB'nin yerel uzayına (Local Space) dönüştür
    let diff = pos_s - pos_obb;
    let local_s = rot_obb.inverse().mul_vec3(diff);

    // Yerel uzaydaki AABB'ye kenetle (Clamp)
    let closest_local = Vec3::new(
        local_s.x.clamp(-obb.half_extents.x, obb.half_extents.x),
        local_s.y.clamp(-obb.half_extents.y, obb.half_extents.y),
        local_s.z.clamp(-obb.half_extents.z, obb.half_extents.z),
    );

    let local_diff = local_s - closest_local;
    let dist_sq = local_diff.length_squared();

    if dist_sq < sphere.radius * sphere.radius {
        // En yakın noktayı dünya uzayına çevir
        let closest_world = pos_obb + rot_obb.mul_vec3(closest_local);

        let dist = dist_sq.sqrt();
        let (normal, penetration) = if dist > 0.0001 {
            let n = rot_obb.mul_vec3(local_diff / dist);
            (n, sphere.radius - dist)
        } else {
            // Tam merkezdeyse rastgele yön fırlat
            let n = rot_obb.mul_vec3(Vec3::new(0.0, 1.0, 0.0));
            (n, sphere.radius)
        };

        CollisionManifold {
            is_colliding: true,
            normal,
            penetration,
            contact_points: vec![(closest_world, penetration)],
        }
    } else {
        CollisionManifold {
            is_colliding: false,
            normal: Vec3::ZERO,
            penetration: 0.0,
            contact_points: vec![],
        }
    }
}

pub fn check_obb_obb_manifold(
    pos_a: Vec3,
    rot_a: gizmo_math::Quat,
    aabb_a: &Aabb,
    pos_b: Vec3,
    rot_b: gizmo_math::Quat,
    aabb_b: &Aabb,
) -> CollisionManifold {
    let axes_a = [
        rot_a.mul_vec3(Vec3::new(1.0, 0.0, 0.0)),
        rot_a.mul_vec3(Vec3::new(0.0, 1.0, 0.0)),
        rot_a.mul_vec3(Vec3::new(0.0, 0.0, 1.0)),
    ];
    let axes_b = [
        rot_b.mul_vec3(Vec3::new(1.0, 0.0, 0.0)),
        rot_b.mul_vec3(Vec3::new(0.0, 1.0, 0.0)),
        rot_b.mul_vec3(Vec3::new(0.0, 0.0, 1.0)),
    ];
    let t = pos_b - pos_a;
    let ea = aabb_a.half_extents;
    let eb = aabb_b.half_extents;

    let mut min_penetration = f32::MAX;
    let mut best_axis = Vec3::ZERO;

    let mut test_axes = Vec::with_capacity(15);
    test_axes.extend_from_slice(&axes_a);
    test_axes.extend_from_slice(&axes_b);
    for i in 0..3 {
        for j in 0..3 {
            let cross = axes_a[i].cross(axes_b[j]);
            if cross.length_squared() > 1e-6 {
                test_axes.push(cross.normalize());
            }
        }
    }

    for mut axis in test_axes {
        let ra = ea.x * axis.dot(axes_a[0]).abs()
            + ea.y * axis.dot(axes_a[1]).abs()
            + ea.z * axis.dot(axes_a[2]).abs();
        let rb = eb.x * axis.dot(axes_b[0]).abs()
            + eb.y * axis.dot(axes_b[1]).abs()
            + eb.z * axis.dot(axes_b[2]).abs();
        let dist = t.dot(axis).abs();
        let p = (ra + rb) - dist;
        if p < 0.0 {
            return CollisionManifold {
                is_colliding: false,
                normal: Vec3::ZERO,
                penetration: 0.0,
                contact_points: vec![],
            };
        }
        if p < min_penetration {
            min_penetration = p;
            if t.dot(axis) < 0.0 {
                axis *= -1.0;
            }
            best_axis = axis;
        }
    }

    // Referans yüzyü: A'nın best_axis yönündeki yüzü
    // Çok noktali temas: referans yüzyüzünü içeri aktarılan (incident) yüze göre clip et
    let contact_points = obb_obb_contact_points(
        pos_a,
        rot_a,
        ea,
        pos_b,
        rot_b,
        eb,
        best_axis,
        min_penetration,
    );

    CollisionManifold {
        is_colliding: true,
        normal: best_axis,
        penetration: min_penetration,
        contact_points,
    }
}

/// OBB-OBB çok noktali temas: referans yüzü clip ederek 1-4 temas noktası üretir.
fn obb_obb_contact_points(
    pos_a: Vec3,
    rot_a: gizmo_math::Quat,
    ea: Vec3,
    pos_b: Vec3,
    rot_b: gizmo_math::Quat,
    eb: Vec3,
    normal: Vec3,
    penetration: f32,
) -> Vec<(Vec3, f32)> {
    // A'nın normal yönündeki destek noktası (köşe)
    let local_dir_a = rot_a.inverse().mul_vec3(normal);
    let sup_a = pos_a
        + rot_a.mul_vec3(Vec3::new(
            if local_dir_a.x > 0.0 { ea.x } else { -ea.x },
            if local_dir_a.y > 0.0 { ea.y } else { -ea.y },
            if local_dir_a.z > 0.0 { ea.z } else { -ea.z },
        ));
    // B'nin -normal yönündeki destek noktası
    let local_dir_b = rot_b.inverse().mul_vec3(-normal);
    let sup_b = pos_b
        + rot_b.mul_vec3(Vec3::new(
            if local_dir_b.x > 0.0 { eb.x } else { -eb.x },
            if local_dir_b.y > 0.0 { eb.y } else { -eb.y },
            if local_dir_b.z > 0.0 { eb.z } else { -eb.z },
        ));

    // Referans yüzü eksenleri belirleme:
    // normal'e en yakın A eksenini bul (referans)
    let axes_a = [
        rot_a.mul_vec3(Vec3::X),
        rot_a.mul_vec3(Vec3::Y),
        rot_a.mul_vec3(Vec3::Z),
    ];
    let face_extents_a = [ea.x, ea.y, ea.z];
    let ref_axis_idx = (0..3)
        .max_by(|&i, &j| {
            axes_a[i]
                .dot(normal)
                .abs()
                .partial_cmp(&axes_a[j].dot(normal).abs())
                .unwrap()
        })
        .unwrap();

    // A'nın referans yüzü merkezini hesapla
    let ref_face_center = pos_a
        + axes_a[ref_axis_idx]
            * face_extents_a[ref_axis_idx]
            * normal.dot(axes_a[ref_axis_idx]).signum();
    let u = axes_a[(ref_axis_idx + 1) % 3];
    let v = axes_a[(ref_axis_idx + 2) % 3];
    let u_ext = face_extents_a[(ref_axis_idx + 1) % 3];
    let v_ext = face_extents_a[(ref_axis_idx + 2) % 3];

    // B'nin temas etki alanına giren köşelerini hesapla ve filtrele
    let axes_b = [
        rot_b.mul_vec3(Vec3::X),
        rot_b.mul_vec3(Vec3::Y),
        rot_b.mul_vec3(Vec3::Z),
    ];
    let face_extents_b = [eb.x, eb.y, eb.z];
    let inc_axis_idx = (0..3)
        .max_by(|&i, &j| {
            axes_b[i]
                .dot(-normal)
                .abs()
                .partial_cmp(&axes_b[j].dot(-normal).abs())
                .unwrap()
        })
        .unwrap();
    let inc_face_center = pos_b
        + axes_b[inc_axis_idx]
            * face_extents_b[inc_axis_idx]
            * (-normal).dot(axes_b[inc_axis_idx]).signum();
    let iu = axes_b[(inc_axis_idx + 1) % 3];
    let iv = axes_b[(inc_axis_idx + 2) % 3];
    let iu_ext = face_extents_b[(inc_axis_idx + 1) % 3];
    let iv_ext = face_extents_b[(inc_axis_idx + 2) % 3];

    // İncident yüz köşeleri
    let corners = [
        inc_face_center + iu * iu_ext + iv * iv_ext,
        inc_face_center - iu * iu_ext + iv * iv_ext,
        inc_face_center - iu * iu_ext - iv * iv_ext,
        inc_face_center + iu * iu_ext - iv * iv_ext,
    ];

    // Referans yüze clip et ve penetrasyon pozitif olan noktaları al
    let mut result = Vec::with_capacity(4);
    for &corner in &corners {
        let local = corner - ref_face_center;
        let du = local.dot(u);
        let dv = local.dot(v);
        // Köşe referans yüz ız(e)düşümünde mi?
        if du.abs() <= u_ext + 0.001 && dv.abs() <= v_ext + 0.001 {
            // Penetrasyon derinliği: normal yönünde ne kadar içeri girmiş
            let dep = penetration.min((ref_face_center - corner).dot(normal) + penetration);
            if dep >= 0.0 {
                result.push((corner, dep.max(0.001)));
            }
        }
    }

    // Hiç köşe yoksa orta nokta fallback
    if result.is_empty() {
        result.push(((sup_a + sup_b) * 0.5, penetration));
    }
    result
}

// ======================== KAPSÜL ÇARPIŞMA FONKSİYONLARI ========================

/// İki çizgi segmenti arasındaki en yakın noktaları bulur.
/// Döndürür: (t_a, t_b) — her parametrik t [0,1] aralığındadır.
/// p_a + t_a * d_a ve p_b + t_b * d_b en yakın noktaları verir.
fn closest_points_on_segments(
    p_a: Vec3,
    d_a: Vec3, // Segment A: başlangıç + yön (uç - başlangıç)
    p_b: Vec3,
    d_b: Vec3, // Segment B
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
    pos_a: Vec3,
    rot_a: gizmo_math::Quat,
    cap_a: &Capsule,
    pos_b: Vec3,
    rot_b: gizmo_math::Quat,
    cap_b: &Capsule,
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
        closest_a,
        &Sphere {
            radius: cap_a.radius,
        },
        closest_b,
        &Sphere {
            radius: cap_b.radius,
        },
    )
}

/// Kapsül-Küre çarpışma manifold'u.
pub fn check_capsule_sphere_manifold(
    pos_cap: Vec3,
    rot_cap: gizmo_math::Quat,
    cap: &Capsule,
    pos_sphere: Vec3,
    sphere: &Sphere,
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

    check_sphere_sphere_manifold(closest, &Sphere { radius: cap.radius }, pos_sphere, sphere)
}

/// Kapsül-AABB çarpışma manifold'u.
pub fn check_capsule_aabb_manifold(
    pos_cap: Vec3,
    rot_cap: gizmo_math::Quat,
    cap: &Capsule,
    pos_aabb: Vec3,
    aabb: &Aabb,
) -> CollisionManifold {
    let cap_top = pos_cap + rot_cap.mul_vec3(Vec3::new(0.0, cap.half_height, 0.0));
    let cap_bot = pos_cap + rot_cap.mul_vec3(Vec3::new(0.0, -cap.half_height, 0.0));

    // Analitik segment-AABB en yakın nokta hesabı:
    // 1. Segment üzerindeki her noktanın AABB'ye yakınlığını parametrik t ile bul
    // 2. En yakın (t, clamped_point) çiftini seç
    let min_b = pos_aabb - aabb.half_extents;
    let max_b = pos_aabb + aabb.half_extents;
    let seg_dir = cap_top - cap_bot;
    let seg_len_sq = seg_dir.length_squared();

    let best_cap_point = if seg_len_sq < 0.0001 {
        // Degenerate kapsül (nokta) — merkez kullan
        pos_cap
    } else {
        // AABB merkezine en yakın segment noktasını başlangıç tahmini olarak kullan
        // sonra iteratif olarak iyileştir (2 adım yeterli — Voronoi bölge yakınsaması)
        let mut best_t = ((pos_aabb - cap_bot).dot(seg_dir) / seg_len_sq).clamp(0.0, 1.0);

        for _ in 0..3 {
            let seg_pt = cap_bot + seg_dir * best_t;
            // Bu segment noktasının AABB'ye en yakın noktası
            let clamped = Vec3::new(
                seg_pt.x.max(min_b.x).min(max_b.x),
                seg_pt.y.max(min_b.y).min(max_b.y),
                seg_pt.z.max(min_b.z).min(max_b.z),
            );
            // Bu AABB noktasına en yakın segment noktasını bul (ters yönlü projeksiyon)
            best_t = ((clamped - cap_bot).dot(seg_dir) / seg_len_sq).clamp(0.0, 1.0);
        }

        cap_bot + seg_dir * best_t
    };

    // En yakın noktadan Sphere-AABB çarpışmasına indirge
    check_sphere_aabb_manifold(
        best_cap_point,
        &Sphere { radius: cap.radius },
        pos_aabb,
        aabb,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_broad_phase_aabb_aabb() {
        let aabb1 = Aabb {
            half_extents: Vec3::new(1.0, 1.0, 1.0),
        };
        let aabb2 = Aabb {
            half_extents: Vec3::new(1.0, 1.0, 1.0),
        };

        assert!(test_aabb_aabb(
            Vec3::ZERO,
            &aabb1,
            Vec3::new(1.0, 0.0, 0.0),
            &aabb2
        ));
        assert!(!test_aabb_aabb(
            Vec3::ZERO,
            &aabb1,
            Vec3::new(3.0, 0.0, 0.0),
            &aabb2
        ));
    }

    #[test]
    fn test_broad_phase_sphere_sphere() {
        let s1 = Sphere { radius: 1.0 };
        let s2 = Sphere { radius: 1.0 };

        assert!(test_sphere_sphere(
            Vec3::ZERO,
            &s1,
            Vec3::new(1.5, 0.0, 0.0),
            &s2
        ));
        assert!(!test_sphere_sphere(
            Vec3::ZERO,
            &s1,
            Vec3::new(2.5, 0.0, 0.0),
            &s2
        ));
    }

    #[test]
    fn test_sphere_sphere_manifold() {
        let s1 = Sphere { radius: 1.0 };
        let s2 = Sphere { radius: 1.0 };

        let manifold = check_sphere_sphere_manifold(Vec3::ZERO, &s1, Vec3::new(1.5, 0.0, 0.0), &s2);
        assert!(manifold.is_colliding);
        // assert!((manifold.penetration - 0.5).abs() < 0.001);
        assert_eq!(manifold.contact_points.len(), 1);
    }
}
