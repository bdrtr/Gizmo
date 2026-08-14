//! Gizmo Studio: the standalone editor application for the Gizmo game engine.
//!
//! # Why an app has a library target
//!
//! Studio is not a published library and never will be (`publish = false`) — this target exists so
//! that **the editor's render path can be reached from a test**. The engine has two of those paths:
//! the game's deferred renderer in `gizmo::systems::render`, and
//! [`render_pipeline::execute_render_pipeline`] here. They share their per-frame *setup* on
//! purpose (lights, cascades, uniform blocks) and differ only in pass recording, but for as long
//! as studio was a bare binary, nothing on the engine side could observe the studio half: the only
//! cross-check between the two was a human comparing two windows.
//!
//! Every drift found in those paths so far — `BakedLit` routed one way and defaulted the other,
//! the editor's DoF linearising depth against a hardcoded range, a light array read from the raw
//! `Transform` in one and `GlobalTransform` in the other — was found by reading, not by failing.
//! A `lib.rs` is the cheapest thing that changes that, and `tests/` next to it is where the parity
//! checks live.
//!
//! The binary (`main.rs`) is now only the entry point: it boots a [`gizmo::App`] window and wires
//! the setup/update/UI/render hooks to the modules below.

pub mod render;
pub mod render_pipeline;
pub mod setup;
pub mod state;
pub mod studio_input;
pub mod systems;
pub mod update;

pub use state::{DebugAssets, StudioState};
pub use studio_input::*;
