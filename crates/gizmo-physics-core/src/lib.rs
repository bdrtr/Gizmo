#![deny(clippy::undocumented_unsafe_blocks)]
//! (`undocumented_unsafe_blocks` is a RATCHET: this crate carries no `unsafe` block without a
//! `// SAFETY:` line stating why it is sound, and the lint keeps it that way. Every crate in the
//! workspace except `gizmo-core` is at zero and denies it; `gizmo-core`'s ECS internals are the
//! measured remainder — see docs/ENGINE.md.)
#![warn(missing_docs)]
//! (`missing_docs` is a RATCHET, not a suggestion. The CI lint gate runs with `-D warnings`,
//! so every public item in this crate must carry a doc comment or the build fails. This crate
//! is Stage A — the dependency-light core that goes to 1.x first — and its documented surface
//! is part of that promise. Do not silence this with `#[allow]`; write the doc.)

//! Core physics primitives for the Gizmo engine, written in pure Rust.
//!
//! This crate provides the data types and algorithms that underpin collision
//! detection and spatial queries:
//!
//! - **Components**: ECS-friendly building blocks such as [`Collider`],
//!   [`Transform`], [`PhysicsMaterial`] and the collider shape variants
//!   ([`SphereShape`], [`BoxShape`], [`CapsuleShape`], [`PlaneShape`],
//!   [`ConvexHullShape`], [`TriMeshShape`]).
//! - **Broadphase**: a [`SpatialHash`] and a [`bvh`] for quickly pruning
//!   pairs that cannot possibly collide.
//! - **Narrowphase**: exact contact generation via [`Gjk`] (GJK/EPA) and SAT,
//!   producing a [`ContactManifold`] of [`ContactPoint`]s.
//! - **Queries**: [`Raycast`] support against the above shapes.
//! - **Geometry**: a Quickhull implementation ([`quickhull`]) for building
//!   convex hulls from point clouds.
//!
//! Most fallible operations return [`Result`] with [`GizmoError`] or
//! [`Option`], so the public surface avoids panicking on bad input.

pub mod body;
/// Broadphase pair pruning over **fattened** world-space AABBs.
///
/// Holds the incremental [`DynamicAabbTree`](broadphase::DynamicAabbTree) and the
/// [`SpatialHash`] shim that forwards to it. Because the boxes stored here are
/// conservative superset: pairs and hits reported here still have to be confirmed
/// by the [`narrowphase`] or by [`raycast`].
pub mod broadphase;
pub mod bvh;
pub mod collision;
pub mod components;
pub mod error;
pub mod gjk;
pub mod narrowphase;
pub mod quickhull;
pub mod raycast;
/// Short aliases for the collider shape types (`Sphere`, `Capsule`, `Plane`, …).
///
/// Beware of one collision of names: `shape::Aabb` aliases [`BoxShape`] — a
/// collider defined by half-extents — and is **not** [`gizmo_math::Aabb`], the
/// min/max bounding box this crate also re-exports at its root.
pub mod shape;

pub use body::BodyHandle;
pub use broadphase::SpatialHash;
pub use gizmo_math::Aabb;

pub use collision::{
    CollisionEvent, CollisionEventType, ContactManifold, ContactPoint, ContactPoints, ContactPointsIter, TriggerEvent, FractureEvent
};
pub use components::{
    Collider, ColliderShape, CollisionLayer, CombineMode, ConvexHullShape, PhysicsMaterial,
    PlaneShape, SphereShape, Transform, TriMeshShape, BoxShape, CapsuleShape
};
pub use error::GizmoError;
pub use gjk::Gjk;
pub use narrowphase::NarrowPhase;
pub use raycast::{Ray, Raycast, RaycastHit};
