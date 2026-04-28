pub mod broadphase;
pub mod collision;
pub mod components;
pub mod fracture;
pub mod integrator;
pub mod narrowphase;
pub mod solver;
pub mod shape;
pub mod world;

pub use broadphase::{Aabb, SpatialHash};
pub use collision::{
    CollisionEvent, CollisionEventType, ContactManifold, ContactPoint, TriggerEvent,
};
pub use integrator::Integrator;
pub use narrowphase::{Gjk, NarrowPhase};
pub use solver::ConstraintSolver;
pub use world::PhysicsWorld;
pub use components::{
    BodyType, BoxShape, Breakable, CapsuleShape, Collider, ColliderShape, CollisionLayer,
    PhysicsMaterial, PlaneShape, RigidBody, SphereShape, Transform, Velocity,
};

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct GpuPhysicsLink {
    pub id: u32,
}

gizmo_core::impl_component!(GpuPhysicsLink);
