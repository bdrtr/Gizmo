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
                let mut p = pos;
                let eps = 1e-4;
                p.x += if dir.x > eps { aabb.half_extents.x } else if dir.x < -eps { -aabb.half_extents.x } else { 0.0 };
                p.y += if dir.y > eps { aabb.half_extents.y } else if dir.y < -eps { -aabb.half_extents.y } else { 0.0 };
                p.z += if dir.z > eps { aabb.half_extents.z } else if dir.z < -eps { -aabb.half_extents.z } else { 0.0 };
                p
            }
            ColliderShape::Capsule(cap) => {
                // Kapsül = iki yarıküre merkezi arasındaki segment + yarıçap
                // Lokal koordinatta: üst = (0, half_height, 0), alt = (0, -half_height, 0)
                // Rotasyonlu dünya koordinatına çevir
                let local_top = Vec3::new(0.0, cap.half_height, 0.0);
                let local_bot = Vec3::new(0.0, -cap.half_height, 0.0);
                let world_top = pos + rot.mul_vec3(local_top);
                let world_bot = pos + rot.mul_vec3(local_bot);
                
                // Hangi uç, dir yönünde daha ilerideyse onu seç
                if dir.dot(world_top) >= dir.dot(world_bot) {
                    world_top + dir * cap.radius
                } else {
                    world_bot + dir * cap.radius
                }
            }
            ColliderShape::ConvexHull(hull) => {
                // Tüm vertexler üzerinde max dot product — O(n) brute force
                // Küçük vertex sayıları (<100) için yeterince hızlı
                let mut best_dot = f32::NEG_INFINITY;
                let mut best_point = pos;
                
                for v in &hull.vertices {
                    let world_v = pos + rot.mul_vec3(*v);
                    let d = dir.dot(world_v);
                    if d > best_dot {
                        best_dot = d;
                        best_point = world_v;
                    }
                }
                best_point
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

