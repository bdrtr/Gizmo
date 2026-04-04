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
