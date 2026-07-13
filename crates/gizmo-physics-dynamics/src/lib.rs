//! High-level physics dynamics for the Gizmo engine.
//!
//! This crate builds gameplay-oriented dynamics on top of the lower-level
//! `gizmo-physics-core` and `gizmo-physics-rigid` crates. It provides three
//! self-contained modules:
//!
//! - [`character`]: a kinematic character controller ([`update_character`]) with
//!   ground snapping, slope handling, step climbing, coyote time and jump buffering.
//! - [`vehicle`]: an arcade-to-sim vehicle model ([`VehicleController`],
//!   [`update_vehicle`]) featuring a combined-slip Pacejka tire model, suspension,
//!   anti-roll bars, aerodynamics and an automatic transmission.
//! - [`ragdoll`]: a [`RagdollBuilder`] for constructing humanoid ragdoll skeletons
//!   ([`RagdollBoneDef`]) wired together with joints, plus a runtime
//!   [`spawn_ragdoll`] that instantiates one rigid body per bone.
//! - [`systems`]: thin ECS system wrappers ([`vehicle_controller_system`],
//!   [`character_controller_system`]) that drive the vehicle/character
//!   controllers from a [`gizmo_core::system::Schedule`].
//!
//! Each update function operates on borrowed ECS components plus a slice of all
//! scene colliders, so callers are responsible for gathering collider data each
//! frame and feeding it in.

pub mod character;
pub mod oxygen;
pub mod ragdoll;
pub mod systems;
pub mod vehicle;

// Re-export common traits and structs
pub use character::*;
pub use oxygen::*;
pub use ragdoll::*;
pub use systems::*;
pub use vehicle::*;
