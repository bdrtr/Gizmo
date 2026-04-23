pub mod components;
pub mod fracture;
pub mod shape;

pub use components::{Breakable, RigidBody, Transform, Velocity};
pub use shape::{Collider, ColliderShape};

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct GpuPhysicsLink {
    pub id: u32,
}

gizmo_core::impl_component!(GpuPhysicsLink);
