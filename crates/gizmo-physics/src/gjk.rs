use crate::shape::ColliderShape;
use gizmo_math::{Quat, Vec3};

/// Minkowski Farkındaki (Minkowski Difference) bir nokta
#[derive(Debug, Clone, Copy)]
pub struct SupportPoint {
    pub v: Vec3, // A'nın support - B'nin support (Fark vektörü)
    // EPA sırasında hangi noktalardan geldiğini hatırlamak için A ve B noktalarını da tutabiliriz
    pub a: Vec3,
    pub b: Vec3,
}

impl SupportPoint {
    pub fn new(a: Vec3, b: Vec3) -> Self {
        Self { v: a - b, a, b }
    }
}

impl Default for SupportPoint {
    fn default() -> Self {
        Self {
            v: Vec3::ZERO,
            a: Vec3::ZERO,
            b: Vec3::ZERO,
        }
    }
}

/// GJK Simplex — En fazla 4 noktadan (Tetrahedron) oluşan basit şekil
pub struct Simplex {
    pub points: [SupportPoint; 4],
    pub size: usize,
}

impl Simplex {
    pub fn new() -> Self {
        Self {
            points: [SupportPoint::default(); 4],
            size: 0,
        }
    }

    pub fn push(&mut self, point: SupportPoint) {
        // Yeni noktayı her zaman VEC'in BAŞINA (0. index) ekleriz.
        // Diğerlerini birer sağa kaydır. Bu sayede en son eklenen nokta her zaman ilk sırada olur.
        self.points[3] = self.points[2];
        self.points[2] = self.points[1];
        self.points[1] = self.points[0];
        self.points[0] = point;
        self.size = (self.size + 1).min(4);
    }
}

impl Default for Simplex {
    fn default() -> Self {
        Self::new()
    }
}

/// GJK için Yardımcı Support Fonsiyonu
/// A - B fark vektörünün verilen dir yönündeki en uç noktasını hesaplar.
pub fn calculate_support(
    shape_a: &ColliderShape,
    pos_a: Vec3,
    rot_a: Quat,
    shape_b: &ColliderShape,
    pos_b: Vec3,
    rot_b: Quat,
    dir: Vec3,
) -> SupportPoint {
    let p_a = shape_a.support_point(pos_a, rot_a, dir);
    let p_b = shape_b.support_point(pos_b, rot_b, dir * -1.0); // Zıt yön

    SupportPoint::new(p_a, p_b)
}

/// GJK Çarpışma Tespit Algoritması
/// İki şeklin (shape_a, shape_b) kesişip kesişmediğini döndürür.
/// Kesişiyorsa, EPA için kullanılacak Simplex'i de döndürür.
pub fn gjk_intersect(
    shape_a: &ColliderShape,
    pos_a: Vec3,
    rot_a: Quat,
    shape_b: &ColliderShape,
    pos_b: Vec3,
    rot_b: Quat,
) -> (bool, Simplex) {
    let mut simplex = Simplex::new();

    // Gelişigüzel bir başlangıç yönü (iki merkez arası vektör mantıklıdır)
    let mut dir = pos_b - pos_a;
    if dir.length_squared() < 0.0001 {
        dir = Vec3::new(1.0, 0.0, 0.0);
    }

    // İlk noktayı al ve ekle
    let support = calculate_support(shape_a, pos_a, rot_a, shape_b, pos_b, rot_b, dir);
    simplex.push(support);

    // Yeni arama yönü: support noktasından orijine doğru
    dir = support.v * -1.0;

    // Uzayda sonsuz döngüyü önlemek için iterasyon limiti
    for _iter in 0..64 {
        let a = calculate_support(shape_a, pos_a, rot_a, shape_b, pos_b, rot_b, dir);

        // Eğer bulduğumuz nokta aradığımız yönde orijini (0,0,0) geçemiyorsa, kesişim imkansızdır.
        // Hassasiyet (epsilon) eklendi, böylece yüzey temaslarında erken pes etmez
        if a.v.dot(dir) < -0.0001 {
            return (false, simplex);
        }

        simplex.push(a);

        // handle_simplex hem yeni yönü bulur, hem de gerekirse gereksiz noktaları siler
        if handle_simplex(&mut simplex, &mut dir) {
            return (true, simplex); // Orijini içerdiğini anladık!
        }
    }

    // 64 iterasyon tükendi — GJK yakınsayamadı.
    // Bu genellikle çok karmaşık / sayısal olarak hassas ConvexHull çiftlerinde oluşur.
    // EPA'ya geçilmeden önce silent failure yerine uyarı verilir.
    #[cfg(debug_assertions)]
    eprintln!(
        "[GJK WARN] 64 iterasyon tükendi, yakınsama başarısız. \
         Karmaşık ConvexHull çiftleri veya degenerate şekiller kontrol edilmeli."
    );

    (false, simplex)
}

/// Simplex boyutuna göre gerekli matematiği uygular ve yeni dir hesaplar.
/// Simplex orijini kapsıyorsa true döner.
fn handle_simplex(simplex: &mut Simplex, dir: &mut Vec3) -> bool {
    loop {
        match simplex.size {
            2 => {
                // Çizgi parçası (Line segment)
                let ab = simplex.points[1].v - simplex.points[0].v;
                let ao = simplex.points[0].v * -1.0;

                if ab.dot(ao) > 0.0 {
                    // AB çizgisine dik ve O'ya (orijine) dönük olan vektörü bul: (AB x AO) x AB
                    let cross = ab.cross(ao);
                    *dir = cross.cross(ab);
                    // Bazen paralel olabilir, o zaman rastgele dikey al
                    if dir.length_squared() < 0.0001 {
                        // ab vektörüne dikey rastgele bir vektör
                        if ab.x.abs() > ab.y.abs() {
                            *dir = Vec3::new(ab.z, 0.0, -ab.x);
                        } else {
                            *dir = Vec3::new(0.0, -ab.z, ab.y);
                        }
                    }
                } else {
                    simplex.size = 1;
                    *dir = ao;
                }
                return false;
            }
            3 => {
                // Üçgen (Triangle)
                let ab = simplex.points[1].v - simplex.points[0].v;
                let ac = simplex.points[2].v - simplex.points[0].v;
                let ao = simplex.points[0].v * -1.0;

                let abc_normal = ab.cross(ac);

                // AB kenarının dışı (Dışa bakan yön: ab x abc_normal)
                if ab.cross(abc_normal).dot(ao) > 0.0 {
                    if ab.dot(ao) > 0.0 {
                        simplex.points[2] = simplex.points[1];
                        simplex.size = 2;
                        *dir = ab.cross(ao).cross(ab);
                    } else {
                        simplex.size = 1;
                        *dir = ao;
                    }
                }
                // AC kenarının dışı (Dışa bakan yön: abc_normal x ac)
                else if abc_normal.cross(ac).dot(ao) > 0.0 {
                    if ac.dot(ao) > 0.0 {
                        simplex.points[1] = simplex.points[2];
                        simplex.size = 2;
                        *dir = ac.cross(ao).cross(ac);
                    } else {
                        simplex.size = 1;
                        *dir = ao;
                    }
                }
                // Üçgenin içi, yukarıya (veya aşağıya) doğru
                else {
                    if abc_normal.dot(ao) > 0.0 {
                        *dir = abc_normal;
                    } else {
                        // Yönleri ters çevir ki winding order (sarma) doğru olsun
                        simplex.points.swap(1, 2);
                        *dir = abc_normal * -1.0;
                    }
                }
                return false;
            }
            4 => {
                // Tetrahedron (Dört yüzlü). Burada da karmaşık "Hangi yüzeyden dışarıdayız?" kontrolleri var.
                let ab = simplex.points[1].v - simplex.points[0].v;
                let ac = simplex.points[2].v - simplex.points[0].v;
                let ad = simplex.points[3].v - simplex.points[0].v;
                let ao = simplex.points[0].v * -1.0;

                let abc_normal = ab.cross(ac);
                let acd_normal = ac.cross(ad);
                let adb_normal = ad.cross(ab);

                if abc_normal.dot(ao) > 0.0 {
                    // ABC dışındayız (A, B, C zaten 0, 1, 2 indekslerinde), sadece boyutu 3'e düşür
                    simplex.size = 3;
                    continue; // handle_simplex rekürsiyonu yerine loop başına dönüldü
                }

                if acd_normal.dot(ao) > 0.0 {
                    simplex.points[1] = simplex.points[2];
                    simplex.points[2] = simplex.points[3];
                    simplex.size = 3;
                    continue;
                }

                if adb_normal.dot(ao) > 0.0 {
                    simplex.points[2] = simplex.points[1];
                    simplex.points[1] = simplex.points[3];
                    simplex.size = 3;
                    continue;
                }

                // Hiçbiri değilse orijin tetrahedron'un içerisindedir! Çarpışma GERÇEKLEŞTİ!
                return true;
            }
            _ => {
                return false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shape::{Aabb, ColliderShape, Sphere};

    #[test]
    fn test_gjk_sphere_sphere_intersect() {
        let shape_a = ColliderShape::Sphere(Sphere { radius: 1.0 });
        let pos_a = Vec3::new(0.0, 0.0, 0.0);
        let rot_a = Quat::IDENTITY;

        let shape_b = ColliderShape::Sphere(Sphere { radius: 1.0 });
        let pos_b = Vec3::new(1.5, 0.0, 0.0);
        let rot_b = Quat::IDENTITY;

        let (intersect, _) = gjk_intersect(&shape_a, pos_a, rot_a, &shape_b, pos_b, rot_b);
        assert!(intersect, "Spheres should intersect");
    }

    #[test]
    fn test_gjk_sphere_sphere_disjoint() {
        let shape_a = ColliderShape::Sphere(Sphere { radius: 1.0 });
        let pos_a = Vec3::new(0.0, 0.0, 0.0);
        let rot_a = Quat::IDENTITY;

        let shape_b = ColliderShape::Sphere(Sphere { radius: 1.0 });
        let pos_b = Vec3::new(3.0, 0.0, 0.0);
        let rot_b = Quat::IDENTITY;

        let (intersect, _) = gjk_intersect(&shape_a, pos_a, rot_a, &shape_b, pos_b, rot_b);
        assert!(!intersect, "Spheres should NOT intersect");
    }

    #[test]
    fn test_gjk_aabb_aabb_intersect() {
        let shape_a = ColliderShape::Aabb(Aabb {
            half_extents: Vec3::new(1.0, 1.0, 1.0),
        });
        let pos_a = Vec3::new(0.0, 0.0, 0.0);
        let rot_a = Quat::IDENTITY;

        let shape_b = ColliderShape::Aabb(Aabb {
            half_extents: Vec3::new(1.0, 1.0, 1.0),
        });
        let pos_b = Vec3::new(1.9, 1.9, 1.9); // Almost touching corner
        let rot_b = Quat::IDENTITY;

        let (intersect, _) = gjk_intersect(&shape_a, pos_a, rot_a, &shape_b, pos_b, rot_b);
        assert!(intersect, "AABBs should intersect");
    }
}
