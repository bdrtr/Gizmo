#![warn(missing_docs)]
//! (`missing_docs` is a RATCHET, not a suggestion. The CI lint gate runs with `-D warnings`,
//! so every public item in this crate must carry a doc comment or the build fails. This crate
//! is Stage A — the dependency-light core that goes to 1.x first — and its documented surface
//! is part of that promise. Do not silence this with `#[allow]`; write the doc.)

//! High-level physics dynamics for the Gizmo engine.
//!
//! This crate builds gameplay-oriented dynamics on top of the lower-level
//! `gizmo-physics-core` and `gizmo-physics-rigid` crates. It provides these
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
//! - [`oxygen`]: a breath/air budget ([`Oxygen`], [`oxygen_system`]) that drains
//!   while an entity's head point is inside a fluid zone and refills whenever it is
//!   not — including when the scene carries no `PhysicsWorld` resource at all.
//! - [`systems`]: thin ECS system wrappers ([`vehicle_controller_system`],
//!   [`character_controller_system`]) that drive the vehicle/character
//!   controllers from a [`gizmo_core::system::Schedule`].
//!
//! Each update function operates on borrowed ECS components plus a slice of all
//! scene colliders, so callers are responsible for gathering collider data each
//! frame and feeding it in.
//!
//! The vehicle model is the exception, and the direction the rest is headed:
//! [`update_vehicle_with_query`] takes a [`WheelGroundQuery`] instead of a slice, so its
//! suspension rays can be answered by a broadphase — a `PhysicsWorld` implements that trait —
//! rather than by a full scan of a list the caller had to clone. [`vehicle_controller_system`]
//! routes through it; see its docs for the (real, stated) change in what a wheel may rest on.
//! [`character_controller_system`] still takes the slice.

/// Kinematic character controller.
///
/// Exposes a single entry point, [`update_character`], which advances one character by one
/// fixed step against a slice of scene colliders, mutating the character's `Transform` and
/// `Velocity` in place.
///
/// It has two modes, selected by its `water_surface_y: Option<f32>` argument. `None` runs the
/// land model: ground detection, gravity, slope movement, wall sliding and step climbing.
/// `Some(surface_y)` takes a buoyant swim step instead and returns immediately — none of the
/// land gravity, ground-snap or step-climb logic runs in that branch.
pub mod character;
/// Requires the `ecs` feature.
#[cfg(feature = "ecs")]
pub mod oxygen;
/// Requires the `ecs` feature.
#[cfg(feature = "ecs")]
pub mod ragdoll;
/// Requires the `ecs` feature.
#[cfg(feature = "ecs")]
pub mod systems;
/// Vehicle dynamics: a raycast-suspension car model.
///
/// [`VehicleController`] holds the per-vehicle state and its [`Wheel`]s; [`VehicleTuning`],
/// [`PacejkaParams`] and [`AeroPackage`] describe the setup, and
/// [`update_vehicle`] advances one chassis by a step, applying suspension, tire and
/// aerodynamic forces to the chassis `Velocity`.
pub mod vehicle;

// Re-export common traits and structs
pub use character::*;
#[cfg(feature = "ecs")]
pub use oxygen::*;
#[cfg(feature = "ecs")]
pub use ragdoll::*;
#[cfg(feature = "ecs")]
pub use systems::*;
pub use vehicle::*;
