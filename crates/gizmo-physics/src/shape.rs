use gizmo_core::impl_component;
use gizmo_math::Vec3;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sphere {
    pub radius: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Aabb {
    pub half_extents: Vec3,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capsule {
    pub radius: f32,
    pub half_height: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ColliderShape {
    Sphere(Sphere),
    Aabb(Aabb),
    Plane { normal: Vec3, constant: f32 },
    Capsule(Capsule),
}

impl ColliderShape {
    pub fn bounding_box_half_extents(&self, _rotation: gizmo_math::Quat) -> Vec3 {
        match self {
            ColliderShape::Aabb(aabb) => aabb.half_extents,
            ColliderShape::Sphere(s) => Vec3::splat(s.radius),
            ColliderShape::Plane { .. } => Vec3::splat(100.0), // Sonsuz ama editor için görece büyük bir değer
            ColliderShape::Capsule(c) => Vec3::new(c.radius, c.half_height + c.radius, c.radius),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Collider {
    pub shape: ColliderShape,
}

impl Default for Collider {
    fn default() -> Self {
        Self::sphere(0.5)
    }
}

impl Collider {
    pub fn sphere(radius: f32) -> Self {
        Self {
            shape: ColliderShape::Sphere(Sphere { radius }),
        }
    }

    pub fn aabb(half_extents: Vec3) -> Self {
        Self {
            shape: ColliderShape::Aabb(Aabb { half_extents }),
        }
    }
    
    pub fn plane(normal: Vec3, constant: f32) -> Self {
        Self {
            shape: ColliderShape::Plane { normal, constant },
        }
    }
    
    pub fn capsule(radius: f32, half_height: f32) -> Self {
        Self {
            shape: ColliderShape::Capsule(Capsule { radius, half_height }),
        }
    }
    
    // Backwards compatibility wrappers
    pub fn new_sphere(radius: f32) -> Self { Self::sphere(radius) }
    pub fn new_aabb(x: f32, y: f32, z: f32) -> Self { Self::aabb(Vec3::new(x, y, z)) }
    pub fn new_capsule(radius: f32, half_height: f32) -> Self { Self::capsule(radius, half_height) }
}

impl_component!(Collider);
