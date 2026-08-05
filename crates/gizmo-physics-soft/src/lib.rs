#![warn(missing_docs)]
//! (`missing_docs` is a RATCHET, not a suggestion. The CI lint gate runs with `-D warnings`,
//! so every public item in this crate must carry a doc comment or the build fails. This crate
//! is Stage A — the dependency-light core that goes to 1.x first — and its documented surface
//! is part of that promise. Do not silence this with `#[allow]`; write the doc.)

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
/// WGPU compute-shader path for stepping [`soft_body::SoftBodyMesh`] on the GPU.
///
/// Only compiled when the `gpu_physics` feature is enabled; that feature also pulls in
/// `wgpu`, `bytemuck` and `pollster`. The CPU path in [`soft_body`] remains available and
/// is what the [`system`] drivers use.
#[cfg(feature = "gpu_physics")]
pub mod gpu_compute;
/// Ropes and chains. A [`rope::Rope`] is a flat chain of particles in which every
/// consecutive pair forms one link; all links share a single rest length, and the strand's
/// only collision geometry is an implicit plane at `y = 0`. Particles store no velocity —
/// it is reconstructed each step from the previous position, so writing a node's position
/// also changes its velocity.
pub mod rope;
/// Volumetric soft bodies. A [`soft_body::SoftBodyMesh`] is a tetrahedral mesh with a
/// Neo-Hookean material; each element captures its rest shape when it is added, so the nodes
/// must already be in their rest configuration at that point. This module also holds
/// [`soft_body::resolve_node_collision`], the per-node swept collider test that both the CPU
/// step and the `gpu_compute` path use to keep nodes out of rigid bodies.
pub mod soft_body;
/// ECS driver systems that step every soft-body component found in a `World`.
///
/// Each system clamps its `dt` to at most 1/30 s before stepping. The soft-body and cloth
/// drivers also collect every entity carrying both a `Transform` and a `Collider` into the
/// collider slice they pass down; the rope driver passes none, since [`rope::Rope`] has no
/// rigid-collider path at all.
/// Requires the `ecs` feature — these are the ECS drivers.
#[cfg(feature = "ecs")]
pub mod system;

pub use error::SoftBodyError;

// Re-export common traits and structs
pub use cloth::*;
pub use rope::*;
pub use soft_body::*;
#[cfg(feature = "ecs")]
pub use system::*;
