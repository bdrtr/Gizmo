pub mod broadphase;
pub mod bvh; // Move near broadphase
pub mod collision;
pub mod components;
pub mod error;
#[cfg(feature = "gpu_physics")]
pub mod gpu_compute;

pub mod destruction;
pub mod fracture;
pub mod gjk;
pub mod integrator;
pub mod island;
pub mod joints;
pub mod narrowphase;
pub(crate) mod pipeline;
pub mod quickhull;
pub mod raycast;
pub mod shape;
pub mod solver;
pub mod system;
pub mod world;

// Additional features (implemented but optional)
pub mod character;
pub mod cloth;
pub mod ragdoll;
pub mod rope;
pub mod soft_body;
pub mod vehicle;

pub use error::GizmoError;

pub use broadphase::SpatialHash;
pub use gizmo_math::Aabb;
pub use soft_body::{SoftBodyMesh, SoftBodyNode, Tetrahedron};

#[cfg(feature = "gpu_physics")]
pub use gpu_compute::{GpuCompute, GpuParameters, GpuPhysicsLink, GpuSoftBodyNode};

pub use collision::{
    CollisionEvent, CollisionEventType, ContactManifold, ContactPoint, TriggerEvent,
};
pub use components::{
    BodyType, BoxShape, Breakable, CapsuleShape, CharacterController, Collider, ColliderShape,
    CollisionLayer, ConvexHullShape, Explosion, FluidSimulation, GlobalTransform, PhysicsMaterial,
    PlaneShape, RigidBody, SphereShape, Transform, TriMeshShape, Velocity,
};
pub use fracture::{generate_fracture_chunks, voronoi_shatter, PreFracturedCache};
pub use gjk::Gjk;
pub use integrator::Integrator;
pub use island::{Island, IslandManager, PhysicsMetrics};
pub use joints::{
    BallSocketJointData, HingeJointData, Joint, JointData, JointSolver, JointType, SliderJointData,
    SpringJointData,
};
pub use narrowphase::NarrowPhase;
pub use raycast::{Ray, Raycast, RaycastHit};
pub use solver::ConstraintSolver;
pub use system::{physics_explosion_system, physics_fracture_system, physics_step_system};
pub use world::PhysicsWorld;
