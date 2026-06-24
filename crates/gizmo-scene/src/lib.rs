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
//! It is used by the editor, Lua scripting, and the runtime.

pub mod error;
pub mod registry;
pub mod scene;
pub mod snapshot;

/// Re-export of [`error::SceneError`].
pub use error::SceneError;
/// Re-export of [`registry::SceneRegistry`].
pub use registry::SceneRegistry;
pub use scene::{EntityData, MaterialData, SceneData};
pub use snapshot::SceneSnapshot;
pub use ron;
