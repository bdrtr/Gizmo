use gizmo_math::{Vec3, Quat};
use crate::shape::ColliderShape;
use crate::gjk::{Simplex, SupportPoint, calculate_support};
use crate::collision::CollisionManifold;

/// EPA (Expanding Polytope Algorithm)
/// GJK kesişim bulduğunda (orijini içeren bir tetrahedron/Simplex), EPA bunu alır ve 
/// orijine en sığ olan çıkış yönünü ve derinliğini hesaplar. Orijin her zaman içeridedir.
pub fn epa_solve(
    initial_simplex: Simplex,
    shape_a: &ColliderShape, pos_a: Vec3, rot_a: Quat,
    shape_b: &ColliderShape, pos_b: Vec3, rot_b: Quat,
) -> CollisionManifold {
    // EPA, genişleyebilen bir poligon kullanır (Polytope). Başlangıçta bu GJK'den gelen Tetrahedron'dur.
    let mut polytope: Vec<SupportPoint> = initial_simplex.points[0..initial_simplex.size].to_vec();
    
    // GJK tetrahedron'undan başlayarak yüzeyleri (faces) index olarak oluşturuyoruz.
    // Her yüzey 3 noktadan (polytope indeksleri) oluşur. Culling (sarma) yönü dışarıya bakmalı!
    let mut faces = vec![
        0, 1, 2,
        0, 3, 1,
        0, 2, 3,
        1, 3, 2,
    ];


    for _ in 0..64 { // Maksimum iterasyon limiti
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
                
                faces.remove(i + 2);
                faces.remove(i + 1);
                faces.remove(i);
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
    let (closest_face_idx, face_normal, dist) = get_closest_face(&polytope, &faces);
    
    // === ÇOKLU TEMAS NOKTASI ÜRETİMİ (Multi-Point Contact Manifold) ===
    // EPA'nın Barycentric koordinatıyla TEK bir nokta üretmek yerine,
    // çarpışma normalinde ŞEKİL A'nın "temas yüzeyini" (support face) buluyoruz.
    //   → Kutu yüz-yüze çarptığında: 4 köşe = torklar karşılıklı iptal = DURAĞAN TEMAS!
    //   → Kenar çarpışması: 2 nokta = stabil
    //   → Köşe çarpışması: 1 nokta = fallback (eskisi gibi)
    
    let contact_points = generate_face_contacts(
        shape_a, pos_a, rot_a,
        shape_b, pos_b, rot_b,
        face_normal, dist,
        &polytope, &faces, closest_face_idx,
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
    shape_a: &ColliderShape, pos_a: Vec3, rot_a: Quat,
    _shape_b: &ColliderShape, _pos_b: Vec3, _rot_b: Quat,
    normal: Vec3, _penetration: f32,
    polytope: &[SupportPoint], faces: &[usize], closest_face_idx: usize,
) -> Vec<Vec3> {
    // Her iki şeklin temas yüzeyini bul, KÜÇÜK olanı kullan!
    // Normal A→B yönünde: A'nın face'i = normal yönünde, B'nin face'i = -normal yönünde
    let face_a = find_support_face(shape_a, pos_a, rot_a, normal);
    let face_b = find_support_face(_shape_b, _pos_b, _rot_b, normal * -1.0);
    
    // Küçük olan yüzeyi seç
    let face_verts = if face_a.len() >= 2 && face_b.len() >= 2 {
        let extent_a = face_extent(&face_a);
        let extent_b = face_extent(&face_b);
        if extent_a <= extent_b { face_a } else { face_b }
    } else if face_a.len() >= 2 {
        face_a
    } else if face_b.len() >= 2 {
        face_b
    } else {
        vec![]
    };
    
    if face_verts.len() >= 2 {
        // Yüz teması! Face vertex'lerinin AĞIRLIK MERKEZİNİ (centroid) tek temas noktası olarak kullan.
        // Bu, EPA'nın hatalı barycentric noktası yerine yüzeyin TAM ORTASInı verir.
        // Centroid = simetrik şekillerde NET TORK = 0 (takla yok!)
        let mut centroid = Vec3::ZERO;
        for v in &face_verts {
            centroid += *v;
        }
        centroid /= face_verts.len() as f32;
        return vec![centroid];
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
        ((d11 * d20 - d01 * d21) / denom, (d00 * d21 - d01 * d20) / denom)
    };
    let mut u_b = (1.0 - v_bary - w_bary).clamp(0.0, 1.0);
    let mut v_b = v_bary.clamp(0.0, 1.0);
    let mut w_b = w_bary.clamp(0.0, 1.0);
    let sum = u_b + v_b + w_b;
    if sum > 0.0001 { u_b /= sum; v_b /= sum; w_b /= sum; }
    else { u_b = 0.333; v_b = 0.333; w_b = 0.333; }
    let contact_point = a_sup.a * u_b + b_sup.a * v_b + c_sup.a * w_b;
    vec![contact_point]
}

/// Bir yüzeyin köşelerinin toplam yayılım alanını (bounding extent) hesapla.
/// Küçük yüzey = küçük extent = daha güvenli temas noktaları.
fn face_extent(verts: &[Vec3]) -> f32 {
    if verts.is_empty() { return f32::MAX; }
    let mut min = verts[0];
    let mut max = verts[0];
    for v in verts {
        min.x = min.x.min(v.x); min.y = min.y.min(v.y); min.z = min.z.min(v.z);
        max.x = max.x.max(v.x); max.y = max.y.max(v.y); max.z = max.z.max(v.z);
    }
    let diff = max - min;
    (diff.x * diff.x + diff.y * diff.y + diff.z * diff.z).sqrt()
}

/// Verilen yönde (dir) bir ConvexHull veya AABB'nin "temas yüzeyini" (support face) bul.
/// Yüzeyin tüm köşelerini dünya koordinatlarında döndürür.
/// Kutu için: yüz-yüze = 4 nokta, kenar = 2 nokta, köşe = 1 nokta
fn find_support_face(shape: &ColliderShape, pos: Vec3, rot: Quat, dir: Vec3) -> Vec<Vec3> {
    let dir_norm = if dir.length_squared() > 0.0001 { dir.normalize() } else { return vec![]; };
    
    match shape {
        ColliderShape::ConvexHull(hull) => {
            // Tüm köşeleri dünya koordinatına çevir ve normal yönünde projeksiyon hesapla
            let mut max_proj = f32::MIN;
            let mut world_verts: Vec<(Vec3, f32)> = Vec::with_capacity(hull.vertices.len());
            
            for v in &hull.vertices {
                let wv = pos + rot.mul_vec3(*v);
                let proj = wv.dot(dir_norm);
                if proj > max_proj { max_proj = proj; }
                world_verts.push((wv, proj));
            }
            
            // Max projeksiyona yakın tüm köşeler aynı "yüz" üzerinde
            // Tolerans: vertex boyutunun ~%5'i (küçük şekiller için 0.01, büyükler için daha geniş)
            let tolerance = 0.05;
            world_verts.iter()
                .filter(|(_, p)| max_proj - p < tolerance)
                .map(|(v, _)| *v)
                .collect()
        },
        ColliderShape::Aabb(aabb) => {
            let he = aabb.half_extents;
            // AABB'nin 8 köşesini oluştur (AABB rotasyon desteklemez)
            let corners = [
                pos + Vec3::new(-he.x, -he.y, -he.z),
                pos + Vec3::new( he.x, -he.y, -he.z),
                pos + Vec3::new( he.x,  he.y, -he.z),
                pos + Vec3::new(-he.x,  he.y, -he.z),
                pos + Vec3::new(-he.x, -he.y,  he.z),
                pos + Vec3::new( he.x, -he.y,  he.z),
                pos + Vec3::new( he.x,  he.y,  he.z),
                pos + Vec3::new(-he.x,  he.y,  he.z),
            ];
            
            let mut max_proj = f32::MIN;
            for c in &corners {
                let proj = c.dot(dir_norm);
                if proj > max_proj { max_proj = proj; }
            }
            
            let tolerance = 0.05;
            corners.iter()
                .filter(|c| max_proj - c.dot(dir_norm) < tolerance)
                .cloned()
                .collect()
        },
        // Küre/Kapsül analitik çözücüden geçer, buraya düşmemeli
        _ => vec![],
    }
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
        edges.remove(pos);
    } else {
        edges.push((a, b));
    }
}
