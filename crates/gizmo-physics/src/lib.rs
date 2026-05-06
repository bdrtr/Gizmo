pub mod broadphase;
pub mod bvh; // Move near broadphase
pub mod collision;
pub mod components;
pub mod error;
pub mod gpu_compute;
pub mod gpu_fluid;
pub mod integrator;
pub mod island;
pub mod joints;
pub mod narrowphase;
pub mod quickhull;
pub mod fracture;
pub mod destruction;
pub mod gjk;
pub mod raycast;
pub mod shape;
pub mod solver;
pub mod system;
pub mod world;
pub(crate) mod pipeline;

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
pub use gpu_compute::{GpuCompute, GpuSoftBodyNode, GpuParameters, GpuPhysicsLink};
pub use island::{Island, IslandManager, PhysicsMetrics};
pub use fracture::{voronoi_shatter, generate_fracture_chunks, PreFracturedCache};
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
pub use components::{
    BodyType, BoxShape, Breakable, CapsuleShape, CharacterController, Collider, ColliderShape, CollisionLayer,
    ConvexHullShape, PhysicsMaterial, PlaneShape, RigidBody, SphereShape, Transform, TriMeshShape,
    Velocity, Explosion, FluidSimulation
};

