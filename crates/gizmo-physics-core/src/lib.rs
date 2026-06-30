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
pub mod broadphase;
pub mod bvh;
pub mod collision;
pub mod components;
pub mod error;
pub mod gjk;
pub mod narrowphase;
pub mod quickhull;
pub mod raycast;
pub mod shape;

pub use body::BodyHandle;
pub use broadphase::SpatialHash;
pub use gizmo_math::Aabb;

pub use collision::{
    CollisionEvent, CollisionEventType, ContactManifold, ContactPoint, ContactPoints, TriggerEvent, FractureEvent
};
pub use components::{
    Collider, ColliderShape, CollisionLayer, CombineMode, ConvexHullShape, PhysicsMaterial,
    PlaneShape, SphereShape, Transform, TriMeshShape, BoxShape, CapsuleShape
};
pub use error::GizmoError;
pub use gjk::Gjk;
pub use narrowphase::NarrowPhase;
pub use raycast::{Ray, Raycast, RaycastHit};
