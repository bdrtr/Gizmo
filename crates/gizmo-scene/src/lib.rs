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
//! developer and loaded with [`SceneData::load_into`] (or, for a scene already in memory,
//! [`SceneData::from_ron_str`] + [`SceneData::instantiate_entities`]) instead of hard-coding
//! entity spawns — the declarative alternative to an imperative `load_level()`. See
//! `scene::tests::hand_authored_scene_ron_loads_and_spawns` for a copy-paste template.
//!
//! ```
//! use gizmo_scene::SceneData;
//!
//! let scene = SceneData::from_ron_str("(version: 1, entities: [])")?;
//! assert!(scene.entities.is_empty());
//! let text = scene.to_ron_string()?;
//! assert!(text.contains("entities"));
//! # Ok::<(), gizmo_scene::SceneError>(())
//! ```
//!
//! The RON parser itself is **not** re-exported. It is an implementation detail of the file
//! format, not part of this crate's API: `gizmo-scene` is Stage A (docs/ENGINE.md §4), so a
//! type from a `0.x` dependency on its public surface would hand that dependency the power to
//! force a `gizmo-scene` 2.0. [`SceneData::from_ron_str`] / [`SceneData::to_ron_string`] are
//! the supported string entry points, and the parser's errors arrive wrapped in
//! [`error::ParseError`] / [`error::SerializeError`]:
//!
//! ```compile_fail
//! let _ = gizmo_scene::ron::from_str::<i32>("1");
//! ```
//!
//! (That example compiled while `pub use ron;` existed — verified by running it unmarked
//! against the old `lib.rs`, where it passed as an ordinary doc-test — so it is this seal's
//! regression test. `compile_fail` on its own only asserts *some* error and this toolchain's
//! rustdoc ignores a `compile_fail,E0nnn` code, so the code is recorded by hand: un-marked,
//! the diagnostic is E0433 cannot find `ron` in `gizmo_scene`.)

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

/// Re-export of [`error::SceneError`] together with the two opaque payloads it carries for
/// the RON failure cases, [`error::ParseError`] and [`error::SerializeError`] — a caller
/// matching on `SceneError::Parse`/`::Serialize` needs to be able to name them.
pub use error::{ParseError, SceneError, SerializeError};
/// Re-export of [`registry::SceneRegistry`].
pub use registry::SceneRegistry;
/// Re-exports of the RON scene-file types: [`SceneData`](scene::SceneData) is the file root,
/// [`EntityData`](scene::EntityData) one of its entity rows, and
/// [`MaterialData`](scene::MaterialData) the PBR parameters (albedo RGBA, roughness,
/// metallic, unlit, optional texture path) copied out of a row's `MaterialSource` — none of
/// which the in-memory [`SceneSnapshot`] carries.
pub use scene::{AssetIdentity, EntityData, MaterialData, NoAssetIdentity, SceneData};
/// Re-export of [`snapshot::SceneSnapshot`] — the editor's in-memory Play/Stop backup. Same
/// component registry as the file format, but no disk I/O, no RON file, and no mesh or
/// material data.
pub use snapshot::SceneSnapshot;
// `ron` is deliberately NOT re-exported (`pub use ron;` was removed 2026-08-09). It is a
// private implementation detail of the scene file format: the parser's types appear in no
// signature of ours, only inside the opaque `error::ParseError` / `error::SerializeError`
// payloads and in the two `From` impls that make `?` work internally. A `ron` major release
// is therefore a chore in this crate rather than a breaking change to it — which is the whole
// point, since Stage A crates go to 1.x. Tracked in docs/ENGINE.md §4; the seal's regression
// tests are the `compile_fail` doc-tests in the crate docs above and on
// `error::ParseError` / `error::SerializeError`.
