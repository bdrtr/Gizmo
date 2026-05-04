pub mod broadphase;
pub mod collision;
pub mod components;
pub mod fracture;
pub mod integrator;
pub mod joints;
pub mod narrowphase;
pub mod island;
pub mod gjk;
pub mod raycast;
pub mod solver;
pub mod shape;
pub mod soft_body; // <--- ADDED FEM SOFT BODY
pub mod world;
pub mod system;
pub mod gpu_compute;
pub mod bvh;
pub mod vehicle;
pub mod character;
pub mod quickhull;
pub mod cloth;
pub mod rope;
pub mod ragdoll;
pub mod destruction;
pub mod error;

pub use error::GizmoError;

pub use broadphase::SpatialHash;
pub use gizmo_math::Aabb;
pub use soft_body::*; // <--- ADDED FEM SOFT BODY
pub use collision::{
    CollisionEvent, CollisionEventType, ContactManifold, ContactPoint, TriggerEvent,
};
pub use integrator::Integrator;
pub use joints::{
    Joint, JointData, JointSolver, JointType, HingeJointData, BallSocketJointData,
    SliderJointData, SpringJointData,
};
pub use narrowphase::NarrowPhase;
pub use gjk::Gjk;
pub use raycast::{Ray, Raycast, RaycastHit};
pub use solver::ConstraintSolver;
pub use world::PhysicsWorld;
pub use system::{physics_step_system, physics_fracture_system, physics_explosion_system};
pub use gpu_compute::*;
pub use components::{
    BodyType, BoxShape, Breakable, CapsuleShape, CharacterController, Collider, ColliderShape, CollisionLayer,
    ConvexHullShape, PhysicsMaterial, PlaneShape, RigidBody, SphereShape, Transform, TriMeshShape,
    Velocity, Explosion
};

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct GpuPhysicsLink {
    pub id: u32,
}

gizmo_core::impl_component!(GpuPhysicsLink);
