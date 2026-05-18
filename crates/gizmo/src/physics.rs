//! Gizmo Physics Module Re-exports
//! This module provides backward-compatible exports for the split physics crates.

pub use gizmo_physics_core::*;

pub mod components {
    pub use gizmo_physics_core::components::*;
    pub use gizmo_physics_rigid::components::*;
}

// Some legacy usages accessed RigidBody directly from gizmo::physics
pub use components::{RigidBody, Velocity, GpuPhysicsLink};
pub use gizmo_physics_core::Transform;

pub mod world {
    pub use gizmo_physics_rigid::world::*;
}

pub mod joints {
    pub use gizmo_physics_rigid::joints::*;
}

#[cfg(feature = "physics-soft")]
pub mod soft_body {
    pub use gizmo_physics_soft::*;
}

pub mod system {
    pub use gizmo_physics_rigid::system::*;
}

#[cfg(feature = "physics-dynamics")]
pub mod vehicle {
    pub use gizmo_physics_dynamics::vehicle::*;
}

#[cfg(feature = "physics-dynamics")]
pub mod character {
    pub use gizmo_physics_dynamics::character::*;
}

#[cfg(feature = "physics-dynamics")]
pub mod ragdoll {
    pub use gizmo_physics_dynamics::ragdoll::*;
}

pub mod shape {
    pub use gizmo_physics_core::ColliderShape;
}

#[cfg(feature = "physics-soft")]
pub mod rope {
    pub use gizmo_physics_soft::rope::*;
}

pub mod fracture {
    pub use gizmo_physics_rigid::fracture::*;
}


#[cfg(feature = "physics-soft")]
pub mod cloth {
    pub use gizmo_physics_soft::cloth::*;
}

pub use system::{physics_fracture_system, physics_explosion_system};
