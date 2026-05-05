use gizmo_math::{Quat, Vec3};
use serde::{Deserialize, Serialize};

use super::{PhysicsMaterial, CollisionLayer, Transform};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Collider {
    pub shape: ColliderShape,
    pub is_trigger: bool,
    pub material: PhysicsMaterial,
    pub collision_layer: CollisionLayer,
}

impl Default for Collider {
    fn default() -> Self {
        Self {
            shape: ColliderShape::Sphere(SphereShape { radius: 0.5 }),
            is_trigger: false,
            material: PhysicsMaterial::default(),
            collision_layer: CollisionLayer::default(),
        }
    }
}

impl Collider {
    /// Calculate AABB for this collider at given transform
    pub fn compute_aabb(&self, position: Vec3, rotation: Quat) -> gizmo_math::Aabb {
        match &self.shape {
            ColliderShape::Sphere(s) => {
                let radius_vec = Vec3::splat(s.radius);
                gizmo_math::Aabb::from_center_half_extents(position, radius_vec)
            }
            ColliderShape::Box(b) => {
                // Rotate the half extents to get world-space AABB
                let corners = [
                    Vec3::new(b.half_extents.x, b.half_extents.y, b.half_extents.z),
                    Vec3::new(-b.half_extents.x, b.half_extents.y, b.half_extents.z),
                    Vec3::new(b.half_extents.x, -b.half_extents.y, b.half_extents.z),
                    Vec3::new(b.half_extents.x, b.half_extents.y, -b.half_extents.z),
                    Vec3::new(-b.half_extents.x, -b.half_extents.y, b.half_extents.z),
                    Vec3::new(-b.half_extents.x, b.half_extents.y, -b.half_extents.z),
                    Vec3::new(b.half_extents.x, -b.half_extents.y, -b.half_extents.z),
                    Vec3::new(-b.half_extents.x, -b.half_extents.y, -b.half_extents.z),
                ];

                let mut min = Vec3::splat(f32::INFINITY);
                let mut max = Vec3::splat(f32::NEG_INFINITY);

                for corner in &corners {
                    let rotated = rotation * (*corner);
                    let world_pos = position + rotated;
                    min = min.min(world_pos);
                    max = max.max(world_pos);
                }

                gizmo_math::Aabb::new(min, max)
            }
            ColliderShape::Capsule(c) => {
                let axis = rotation * Vec3::Y;
                let half_height_vec = axis * c.half_height;
                let radius_vec = Vec3::splat(c.radius);
                let extent = half_height_vec.abs() + radius_vec;
                gizmo_math::Aabb::from_center_half_extents(position, extent)
            }
            ColliderShape::Plane(_) => {
                // Infinite plane - use a very large AABB
                let large = 10000.0;
                gizmo_math::Aabb::new(
                    position - Vec3::splat(large),
                    position + Vec3::splat(large),
                )
            }
            ColliderShape::TriMesh(tm) => {
                let mut min = Vec3::splat(f32::INFINITY);
                let mut max = Vec3::splat(f32::NEG_INFINITY);
                for v in tm.vertices.iter() {
                    let world_pos = position + rotation * (*v);
                    min = min.min(world_pos);
                    max = max.max(world_pos);
                }
                gizmo_math::Aabb::new(min, max)
            }
            ColliderShape::ConvexHull(ch) => {
                let mut min = Vec3::splat(f32::INFINITY);
                let mut max = Vec3::splat(f32::NEG_INFINITY);
                for v in ch.vertices.iter() {
                    let world_pos = position + rotation * (*v);
                    min = min.min(world_pos);
                    max = max.max(world_pos);
                }
                gizmo_math::Aabb::new(min, max)
            }
            ColliderShape::Compound(shapes) => {
                let mut min = Vec3::splat(f32::INFINITY);
                let mut max = Vec3::splat(f32::NEG_INFINITY);
                for (local_t, sub_shape) in shapes {
                    let world_pos = position + rotation.mul_vec3(local_t.position);
                    let world_rot = rotation * local_t.rotation;
                    
                    let temp_col = Collider {
                        shape: (**sub_shape).clone(),
                        ..Default::default()
                    };
                    let sub_aabb = temp_col.compute_aabb(world_pos, world_rot);
                    min = min.min(sub_aabb.min.into());
                    max = max.max(sub_aabb.max.into());
                }
                gizmo_math::Aabb::new(min, max)
            }
        }
    }

    pub fn plane(normal: Vec3, distance: f32) -> Self {
        Self {
            shape: ColliderShape::Plane(PlaneShape { normal, distance }),
            ..Default::default()
        }
    }

    pub fn sphere(radius: f32) -> Self {
        Self {
            shape: ColliderShape::Sphere(SphereShape { radius }),
            ..Default::default()
        }
    }

    pub fn box_collider(half_extents: Vec3) -> Self {
        Self {
            shape: ColliderShape::Box(BoxShape { half_extents }),
            ..Default::default()
        }
    }

    pub fn capsule(radius: f32, half_height: f32) -> Self {
        Self {
            shape: ColliderShape::Capsule(CapsuleShape {
                radius,
                half_height,
            }),
            ..Default::default()
        }
    }

    pub fn convex_hull(points: &[Vec3]) -> Self {
        let hull = crate::quickhull::compute_convex_hull(points);
        Self {
            shape: ColliderShape::ConvexHull(ConvexHullShape {
                vertices: std::sync::Arc::new(hull.vertices),
                faces: std::sync::Arc::new(hull.faces),
            }),
            ..Default::default()
        }
    }

    pub fn with_trigger(mut self, is_trigger: bool) -> Self {
        self.is_trigger = is_trigger;
        self
    }

    pub fn with_material(mut self, material: PhysicsMaterial) -> Self {
        self.material = material;
        self
    }

    // Backwards compatibility wrappers
    pub fn aabb(half_extents: Vec3) -> Self {
        Self::box_collider(half_extents)
    }

    pub fn new_sphere(radius: f32) -> Self {
        Self::sphere(radius)
    }

    pub fn new_aabb(x: f32, y: f32, z: f32) -> Self {
        Self::box_collider(Vec3::new(x, y, z))
    }

    pub fn new_capsule(radius: f32, half_height: f32) -> Self {
        Self::capsule(radius, half_height)
    }

    pub fn with_layer(mut self, layer: CollisionLayer) -> Self {
        self.collision_layer = layer;
        self
    }

    pub fn volume(&self) -> f32 {
        match &self.shape {
            ColliderShape::Sphere(s) => (4.0 / 3.0) * std::f32::consts::PI * s.radius.powi(3),
            ColliderShape::Box(b) => 8.0 * b.half_extents.x * b.half_extents.y * b.half_extents.z,
            ColliderShape::Capsule(c) => {
                let cylinder_vol = std::f32::consts::PI * c.radius.powi(2) * (c.half_height * 2.0);
                let sphere_vol = (4.0 / 3.0) * std::f32::consts::PI * c.radius.powi(3);
                cylinder_vol + sphere_vol
            }
            ColliderShape::Plane(_) => f32::MAX, // Safe value instead of INFINITY for inertia calculations
            ColliderShape::TriMesh(_) | ColliderShape::ConvexHull(_) | ColliderShape::Compound(_) => {
                let aabb = self.compute_aabb(Vec3::ZERO, Quat::IDENTITY);
                let e = aabb.max - aabb.min;
                e.x * e.y * e.z * 0.5 // Approximate volume from AABB
            }
        }
    }

    pub fn extents_y(&self) -> f32 {
        match &self.shape {
            ColliderShape::Sphere(s) => s.radius,
            ColliderShape::Box(b) => b.half_extents.y,
            ColliderShape::Capsule(c) => c.half_height + c.radius,
            ColliderShape::Plane(_) => 0.0,
            ColliderShape::TriMesh(_) | ColliderShape::ConvexHull(_) | ColliderShape::Compound(_) => {
                let aabb = self.compute_aabb(Vec3::ZERO, Quat::IDENTITY);
                (aabb.max.y - aabb.min.y) * 0.5
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ColliderShape {
    Sphere(SphereShape),
    Box(BoxShape),
    Capsule(CapsuleShape),
    Plane(PlaneShape),
    TriMesh(TriMeshShape),
    ConvexHull(ConvexHullShape),
    Compound(Vec<(Transform, Box<ColliderShape>)>),
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SphereShape {
    pub radius: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BoxShape {
    pub half_extents: Vec3,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CapsuleShape {
    pub radius: f32,
    pub half_height: f32, // Height of cylindrical part (not including hemispheres)
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PlaneShape {
    pub normal: Vec3,
    pub distance: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(into = "TriMeshShapeData", from = "TriMeshShapeData")]
pub struct TriMeshShape {
    pub vertices: std::sync::Arc<Vec<Vec3>>,
    pub indices: std::sync::Arc<Vec<u32>>,
    #[serde(skip)]
    pub bvh: std::sync::Arc<crate::bvh::BvhTree>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct TriMeshShapeData {
    vertices: Vec<Vec3>,
    indices: Vec<u32>,
}

impl From<TriMeshShapeData> for TriMeshShape {
    fn from(mut data: TriMeshShapeData) -> Self {
        let bvh = crate::bvh::BvhTree::build(&data.vertices, &mut data.indices).unwrap_or_default();
        Self {
            vertices: std::sync::Arc::new(data.vertices),
            indices: std::sync::Arc::new(data.indices),
            bvh: std::sync::Arc::new(bvh),
        }
    }
}

impl From<TriMeshShape> for TriMeshShapeData {
    fn from(shape: TriMeshShape) -> Self {
        Self {
            vertices: (*shape.vertices).clone(),
            indices: (*shape.indices).clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(into = "ConvexHullShapeData", from = "ConvexHullShapeData")]
pub struct ConvexHullShape {
    pub vertices: std::sync::Arc<Vec<Vec3>>,
    pub faces: std::sync::Arc<Vec<[u32; 3]>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ConvexHullShapeData {
    points: Vec<Vec3>, // These are raw points, we rebuild the hull on load
}

impl From<ConvexHullShapeData> for ConvexHullShape {
    fn from(data: ConvexHullShapeData) -> Self {
        let hull = crate::quickhull::compute_convex_hull(&data.points);
        Self {
            vertices: std::sync::Arc::new(hull.vertices),
            faces: std::sync::Arc::new(hull.faces),
        }
    }
}

impl From<ConvexHullShape> for ConvexHullShapeData {
    fn from(shape: ConvexHullShape) -> Self {
        Self {
            points: (*shape.vertices).clone(),
        }
    }
}

gizmo_core::impl_component!(Collider);
