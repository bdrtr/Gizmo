pub mod components;
pub mod shape;
pub mod fracture;

pub use components::{RigidBody, Transform, Velocity, Breakable};
pub use shape::{Collider, ColliderShape};

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct GpuPhysicsLink {
    pub id: u32,
}

gizmo_core::impl_component!(GpuPhysicsLink);
