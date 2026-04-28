pub mod broadphase;
pub mod collision;
pub mod components;
pub mod fracture;
pub mod integrator;
pub mod joints;
pub mod narrowphase;
pub mod raycast;
pub mod solver;
pub mod shape;
pub mod world;

pub use broadphase::SpatialHash;
pub use gizmo_math::Aabb;
pub use collision::{
    CollisionEvent, CollisionEventType, ContactManifold, ContactPoint, TriggerEvent,
};
pub use integrator::Integrator;
pub use joints::{
    Joint, JointData, JointSolver, JointType, HingeJointData, BallSocketJointData,
    SliderJointData, SpringJointData,
};
pub use narrowphase::{Gjk, NarrowPhase};
pub use raycast::{Ray, Raycast, RaycastHit};
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
