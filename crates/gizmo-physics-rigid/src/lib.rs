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
//! - **`multibody`** — Featherstone Articulated Body Algorithm (ABA) for
//!   reduced-coordinate articulations.
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
//! - [`vehicle`], [`multibody`], [`system`], [`world`] — vehicle dynamics,
//!   articulated bodies, ECS systems and the world container.

pub mod components;
pub mod multibody;
pub mod destruction;
pub mod fracture;
pub mod integrator;
pub mod island;
pub mod joints;
pub(crate) mod pipeline;
pub mod solver;
pub mod system;
pub mod vehicle;
pub mod world;

pub use components::{Breakable, Explosion, RigidBody, Velocity, BodyType, Vehicle, Wheel};
pub use destruction::*;
pub use fracture::{generate_fracture_chunks, voronoi_shatter, PreFracturedCache};
pub use integrator::Integrator;
pub use island::{Island, IslandManager, PhysicsMetrics};
pub use joints::{
    BallSocketJointData, HingeJointData, Joint, JointData, JointSolver, JointType, SliderJointData,
    SpringJointData,
};
pub use solver::ConstraintSolver;
pub use system::{physics_explosion_system, physics_fracture_system, physics_step_system};
pub use vehicle::physics_vehicle_system;
pub use world::{PhysicsWorld, SnapshotError, WorldSnapshot};
