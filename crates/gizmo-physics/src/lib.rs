pub mod broadphase;
pub mod collision;
pub mod components;
pub mod fracture;
pub mod narrowphase;
pub mod shape;

pub use broadphase::{Aabb, SpatialHash};
pub use collision::{
    CollisionEvent, CollisionEventType, ContactManifold, ContactPoint, TriggerEvent,
};
pub use narrowphase::{Gjk, NarrowPhase};
pub use components::{
    BodyType, BoxShape, Breakable, CapsuleShape, Collider, ColliderShape, CollisionLayer,
    PhysicsMaterial, PlaneShape, RigidBody, SphereShape, Transform, Velocity,
};
pub use shape::{Collider as ShapeCollider, ColliderShape as ShapeColliderShape};

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct GpuPhysicsLink {
    pub id: u32,
}

gizmo_core::impl_component!(GpuPhysicsLink);
