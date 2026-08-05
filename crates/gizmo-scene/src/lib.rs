#![warn(missing_docs)]
//! (`missing_docs` is a RATCHET, not a suggestion. The CI lint gate runs with `-D warnings`,
//! so every public item in this crate must carry a doc comment or the build fails. This crate
//! is Stage A — the dependency-light core that goes to 1.x first — and its documented surface
//! is part of that promise. Do not silence this with `#[allow]`; write the doc.)

//! Gizmo Scene — scene serialization and management.
//!
//! This crate persists and restores ECS [`World`](gizmo_core::World) state:
//!
//! - [`scene`]: on-disk scene/prefab serialization to RON files
//!   ([`SceneData`], [`MaterialData`], [`EntityData`]).
//! - [`snapshot`]: fast in-memory snapshots for the editor's Play/Stop flow
//!   ([`SceneSnapshot`]), with no disk I/O.
//! - [`registry`]: the [`SceneRegistry`] describing which components can be
//!   (de)serialized, plus [`default_scene_registry`](registry::default_scene_registry).
//!
//! It is used by the editor and the runtime.
//!
//! Scenes are also **hand-authorable**: a RON file (or string) can be written by a
//! developer and loaded with [`SceneData::load_into`] (or `ron::from_str` +
//! [`SceneData::instantiate_entities`]) instead of hard-coding entity spawns — the
//! declarative alternative to an imperative `load_level()`. See
//! `scene::tests::hand_authored_scene_ron_loads_and_spawns` for a copy-paste template.

pub mod error;
/// Which components a scene or snapshot is allowed to (de)serialize.
///
/// [`SceneRegistry`](registry::SceneRegistry) is an alias for `gizmo-core`'s
/// `ComponentRegistry`; [`default_scene_registry`](registry::default_scene_registry) fills
/// one with the engine's built-in physics/gameplay components. A component that is *not*
/// registered is invisible to both save/load and snapshot capture — silently, so a forgotten
/// registration looks exactly like data loss. Renderer, audio and scripting components are
/// added a layer up (`gizmo-app`'s `full_scene_registry`).
pub mod registry;
/// The on-disk scene and prefab format: [`SceneData`](scene::SceneData) /
/// [`PrefabData`](scene::PrefabData) written as RON, stamped with
/// [`CURRENT_SCENE_VERSION`](scene::CURRENT_SCENE_VERSION).
///
/// Unlike [`snapshot`], this path also records mesh/material sources and parent links, so it
/// is the only one that can rebuild a scene's GPU-side resources and hierarchy from nothing.
pub mod scene;
mod serde_bridge;
pub mod snapshot;

/// Re-export of [`error::SceneError`].
pub use error::SceneError;
/// Re-export of [`registry::SceneRegistry`].
pub use registry::SceneRegistry;
/// Re-exports of the RON scene-file types: [`SceneData`](scene::SceneData) is the file root,
/// [`EntityData`](scene::EntityData) one of its entity rows, and
/// [`MaterialData`](scene::MaterialData) the PBR parameters (albedo RGBA, roughness,
/// metallic, unlit, optional texture path) copied out of a row's `MaterialSource` — none of
/// which the in-memory [`SceneSnapshot`] carries.
pub use scene::{EntityData, MaterialData, SceneData};
/// Re-export of [`snapshot::SceneSnapshot`] — the editor's in-memory Play/Stop backup. Same
/// component registry as the file format, but no disk I/O, no RON file, and no mesh or
/// material data.
pub use snapshot::SceneSnapshot;
/// `ron` is a deliberate, intentional **public dependency**: the scene file format is
/// RON and [`SceneError`] exposes `ron::error::SpannedError` / `ron::Error` in its public
/// API. As with `glam` in `gizmo-math`, a `ron` major-version bump is therefore a breaking
/// change to this crate's public API and is treated as a breaking `gizmo-scene` bump for
/// semver. (`ron` is currently a `0.x` crate; this is tracked in docs/ENGINE.md §4.)
pub use ron;
