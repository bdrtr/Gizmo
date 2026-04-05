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

    let mut normal = Vec3::ZERO;
    let mut penetration = 0.0;

    for _ in 0..64 { // Maksimum iterasyon limiti
        // Orijine (0,0,0) en yakın yüzeyi bul
        let (_closest_face_idx, face_normal, dist) = get_closest_face(&polytope, &faces);
        
        // O yüzeyin normali yönünde yeni bir support noktası ara
        let support = calculate_support(shape_a, pos_a, rot_a, shape_b, pos_b, rot_b, face_normal);
        let support_dist = support.v.dot(face_normal);

        // Eğer yeni bulduğumuz nokta (support), o yüzeyin orijine olan uzaklığından öteye gidemiyorsa
        // (ya da çok çok az öteye gidiyorsa, toleransımız 0.001), 
        // sınırımıza ulaştık demektir! Bu yüzey dış sınırdır.
        if (support_dist - dist).abs() < 0.001 {
            normal = face_normal;
            penetration = dist;
            break;
        }

        // --- Eğer sınır değilse Polytope'u Genişlet ---

        // Yeni noktadan "görülebilen" tüm yüzeyleri kaldıracağız (Çünkü artık o yüzeyler içeride kalacak)
        // Bunun yerine yeni noktamızla birlikte yepyeni yüzeyler oluşturacağız.
        let mut edges = Vec::new();
        let mut i = 0;
        while i < faces.len() {
            let a = polytope[faces[i]];
            let b = polytope[faces[i + 1]];
            let c = polytope[faces[i + 2]];

            let n = (b.v - a.v).cross(c.v - a.v).normalize();
            
            // Eğer yeni support noktası bu yüzeyin "önündeyse" (noktadan yüzeye doğru gidiyorsak)
            if n.dot(support.v - a.v) > 0.0 {
                // Bu yüzeyi kopar. Kenarlarını (A-B, B-C, C-A) kaydet ki sonra tamir edebilelim.
                add_edge_if_unique(&mut edges, faces[i], faces[i + 1]);
                add_edge_if_unique(&mut edges, faces[i + 1], faces[i + 2]);
                add_edge_if_unique(&mut edges, faces[i + 2], faces[i]);
                
                faces.remove(i + 2);
                faces.remove(i + 1);
                faces.remove(i);
                // i artmıyor çünkü listeyi sildik
            } else {
                i += 3;
            }
        }

        // Koparılan delikleri yamalamak için support noktasını kullan 
        let support_idx = polytope.len();
        polytope.push(support);
        
        for (edge_a, edge_b) in edges {
            faces.push(support_idx);
            faces.push(edge_a);
            faces.push(edge_b);
        }
    }

    // --- Contact Point (Temas Noktası) Hesabı ---
    // En yakın yüzeyin normallerinden ve Support_a - Support_b verilerinden barycentric ağırlıklarla 
    // gerçek dünya temas noktasını çıkarabiliriz. Şimdilik orta noktadan basit bir çıkış alıyoruz
    let contact_point = pos_a + normal * (penetration * -0.5); 
    // Normal her zaman B'den A'ya itecek yönde olmalı
    // EPA'dan "A - B" çıkıyor. support_dist pozitif. 
    // (A + norm*pen) yaparsak B den uzaklaşır.

    CollisionManifold {
        is_colliding: true,
        normal,                // Çarpışma Normali
        penetration,           // Geçme Miktarı
        contact_points: vec![contact_point],
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
        if n.length_squared() < 0.0001 {
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
