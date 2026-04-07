use gizmo_math::Vec3;

// En performanslı 3D çarpışma nesneleri: Küre, Kutu, Kapsül ve Konveks Gövde

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Sphere {
    pub radius: f32,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Aabb {
    pub half_extents: Vec3,
}

/// Kapsül şekli — İki yarıküre + silindir. Karakter kontrolcüsü için ideal.
/// Dikey eksen Y'dir: üst merkez (0, half_height, 0), alt merkez (0, -half_height, 0)
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Capsule {
    pub radius: f32,
    pub half_height: f32, // Yarıküre merkezleri arası mesafenin yarısı
}

/// Konveks Gövde — Rastgele vertex kümesinin konveks zarfı. Mesh collider için kullanılır.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConvexHull {
    pub vertices: Vec<Vec3>,
}

// ECS Component'i olmak üzere hazırlanmış Çarpışma Şekilleri listesi
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum ColliderShape {
    Sphere(Sphere),
    Aabb(Aabb),
    Capsule(Capsule),
    ConvexHull(ConvexHull),
}

impl ColliderShape {
    /// GJK Algoritması için Destek Fonksiyonu (Support Function).
    /// Verilen bir 'direction' (yön) vektörü üzerinde, şeklin en uç noktasını döndürür.
    /// Transform (pozisyon ve rotasyon) dikkate alınarak Dünya Koordinatında (`World Space`) hesaplar.
    pub fn support_point(&self, pos: Vec3, rot: gizmo_math::Quat, mut dir: Vec3) -> Vec3 {
        if dir.length_squared() < 0.0001 {
            dir = Vec3::new(1.0, 0.0, 0.0);
        } else {
            dir = dir.normalize();
        }

        match self {
            ColliderShape::Sphere(s) => {
                pos + dir * s.radius
            }
            ColliderShape::Aabb(aabb) => {
                // AABB'yi gerçek rotasyona tepki veren bir OBB gibi ele alıyoruz:
                let local_dir = rot.inverse().mul_vec3(dir);
                let lx = if local_dir.x >= 0.0 { aabb.half_extents.x } else { -aabb.half_extents.x };
                let ly = if local_dir.y >= 0.0 { aabb.half_extents.y } else { -aabb.half_extents.y };
                let lz = if local_dir.z >= 0.0 { aabb.half_extents.z } else { -aabb.half_extents.z };
                pos + rot.mul_vec3(Vec3::new(lx, ly, lz))
            }
            ColliderShape::Capsule(cap) => {
                // Arama vektörünü lokal uzaya çek (Böylece vertexlere döngüde rotasyon uygulamaktan kurtuluyoruz)
                let local_dir = rot.inverse().mul_vec3(dir);
                let local_top = Vec3::new(0.0, cap.half_height, 0.0);
                let local_bot = Vec3::new(0.0, -cap.half_height, 0.0);
                
                // Lokal yönü baz alıp seçimi yap
                let best_local = if local_dir.dot(local_top) >= local_dir.dot(local_bot) {
                    local_top
                } else {
                    local_bot
                };
                
                // Sadece seçili (1 adet) noktayı tekrar dünyaya çevirip küre yarıçapını ekle
                pos + rot.mul_vec3(best_local) + dir * cap.radius
            }
            ColliderShape::ConvexHull(hull) => {
                // Arama yönünü konveks gövdenin kendi eksenine göre (Lokal uzaya) çeviriyoruz.
                // Bu optimizasyon sayeseinde döngü içindeki N adet rotasyon işlemi, 1 inverse rotasyon işlemine düşer.
                let local_dir = rot.inverse().mul_vec3(dir);
                let mut best_dot = f32::NEG_INFINITY;
                let mut best_local = Vec3::ZERO;
                
                for v in &hull.vertices {
                    let d = local_dir.dot(*v);
                    if d > best_dot {
                        best_dot = d;
                        best_local = *v;
                    }
                }
                // Sadece son kazanan vertex noktasına Dünya Uzayı rotasyonunu ve pozisyonunu ver.
                pos + rot.mul_vec3(best_local)
            }
        }
    }
}

// Fiziksel varlıkları tespit edecek `Collider` bileşeni
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Collider {
    pub shape: ColliderShape,
}

impl Collider {
    pub fn new_sphere(radius: f32) -> Self {
        Self { shape: ColliderShape::Sphere(Sphere { radius }) }
    }

    pub fn new_aabb(hx: f32, hy: f32, hz: f32) -> Self {
        Self { shape: ColliderShape::Aabb(Aabb { half_extents: Vec3::new(hx, hy, hz) }) }
    }

    /// Kapsül collider oluşturur. Toplam yükseklik = 2*(half_height + radius)
    pub fn new_capsule(radius: f32, half_height: f32) -> Self {
        Self { shape: ColliderShape::Capsule(Capsule { radius, half_height }) }
    }

    /// Konveks gövde collider oluşturur. Vertex'ler lokal koordinatlarda verilir.
    pub fn new_convex(vertices: Vec<Vec3>) -> Self {
        Self { shape: ColliderShape::ConvexHull(ConvexHull { vertices }) }
    }
}

