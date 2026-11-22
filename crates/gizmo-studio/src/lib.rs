#![deny(clippy::undocumented_unsafe_blocks)]
#![warn(missing_docs)]
//! (`undocumented_unsafe_blocks` is a RATCHET: this crate carries no `unsafe` block without a
//! `// SAFETY:` line stating why it is sound, and the lint keeps it that way. Every crate in the
//! workspace except `gizmo-core` is at zero and denies it; `gizmo-core`'s ECS internals are the
//! measured remainder — see docs/ENGINE.md.)
//! Gizmo Studio: the standalone editor application for the Gizmo game engine.
//!
//! # Why an app has a library target
//!
//! Studio is not a published library and never will be (`publish = false`) — this target exists so
//! that **the editor's draw can be reached from a test**: `tests/studio_render_pixels.rs` reads
//! back what the Scene and Game panels show.
//!
//! It was made for a second reason that is gone. The editor had a render path of its own,
//! [`render_pipeline`] then, beside the game's deferred renderer in `gizmo::systems::render`, and
//! the only cross-check between the two was a human comparing two windows. Every drift found was
//! found by reading — `BakedLit` routed one way and defaulted the other, the editor's DoF
//! linearising depth against a hardcoded range, a light array read from the raw `Transform` in one
//! and `GlobalTransform` in the other — and `tests/render_parity.rs` held the two side by side.
//! Since 2026-11-22 both panels are drawn through the game path, and there is one renderer.
//!
//! The binary (`main.rs`) is only the entry point: [`run`] boots a [`gizmo::App`](gizmo::prelude::App) window and wires
//! the setup/update/UI/render hooks to the modules below — on a game, too ([`StudioGame`]).

/// The studio's render hook: editor objects, the scene/game viewports and their render targets.
pub mod render;
/// The studio's two panels, both drawn through the game path (`draw_camera_into`).
pub mod render_pipeline;
/// Builds the studio's starting scene: the editor camera, the grid, the default light.
pub mod setup;
/// The studio's runtime state — cameras, timers, frame counters — and the table every spawned
/// primitive's size is read from.
pub mod state;
/// The mesh an entity draws, from its component or from its handle into `Assets`.
pub mod drawn_mesh;
/// Viewport picking: the mouse ray, OBB selection and the rubber band.
pub mod studio_input;
/// The per-frame systems the studio's update hook runs, one module each.
pub mod systems;
/// The studio's update hook — the per-frame entry point the engine calls.
pub mod update;
// ▶ in a world of its own: see its module docs.
pub mod pie;
// The studio as a library call, on a game or on its own: see its module docs.
pub mod app;

pub use app::run;

pub use state::{DebugAssets, StudioGame, StudioState};
pub use studio_input::*;
