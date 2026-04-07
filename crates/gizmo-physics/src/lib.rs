pub mod shape;
pub mod collision;
pub mod components;
pub mod system;
pub mod constraints;
pub mod gjk;
pub mod epa;
pub mod integration;
pub mod vehicle;
pub mod character;
pub use shape::{Aabb, Sphere, Capsule, ConvexHull, ColliderShape, Collider};
pub use collision::{
    test_aabb_aabb, test_sphere_sphere, CollisionManifold,
    check_aabb_aabb_manifold, check_sphere_sphere_manifold, check_sphere_aabb_manifold,
    check_capsule_capsule_manifold, check_capsule_sphere_manifold, check_capsule_aabb_manifold,
};
pub use components::{Transform, Velocity, RigidBody};
pub use system::{PhysicsSolverState, physics_collision_system};
pub use integration::physics_movement_system;
pub use constraints::{Joint, JointKind, JointWorld, solve_constraints};
pub use vehicle::{Wheel, VehicleController, physics_vehicle_system};
pub use character::{CharacterController, physics_character_system};

#[derive(Clone, Debug)]
pub struct CollisionEvent {
    pub entity_a: u32,
    pub entity_b: u32,
    pub position: gizmo_math::Vec3,
    pub normal: gizmo_math::Vec3,
    pub impulse: f32,
}
