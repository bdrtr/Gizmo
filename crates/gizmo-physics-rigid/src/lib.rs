#![warn(missing_docs)]
//! (`missing_docs` is a RATCHET, not a suggestion. The CI lint gate runs with `-D warnings`,
//! so every public item in this crate must carry a doc comment or the build fails. This crate
//! is Stage A — the dependency-light core that goes to 1.x first — and its documented surface
//! is part of that promise. Do not silence this with `#[allow]`; write the doc.)

//! Pure-Rust rigid-body physics engine for the Gizmo game engine.
//!
//! This crate provides a deterministic, fixed-substep rigid-body simulation
//! built around a Structure-of-Arrays [`PhysicsWorld`]. It is independent of
//! any rendering backend and operates on ECS-style components.
//!
//! # Core concepts
//!
//! - **[`PhysicsWorld`]** — the central simulation container. It stores bodies,
//!   transforms, velocities and colliders in parallel arrays (SoA) and drives
//!   the simulation forward with a fixed-substep accumulator (decoupled from the
//!   render frame rate) for reproducible, deterministic results.
//! - **[`Integrator`]** — semi-implicit Euler integration of velocities and
//!   positions, including forces, torques, damping and axis locks.
//! - **[`ConstraintSolver`]** — sequential-impulse contact solver with
//!   warm-starting, friction cones and an optional TGS-soft constraint path.
//! - **[`JointSolver`] / [`Joint`]** — articulated constraints (ball-socket,
//!   hinge, slider, spring).
//! - **[`IslandManager`] / [`Island`]** — splits the world into independent
//!   contact islands so they can be solved in parallel and put to sleep.
//! - **[`DestructionSystem`] and fracture utilities** — runtime breaking and
//!   pre-fractured Voronoi shattering for destructible objects.
//! - **`multibody`** *(experimental, `experimental-multibody` feature)* —
//!   Featherstone Articulated Body Algorithm (ABA) for reduced-coordinate
//!   articulations. Off by default; see the module docs for its limitations.
//!
//! # Determinism
//!
//! The simulation advances in fixed substeps and is designed to produce
//! identical results given identical inputs, enabling snapshot-based rollback
//! and replay via [`WorldSnapshot`].
//!
//! # Module map
//!
//! - [`components`] — ECS components ([`RigidBody`], [`Velocity`], [`Vehicle`],
//!   [`Breakable`], [`Explosion`]).
//! - [`integrator`], [`solver`], [`joints`], [`island`] — the solver stack.
//! - [`destruction`], [`fracture`] — destruction and shattering.
//! - [`vehicle`], [`system`], [`world`] — vehicle dynamics, ECS systems and the
//!   world container. (`multibody` is an opt-in experimental module — above.)

// Sequential fallback for rayon on wasm (no OS threads); native uses rayon.
#[cfg(target_arch = "wasm32")]
mod parallel_compat;
/// ECS components describing a simulated body: [`RigidBody`] (body type, mass in kg,
/// damping, axis locks, sleep bookkeeping and the per-step force/torque accumulators),
/// [`Velocity`] (linear m/s, angular rad/s as a scaled axis, plus the previous-substep
/// values the trapezoidal position integrator needs), and the destruction inputs
/// [`Breakable`] (health/impulse threshold) and [`Explosion`] (radial force + falloff).
///
/// Contact friction and restitution are deliberately *not* stored here — they come from
/// the collider's material and are combined per contact pair.
pub mod components;
/// Experimental articulated-body (multibody) dynamics — opt-in, off by default.
/// See the crate's `experimental-multibody` feature and the module docs.
#[cfg(feature = "experimental-multibody")]
pub mod multibody;
/// Impact-driven runtime breaking. [`DestructionSystem`] inspects the world's collision
/// events and reports the bodies whose impact impulse exceeded their threshold; it is a
/// pure observer — it neither mutates the world nor spawns the debris itself.
pub mod destruction;
/// Voronoi shattering and debris generation. The shatter is driven by an explicit `u64`
/// seed, so the same seed reproduces the same chunk set on the same build (this is what
/// keeps destruction replay-safe). [`PreFracturedCache`] trades memory for frame time by
/// shattering ahead of time — typically during loading — so the runtime path is a clone.
pub mod fracture;
/// Time integration of a single body: semi-implicit Euler for velocity (gravity, the
/// force/torque accumulators, aerodynamic drag, wind, exponential damping) followed by
/// trapezoidal (Heun) integration of position and orientation. See [`Integrator`].
pub mod integrator;
/// Partitioning of the contact set into independent [`Island`]s (union-find over the
/// contact graph) so they can be solved in parallel and slept as a unit, plus the
/// per-frame profiling counters in [`PhysicsMetrics`]. Islands hold no state across
/// frames — they are rebuilt from scratch every substep.
pub mod island;
/// Articulated constraints — fixed, hinge, ball-socket, slider, spring, distance and the
/// generic 6-DOF joint — together with the iterative [`JointSolver`] that runs after the
/// contact solver within each substep.
pub mod joints;
pub(crate) mod pipeline;
/// Contact constraint solving. [`ConstraintSolver`] carries the tuning for both the
/// default TGS-soft path and the legacy split-impulse sequential-impulse path; it holds
/// no per-frame state (accumulated impulses live in the contact manifolds), which is why
/// it is a plain `Copy` config value.
pub mod solver;
/// ECS glue between a `gizmo-core` `World` and the [`PhysicsWorld`] resource: stepping the
/// simulation and writing the results back, spawning fracture debris, and applying
/// explosion impulses.
/// Requires the `ecs` feature — this is the ECS bridge.
#[cfg(feature = "ecs")]
pub mod system;
/// The [`PhysicsWorld`] container: construction and body management, fixed-substep
/// stepping, scene queries and rollback snapshots. Bodies live in parallel
/// structure-of-arrays vectors that share one index; removal is a swap-remove, so an index
/// obtained from the world is not stable across body removal.
pub mod world;

pub use gizmo_physics_core::BodyHandle;
pub use components::{Breakable, Explosion, RigidBody, Velocity, BodyType};
pub use destruction::*;
pub use fracture::{generate_fracture_chunks, voronoi_shatter, PreFracturedCache};
pub use integrator::Integrator;
pub use island::{Island, IslandManager, PhysicsMetrics};
pub use joints::{
    BallSocketJointData, D6Drive, D6JointData, D6Motion, DistanceJointData, HingeJointData, Joint,
    JointData, JointSolver, JointType, SliderJointData, SpringJointData,
};
pub use solver::{ConstraintSolver, SolveStats};
#[cfg(feature = "ecs")]
pub use system::{physics_explosion_system, physics_fracture_system, physics_step_system};
pub use world::{PhysicsWorld, SnapshotError, WorldSnapshot};
