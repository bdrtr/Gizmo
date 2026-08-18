//! Gizmo Physics Module Re-exports
//! This module provides backward-compatible exports for the split physics crates.

pub use gizmo_physics_core::*;

/// Physics components from both halves of the split: the core's `Transform`, `Collider` and
/// materials, plus the rigid crate's `RigidBody` and `Velocity`.
pub mod components {
    pub use gizmo_physics_core::components::*;
    pub use gizmo_physics_rigid::components::*;
}

// Some legacy usages accessed RigidBody directly from gizmo::physics
pub use components::{RigidBody, Velocity, GpuPhysicsLink, GlobalTransform};
pub use gizmo_physics_core::Transform;

/// The rigid-body world — `PhysicsWorld`, its step, and the scene queries over it.
pub mod world {
    pub use gizmo_physics_rigid::world::*;
}

/// Joints and the joint solver: hinges, sliders, ball sockets, D6 and the rest.
pub mod joints {
    pub use gizmo_physics_rigid::joints::*;
}

#[cfg(feature = "physics-soft")]
/// Soft bodies: the FEM solver and the deformable primitives built on it.
pub mod soft_body {
    pub use gizmo_physics_soft::*;
}

/// The ECS bridge — `physics_step_system` and the systems around it.
pub mod system {
    pub use gizmo_physics_rigid::system::*;
}

#[cfg(feature = "physics-dynamics")]
/// Vehicle dynamics: the wheel model, the drivetrain and the tyre curves.
pub mod vehicle {
    pub use gizmo_physics_dynamics::vehicle::*;
}

#[cfg(feature = "physics-dynamics")]
/// The kinematic character controller, including its swimming path.
pub mod character {
    pub use gizmo_physics_dynamics::character::*;
}

#[cfg(feature = "physics-dynamics")]
/// Ragdolls — a joint skeleton built from a set of bodies.
pub mod ragdoll {
    pub use gizmo_physics_dynamics::ragdoll::*;
}

/// Just [`ColliderShape`](gizmo_physics_core::ColliderShape), for code that only needs the
/// shape vocabulary.
pub mod shape {
    pub use gizmo_physics_core::ColliderShape;
}

#[cfg(feature = "physics-soft")]
/// Ropes and cables, from the soft-body crate.
pub mod rope {
    pub use gizmo_physics_soft::rope::*;
}

/// Fracture: Voronoi shattering and the debris it produces.
pub mod fracture {
    pub use gizmo_physics_rigid::fracture::*;
}


#[cfg(feature = "physics-soft")]
/// Cloth simulation, from the soft-body crate.
pub mod cloth {
    pub use gizmo_physics_soft::cloth::*;
}

pub use system::{physics_fracture_system, physics_explosion_system, physics_step_system};

// Gameplay controller systems (Pacejka vehicle + kinematic character). These drive
// `VehicleController` / `CharacterController` and must run *before* `physics_step_system`
// so the rigid step integrates the forces they write into `Velocity`. Demos that step
// physics manually (car_demo, vehicle_scene, hill_climb) call these directly.
//
// `fighter_frame_system` is in the same list for a different reason: it integrates nothing, it
// counts frames on `FighterController`, and it needs the same one-call-per-fixed-step placement.
// `PlayLoop` calls it, so the editor's ▶ and every exported game have the fight clock; a game
// stepping physics by hand calls it the way the demos above call theirs.
#[cfg(feature = "physics-dynamics")]
pub use gizmo_physics_dynamics::{
    character_controller_system, fighter_frame_system, oxygen_system, vehicle_controller_system,
    Oxygen,
};
