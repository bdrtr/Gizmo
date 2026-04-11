pub mod character;
pub mod collision;
pub mod components;
pub mod constraints;
pub mod epa;
pub mod gjk;
pub mod integration;
pub mod shape;
pub mod system;
pub mod vehicle;
pub use character::{physics_character_system, CharacterController};
pub use collision::{
    check_aabb_aabb_manifold, check_capsule_aabb_manifold, check_capsule_capsule_manifold,
    check_capsule_sphere_manifold, check_sphere_aabb_manifold, check_sphere_sphere_manifold,
    test_aabb_aabb, test_sphere_sphere, CollisionManifold,
};
pub use components::{RigidBody, Transform, Velocity};
pub use constraints::{solve_constraints, Joint, JointKind, JointWorld};
pub use integration::{physics_apply_forces_system, physics_movement_system};
pub use shape::{Aabb, Capsule, Collider, ColliderShape, ConvexHull, Sphere};
pub use system::{physics_collision_system, PhysicsSolverState};
pub use vehicle::{physics_vehicle_system, VehicleController, Wheel};
pub mod race_ai;
pub use race_ai::{race_ai_system, RaceAI};

#[derive(Clone, Debug)]
pub struct CollisionEvent {
    pub entity_a: u32,
    pub entity_b: u32,
    pub position: gizmo_math::Vec3,
    pub normal: gizmo_math::Vec3,
    pub impulse: f32,
}
