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
    #[serde(skip)]
    Swept {
        base: Box<ColliderShape>,
        sweep_vector: Vec3,
    },
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
                pos + rot.mul_vec3(best_local)
            }
            ColliderShape::Swept { base, sweep_vector } => {
                // Sweep mantığı: Orijinal şeklin uç noktasına, eğer tarama yönü 'dir' ile aynı yöndeyse
                // sweep_vector (süpürme hareketi) eklenir. Bu sayede şekil hareket ettiği hacmi kapsar.
                let offset = if dir.dot(*sweep_vector) > 0.0 { *sweep_vector } else { Vec3::ZERO };
                base.support_point(pos, rot, dir) + offset
            }
        }
    }

    /// Tüm boyut şekillerini çevreleyen temel bir Bounding Box (AABB) üretir.
    /// Kesişim (Raycast/Broadphase) algoritmalarında ön ve hızlı test için gereklidir.
    pub fn bounding_box_half_extents(&self) -> Vec3 {
        match self {
            ColliderShape::Sphere(s) => Vec3::new(s.radius, s.radius, s.radius),
            ColliderShape::Aabb(a) => a.half_extents,
            ColliderShape::Capsule(c) => Vec3::new(c.radius, c.half_height + c.radius, c.radius),
            ColliderShape::ConvexHull(c) => {
                let mut max_x = 0.0_f32;
                let mut max_y = 0.0_f32;
                let mut max_z = 0.0_f32;
                for v in &c.vertices {
                    max_x = max_x.max(v.x.abs());
                    max_y = max_y.max(v.y.abs());
                    max_z = max_z.max(v.z.abs());
                }
                Vec3::new(max_x, max_y, max_z)
            }
            ColliderShape::Swept { base, sweep_vector } => {
                let base_ext = base.bounding_box_half_extents();
                Vec3::new(
                    base_ext.x + sweep_vector.x.abs() * 0.5,
                    base_ext.y + sweep_vector.y.abs() * 0.5,
                    base_ext.z + sweep_vector.z.abs() * 0.5,
                )
            }
        }
    }
}

/// Fiziksel çarpışma bileşeni.
///
/// **ÖNEMLİ TASARIM KARARI — `Transform.scale` ETKİSİ:**
/// Collider boyutları (half_extents, radius, half_height) `Transform.scale` ile
/// otomatik olarak **çarpılmaz**. Şekil boyutlarını doğrudan oluştururken belirleyin.
///
/// Bu bilinçli bir karardır:
/// - Fizik deterministik kalır (scale animasyonu collision'ı bozmaz)
/// - Runtime'da her frame scale × collider çarpımı yapılmaz
/// - Non-uniform scale (2, 1, 3) küreyi elipse çevirirdi ki bu GJK/EPA'yı bozar
///
/// Doğru kullanım örneği:
/// ```
/// // Görsel: Transform scale (2, 2, 2) ile 2x büyütülmüş küp
/// // Collider: half_extents de 2x olmalı → new_aabb(2.0, 2.0, 2.0)
/// ```
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

