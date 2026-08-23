//! # `gizmo-physics` is deprecated
//!
//! This name belongs to an earlier generation of the engine, published from a self-hosted
//! repository that no longer backs it. Its last real release was 0.1.2 (2026-05-13).
//!
//! **It was not renamed — it was split**, because "physics" turned out to be four libraries with
//! different dependency weights, and shipping them as one forced every consumer to take all of it:
//!
//! | crate | what it is |
//! |---|---|
//! | [`gizmo-physics-core`](https://crates.io/crates/gizmo-physics-core) | shapes, GJK/EPA, BVH, broadphase, raycasts — usable without an ECS |
//! | [`gizmo-physics-rigid`](https://crates.io/crates/gizmo-physics-rigid) | the rigid-body solver, joints, CCD |
//! | [`gizmo-physics-dynamics`](https://crates.io/crates/gizmo-physics-dynamics) | vehicles, characters, buoyancy |
//! | [`gizmo-physics-soft`](https://crates.io/crates/gizmo-physics-soft) | FEM soft bodies, cloth, rope, fracture |
//!
//! If you want collision and geometry and nothing else, `gizmo-physics-core` with
//! `default-features = false` is the one to reach for.
//!
//! The engine now lives at <https://github.com/bdrtr/Gizmo>.
#![deprecated(note = "DEPRECATED — split into gizmo-physics-core, gizmo-physics-rigid, gizmo-physics-dynamics and gizmo-physics-soft.")]
#![no_std]
