use crate::collision::CollisionManifold;
use crate::gjk::{calculate_support, Simplex, SupportPoint};
use crate::shape::ColliderShape;
use gizmo_math::{Quat, Vec3};

/// EPA (Expanding Polytope Algorithm)
/// GJK kesişim bulduğunda (orijini içeren bir tetrahedron/Simplex), EPA bunu alır ve
/// orijine en sığ olan çıkış yönünü ve derinliğini hesaplar. Orijin her zaman içeridedir.
pub fn epa_solve(
    initial_simplex: Simplex,
    shape_a: &ColliderShape,
    pos_a: Vec3,
    rot_a: Quat,
    shape_b: &ColliderShape,
    pos_b: Vec3,
    rot_b: Quat,
) -> CollisionManifold {
    // EPA, genişleyebilen bir poligon kullanır (Polytope). Başlangıçta bu GJK'den gelen Tetrahedron'dur.
    let mut polytope: Vec<SupportPoint> = initial_simplex.points[0..initial_simplex.size].to_vec();

    // GJK tetrahedron'undan başlayarak yüzeyleri (faces) index olarak oluşturuyoruz.
    // Her yüzey 3 noktadan (polytope indeksleri) oluşur. Culling (sarma) yönü dışarıya bakmalı!
    let mut faces = vec![0, 1, 2, 0, 3, 1, 0, 2, 3, 1, 3, 2];

    for _ in 0..64 {
        // Maksimum iterasyon limiti
        // Orijine (0,0,0) en yakın yüzeyi bul
        let (_closest_face_idx, face_normal, dist) = get_closest_face(&polytope, &faces);

        // Döngü her döndüğünde en iyi tahmini yakalarız. Limit aşılırsa alt tarafta tekrar 'get_closest_face' yapılarak güncel data çekilir.

        // O yüzeyin normali yönünde yeni bir support noktası ara
        let support = calculate_support(shape_a, pos_a, rot_a, shape_b, pos_b, rot_b, face_normal);
        let support_dist = support.v.dot(face_normal);

        // Eğer yeni bulduğumuz nokta (support), o yüzeyin orijine olan uzaklığından öteye gidemiyorsa
        // (ya da çok çok az öteye gidiyorsa, toleransımız 0.005),
        // sınırımıza ulaştık demektir! Bu yüzey dış sınırdır.
        if (support_dist - dist).abs() < 0.005 {
            break;
        }

        // --- Eğer sınır değilse Polytope'u Genişlet ---
        let mut edges = Vec::new();
        let mut i = 0;
        while i < faces.len() {
            let a = polytope[faces[i]];
            let b = polytope[faces[i + 1]];
            let c = polytope[faces[i + 2]];

            let mut n = (b.v - a.v).cross(c.v - a.v);
            if n.length_squared() > 1e-8 {
                n = n.normalize();
            } else {
                n = face_normal;
            }

            // Eğer yeni support noktası bu yüzeyin "önündeyse" (noktadan yüzeye doğru gidiyorsak)
            if n.dot(support.v - a.v) > 1e-6 {
                add_edge_if_unique(&mut edges, faces[i], faces[i + 1]);
                add_edge_if_unique(&mut edges, faces[i + 1], faces[i + 2]);
                add_edge_if_unique(&mut edges, faces[i + 2], faces[i]);

                let last_idx = faces.len() - 3;
                faces.swap(i, last_idx);
                faces.swap(i + 1, last_idx + 1);
                faces.swap(i + 2, last_idx + 2);
                faces.truncate(last_idx);
            } else {
                i += 3;
            }
        }

        // Eğer hiçbir yüzey kırılamadıysa sonsuz döngüyü kır
        if edges.is_empty() {
            break;
        }

        let support_idx = polytope.len();
        polytope.push(support);

        for (edge_a, edge_b) in edges {
            faces.push(support_idx);
            faces.push(edge_a);
            faces.push(edge_b);
        }
    }

    // Döngü bittiğinde en yakın yüzeyin verilerini alıyoruz
    if faces.is_empty() {
        return CollisionManifold {
            is_colliding: false,
            normal: Vec3::ZERO,
            penetration: 0.0,
            contact_points: vec![],
        };
    }
    let (closest_face_idx, face_normal, dist) = get_closest_face(&polytope, &faces);

    // === ÇOKLU TEMAS NOKTASI ÜRETİMİ (Multi-Point Contact Manifold) ===
    // EPA'nın Barycentric koordinatıyla TEK bir nokta üretmek yerine,
    // çarpışma normalinde ŞEKİL A'nın "temas yüzeyini" (support face) buluyoruz.
    //   → Kutu yüz-yüze çarptığında: 4 köşe = torklar karşılıklı iptal = DURAĞAN TEMAS!
    //   → Kenar çarpışması: 2 nokta = stabil
    //   → Köşe çarpışması: 1 nokta = fallback (eskisi gibi)

    let contact_points = generate_face_contacts(
        shape_a,
        pos_a,
        rot_a,
        shape_b,
        pos_b,
        rot_b,
        face_normal,
        dist,
        &polytope,
        &faces,
        closest_face_idx,
    );

    CollisionManifold {
        is_colliding: true,
        normal: face_normal,
        penetration: dist,
        contact_points,
    }
}

/// Çarpışma normalinde Shape A'nın temas yüzeyini bularak çoklu temas noktası üret.
/// Kutu gibi şekillerde yüz-yüze (face-to-face) çarpışmalarda 4 nokta döner.
fn generate_face_contacts(
    shape_a: &ColliderShape,
    pos_a: Vec3,
    rot_a: Quat,
    shape_b: &ColliderShape,
    pos_b: Vec3,
    rot_b: Quat,
    normal: Vec3,
    _penetration: f32,
    polytope: &[SupportPoint],
    faces: &[usize],
    closest_face_idx: usize,
) -> Vec<(Vec3, f32)> {
    // Her iki şeklin temas yüzeyini bul, KÜÇÜK olanı kullan!
    // Normal A→B yönünde: A'nın face'i = normal yönünde, B'nin face'i = -normal yönünde
    let (face_a, normal_a) = find_support_face(shape_a, pos_a, rot_a, normal);
    let (face_b, normal_b) = find_support_face(shape_b, pos_b, rot_b, normal * -1.0);

    // Sutherland-Hodgman Kırpması için Reference (Ref) ve Incident (Inc) Yüzey Seçimi
    // Reference yüzü, normali bizim arama yönümüzle en uyumlu (ve kapladığı alanı daha uygun) yüzdür.
    let extent_a = face_extent(&face_a);
    let extent_b = face_extent(&face_b);

    // B her zaman Incident Face, A Reference Face gibi ele alacağız
    // Ama B'den daha dik ve güçlü bir Support Face gelirse değiştiririz.
    // İhtiyacımız olan Contact noktalarını "reference yüzeyine" düşen noktalar oluşturur.
    let (ref_face, mut inc_face, ref_normal) = if extent_a <= extent_b {
        // A'yı Reference Face yap (Normal A'dan B'ye)
        (face_a.clone(), face_b.clone(), normal_a)
    } else {
        // B'yi Reference Face yap (Normal B'dan A'ya, dikkat et yönü ters!)
        (face_b.clone(), face_a.clone(), normal_b)
    };

    if ref_face.len() >= 3 && inc_face.len() >= 3 {
        // TAM YÜZEY-YÜZEY TEMASI -> SUTHERLAND HODGMAN KESİŞİMİ (Poligon Kırpma)
        for i in 0..ref_face.len() {
            let a = ref_face[i];
            let b = ref_face[(i + 1) % ref_face.len()];
            let edge_dir = (b - a).normalize();

            // Eğer Yüzey köşeleri Saat Yönünün Tersine (CCW) dönüyorsa, Inward normal şu şekilde hesaplanır:
            let inward_normal = ref_normal.cross(edge_dir).normalize();

            inc_face = clip_polygon_against_plane(&inc_face, a, inward_normal);
        }

        // Clip sonrası dışarı taşan (Reference yüzeyinin önünde kalan) noktaları filtrele
        // Sadece içeri giren (penetration) noktaları Manifold olarak kabul et
        let mut final_contacts = Vec::new();
        // inc_face şuan kırpma (clipping) sınırlarını geçen noktaları barındırır.
        for pt in &inc_face {
            let dist = (*pt - ref_face[0]).dot(ref_normal);

            // distance <= 0.05 anlamına gelir ki noktanın yüksekliği referans düzleminin altına girmiş
            if dist <= 0.05 {
                final_contacts.push((*pt, -dist.min(0.0)));
            }
        }

        if final_contacts.len() > 0 {
            return final_contacts;
        } else {
            // Clipping produced 0 points, return center of ref_face to avoid stale index panic in fallback
            let mut center = Vec3::ZERO;
            for p in &ref_face {
                center += *p;
            }
            if !ref_face.is_empty() {
                center /= ref_face.len() as f32;
            }
            return vec![(center, _penetration)];
        }
    } else if ref_face.len() >= 2 {
        // Kenar - Yüzey veya benzer azınlık temasında direkt orijinalini dön, hepsine uniform pen ver.
        return ref_face.into_iter().map(|v| (v, _penetration)).collect();
    }

    // Köşe teması veya küre gibi yuvarlak şekil → tek noktaya fallback (eski yöntem)
    let a_sup = polytope[faces[closest_face_idx]];
    let b_sup = polytope[faces[closest_face_idx + 1]];
    let c_sup = polytope[faces[closest_face_idx + 2]];

    let p_proj = normal * _penetration;
    let v0 = b_sup.v - a_sup.v;
    let v1 = c_sup.v - a_sup.v;
    let v2 = p_proj - a_sup.v;
    let d00 = v0.dot(v0);
    let d01 = v0.dot(v1);
    let d11 = v1.dot(v1);
    let d20 = v2.dot(v0);
    let d21 = v2.dot(v1);
    let denom = d00 * d11 - d01 * d01;
    let (v_bary, w_bary) = if denom.abs() < 1e-8 {
        (0.333, 0.333)
    } else {
        (
            (d11 * d20 - d01 * d21) / denom,
            (d00 * d21 - d01 * d20) / denom,
        )
    };
    let mut u_b = (1.0 - v_bary - w_bary).clamp(0.0, 1.0);
    let mut v_b = v_bary.clamp(0.0, 1.0);
    let mut w_b = w_bary.clamp(0.0, 1.0);
    let sum = u_b + v_b + w_b;
    if sum > 0.0001 {
        u_b /= sum;
        v_b /= sum;
        w_b /= sum;
    } else {
        u_b = 0.333;
        v_b = 0.333;
        w_b = 0.333;
    }
    let contact_point = a_sup.a * u_b + b_sup.a * v_b + c_sup.a * w_b;
    vec![(contact_point, _penetration)]
}

/// Bir yüzeyin köşelerinin toplam yayılım alanını (bounding extent) hesapla.
/// Küçük yüzey = küçük extent = daha güvenli temas noktaları.
fn face_extent(verts: &[Vec3]) -> f32 {
    if verts.is_empty() {
        return f32::MAX;
    }
    let mut min = verts[0];
    let mut max = verts[0];
    for v in verts {
        min.x = min.x.min(v.x);
        min.y = min.y.min(v.y);
        min.z = min.z.min(v.z);
        max.x = max.x.max(v.x);
        max.y = max.y.max(v.y);
        max.z = max.z.max(v.z);
    }
    let diff = max - min;
    (diff.x * diff.x + diff.y * diff.y + diff.z * diff.z).sqrt()
}

/// Verilen yönde (dir) bir ConvexHull veya AABB'nin "temas yüzeyini" (support face) bul.
/// Yüzeyin tüm köşelerini dünya koordinatlarında döndürür.
/// Kutu için: yüz-yüze = 4 nokta, kenar = 2 nokta, köşe = 1 nokta
fn find_support_face(shape: &ColliderShape, pos: Vec3, rot: Quat, dir: Vec3) -> (Vec<Vec3>, Vec3) {
    let dir_norm = if dir.length_squared() > 0.0001 {
        dir.normalize()
    } else {
        return (vec![], Vec3::ZERO);
    };

    match shape {
        ColliderShape::ConvexHull(hull) => {
            let mut max_proj = f32::MIN;
            let mut world_verts: Vec<(Vec3, f32)> = Vec::with_capacity(hull.vertices.len());

            for v in &hull.vertices {
                let wv = pos + rot.mul_vec3(*v);
                let proj = wv.dot(dir_norm);
                if proj > max_proj {
                    max_proj = proj;
                }
                world_verts.push((wv, proj));
            }

            let extent = hull.vertices.iter().fold(0.0f32, |acc, v| acc.max(v.length_squared())).sqrt();
            let tolerance = (extent * 0.05).max(0.01);
            let verts: Vec<Vec3> = world_verts
                .iter()
                .filter(|(_, p)| max_proj - p < tolerance)
                .map(|(v, _)| *v)
                .collect();
            // Yüzeysel Normal varsayımı: dir_norm
            (verts, dir_norm)
        }
        ColliderShape::Aabb(aabb) => {
            let he = aabb.half_extents;
            // OBY'ye (OBB) göre Lokal Arama Yönünü bul
            let local_dir = rot.inverse().mul_vec3(dir_norm);
            let abs_x = local_dir.x.abs();
            let abs_y = local_dir.y.abs();
            let abs_z = local_dir.z.abs();

            let mut local_corners = Vec::with_capacity(4);
            let local_normal;

            // Köşeleri Saat Yönünün Tersine (CCW) eklemek hayati önem taşır (Sutherland-Hodgman için)
            if abs_x >= abs_y && abs_x >= abs_z {
                let sign = local_dir.x.signum();
                local_normal = Vec3::new(sign, 0.0, 0.0);
                if sign > 0.0 {
                    local_corners.push(Vec3::new(he.x, he.y, -he.z));
                    local_corners.push(Vec3::new(he.x, he.y, he.z));
                    local_corners.push(Vec3::new(he.x, -he.y, he.z));
                    local_corners.push(Vec3::new(he.x, -he.y, -he.z));
                } else {
                    local_corners.push(Vec3::new(-he.x, he.y, he.z));
                    local_corners.push(Vec3::new(-he.x, he.y, -he.z));
                    local_corners.push(Vec3::new(-he.x, -he.y, -he.z));
                    local_corners.push(Vec3::new(-he.x, -he.y, he.z));
                }
            } else if abs_y >= abs_x && abs_y >= abs_z {
                let sign = local_dir.y.signum();
                local_normal = Vec3::new(0.0, sign, 0.0);
                if sign > 0.0 {
                    local_corners.push(Vec3::new(-he.x, he.y, -he.z));
                    local_corners.push(Vec3::new(-he.x, he.y, he.z));
                    local_corners.push(Vec3::new(he.x, he.y, he.z));
                    local_corners.push(Vec3::new(he.x, he.y, -he.z));
                } else {
                    local_corners.push(Vec3::new(-he.x, -he.y, he.z));
                    local_corners.push(Vec3::new(-he.x, -he.y, -he.z));
                    local_corners.push(Vec3::new(he.x, -he.y, -he.z));
                    local_corners.push(Vec3::new(he.x, -he.y, he.z));
                }
            } else {
                let sign = local_dir.z.signum();
                local_normal = Vec3::new(0.0, 0.0, sign);
                if sign > 0.0 {
                    local_corners.push(Vec3::new(-he.x, he.y, he.z));
                    local_corners.push(Vec3::new(-he.x, -he.y, he.z));
                    local_corners.push(Vec3::new(he.x, -he.y, he.z));
                    local_corners.push(Vec3::new(he.x, he.y, he.z));
                } else {
                    local_corners.push(Vec3::new(-he.x, -he.y, -he.z));
                    local_corners.push(Vec3::new(-he.x, he.y, -he.z));
                    local_corners.push(Vec3::new(he.x, he.y, -he.z));
                    local_corners.push(Vec3::new(he.x, -he.y, -he.z));
                }
            }

            // Tüm CCW köşelerini GERÇEK dünya uzayına çevir (Rotasyonu hesaba katarak)
            let world_corners: Vec<Vec3> = local_corners
                .iter()
                .map(|v| pos + rot.mul_vec3(*v))
                .collect();
            let world_normal = rot.mul_vec3(local_normal);

            (world_corners, world_normal)
        }
        _ => (vec![], Vec3::ZERO),
    }
}

fn clip_polygon_against_plane(
    polygon: &[Vec3],
    plane_point: Vec3,
    inward_normal: Vec3,
) -> Vec<Vec3> {
    if polygon.is_empty() {
        return vec![];
    }
    let mut clipped = Vec::new();
    let mut prev_v = polygon.last().unwrap();
    let mut prev_d = (*prev_v - plane_point).dot(inward_normal);

    for v in polygon {
        let curr_d = (*v - plane_point).dot(inward_normal);

        if prev_d >= 0.0 {
            if curr_d >= 0.0 {
                clipped.push(*v);
            } else {
                let t = prev_d / (prev_d - curr_d);
                clipped.push(*prev_v + (*v - *prev_v) * t);
            }
        } else {
            if curr_d >= 0.0 {
                let t = prev_d / (prev_d - curr_d);
                clipped.push(*prev_v + (*v - *prev_v) * t);
                clipped.push(*v);
            }
        }
        prev_v = v;
        prev_d = curr_d;
    }
    clipped
}

/// Tüm yüzeyler arasında Orijin'e (0,0,0) en yakın olan üçgen yüzeyi bulur.
/// (Yüzey indeksini, yüzey normalini ve en kısa mesafeyi döner)
fn get_closest_face(polytope: &[SupportPoint], faces: &[usize]) -> (usize, Vec3, f32) {
    let mut min_dist = f32::MAX;
    let mut closest_face = 0;
    let mut closest_normal = Vec3::ZERO;

    for i in (0..faces.len()).step_by(3) {
        let a = polytope[faces[i]].v;
        let b = polytope[faces[i + 1]].v;
        let c = polytope[faces[i + 2]].v;

        let mut n = (b - a).cross(c - a);
        if n.length_squared() < 1e-8 {
            continue; // Dejenere üçgen (bozuk normal)
        }
        n = n.normalize();

        let dist = n.dot(a);

        // Eğer yüzeyin sarma yönü hatalıysa (normal orijine bakıyorsa) ters çevir
        let (dist, n) = if dist < 0.0 {
            (-dist, n * -1.0)
        } else {
            (dist, n)
        };

        if dist < min_dist {
            min_dist = dist;
            closest_face = i;
            closest_normal = n;
        }
    }

    (closest_face, closest_normal, min_dist)
}

/// Kenar yönetim sistemi: İki bitişik yüzey tamamen kaldırılırsa ortak kenarları da çöpe gider.
/// O yüzden unique (tekrar etmeyen) kenarları tutmak, "delikleri" tespit etmemizi sağlar.
fn add_edge_if_unique(edges: &mut Vec<(usize, usize)>, a: usize, b: usize) {
    // Aynı kenar zıt yönde var mı (b, a)? Varsa onu sil, demek ki bu iç kenar! Biz dış delik arıyoruz.
    if let Some(pos) = edges.iter().position(|&(ea, eb)| ea == b && eb == a) {
        edges.swap_remove(pos);
    } else {
        edges.push((a, b));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gjk::gjk_intersect;
    use crate::shape::{ColliderShape, Sphere};

    #[test]
    fn test_epa_sphere_penetration() {
        let shape_a = ColliderShape::Sphere(Sphere { radius: 1.0 });
        let pos_a = Vec3::new(0.0, 0.0, 0.0);
        let rot_a = Quat::IDENTITY;

        let shape_b = ColliderShape::Sphere(Sphere { radius: 1.0 });
        let pos_b = Vec3::new(1.5, 0.0, 0.0);
        let rot_b = Quat::IDENTITY;

        let (intersect, simplex) = gjk_intersect(&shape_a, pos_a, rot_a, &shape_b, pos_b, rot_b);
        assert!(intersect, "Spheres should intersect for EPA to run");

        let manifold = epa_solve(simplex, &shape_a, pos_a, rot_a, &shape_b, pos_b, rot_b);

        assert!(manifold.is_colliding);
        // They are 1.5 units apart in X, radius sum is 2.0. Penetration should be 0.5.
        // Normal should point from B to A or A to B depending on who is first, typically away from origin in Minkowski difference context.
        // In this implementation, normal is 'face_normal' from Minkowski difference (A-B).
        // A is at 0, B is at 1.5. A - B is at -1.5. So origin is "inside" Minkowski diff.
        // The closest boundary of Minkowski diff to origin is at X = 0.5 (penetration depth).
        // Let's verify the distance is close to 0.5.
        assert!(
            (manifold.penetration - 0.5).abs() < 0.05,
            "Penetration depth should be approx 0.5, got {}",
            manifold.penetration
        );

        // Normal should be along X axis
        assert!(manifold.normal.x.abs() > 0.95);
        assert!(manifold.normal.y.abs() < 0.05);
        assert!(manifold.normal.z.abs() < 0.05);
    }
}
