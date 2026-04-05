use yelbegen_math::Vec3;

// En performanslı 3D çarpışma nesneleri: Küre ve Axis-Aligned Kutu

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Sphere {
    pub radius: f32,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Aabb {
    pub half_extents: Vec3,
}

// ECS Component'i olmak üzere hazırlanmış Çarpışma Şekilleri listesi
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub enum ColliderShape {
    Sphere(Sphere),
    Aabb(Aabb),
}

impl ColliderShape {
    /// GJK Algoritması için Destek Fonsiyonu (Support Function).
    /// Verilen bir 'direction' (yön) vektörü üzerinde, şeklin en uç noktasını döndürür.
    /// Transform (pozisyon ve rotasyon) dikkate alınarak Dünya Koordinatında (`World Space`) hesaplar.
    pub fn support_point(&self, pos: Vec3, _rot: yelbegen_math::Quat, mut dir: Vec3) -> Vec3 {
        if dir.length_squared() < 0.0001 {
            dir = Vec3::new(1.0, 0.0, 0.0);
        } else {
            dir = dir.normalize();
        }

        match self {
            ColliderShape::Sphere(s) => {
                // Kürenin rotasyonu önemli değildir, merkezden o yöne doğru yarıçap kadar gitmek kafidir.
                pos + dir * s.radius
            }
            ColliderShape::Aabb(aabb) => {
                // AABB şu an Axis-Aligned (Rotasyonsuz), eğer rotasyon eklenirse OBB (Oriented Bounding Box) olur.
                // Şimdilik sadece pozisyonu dikkate alıyoruz (Rotasyonu AABB üzerinde uygulamayız).
                let mut p = pos;
                p.x += if dir.x > 0.0 { aabb.half_extents.x } else { -aabb.half_extents.x };
                p.y += if dir.y > 0.0 { aabb.half_extents.y } else { -aabb.half_extents.y };
                p.z += if dir.z > 0.0 { aabb.half_extents.z } else { -aabb.half_extents.z };
                p
            }
        }
    }
}

// Fiziksel varlıkları tespit edecek `Collider` bileşeni
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Collider {
    pub shape: ColliderShape,
}

impl Collider {
    pub fn new_sphere(radius: f32) -> Self {
        Self {
            shape: ColliderShape::Sphere(Sphere { radius }),
        }
    }

    pub fn new_aabb(hx: f32, hy: f32, hz: f32) -> Self {
        Self {
            shape: ColliderShape::Aabb(Aabb {
                half_extents: Vec3::new(hx, hy, hz),
            }),
        }
    }
}
