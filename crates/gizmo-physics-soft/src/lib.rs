//! Soft-body physics for the Gizmo engine.
//!
//! This crate provides position-based soft-body simulations:
//!
//! - [`cloth::Cloth`] — a cloth sheet solved with Extended Position Based Dynamics (XPBD).
//! - [`rope::Rope`] — a rope/chain solved with Position Based Dynamics (PBD).
//! - [`soft_body::SoftBodyMesh`] — a tetrahedral FEM soft body using a Neo-Hookean
//!   hyperelastic material model.
//!
//! Each simulation type is an ECS [`gizmo_core::Component`] and has a matching driver in
//! the [`system`] module ([`system::cloth_step_system`], [`system::rope_step_system`],
//! [`system::soft_body_step_system`]) that steps every instance found in the world.
//!
//! Enabling the optional `gpu_physics` feature adds a WGPU compute path for the FEM
//! soft body via the `gpu_compute` module.

// Sequential fallback for rayon on wasm (no OS threads); native uses rayon.
#[cfg(target_arch = "wasm32")]
mod parallel_compat;
pub mod cloth;
pub mod error;
#[cfg(feature = "gpu_physics")]
pub mod gpu_compute;
pub mod rope;
pub mod soft_body;
pub mod system;

pub use error::SoftBodyError;

// Re-export common traits and structs
pub use cloth::*;
pub use rope::*;
pub use soft_body::*;
pub use system::*;
