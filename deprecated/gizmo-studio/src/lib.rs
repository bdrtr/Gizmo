//! # `gizmo-studio` is deprecated
//!
//! This name belongs to an earlier generation of the engine, published from a self-hosted
//! repository that no longer backs it. Its last real release was 0.1.7 (2026-06-02).
//!
//! **There is no successor on crates.io, and that is deliberate.** The editor is an application
//! rather than a library, so the current `gizmo-studio` is marked `publish = false` and ships
//! with the engine repository instead:
//!
//! ```text
//! git clone https://github.com/bdrtr/Gizmo
//! cargo run --release -p gizmo-studio
//! ```
//!
//! If what you wanted was the editor's building blocks rather than the application, those are
//! published: [`gizmo-editor`](https://crates.io/crates/gizmo-editor) for the panels and
//! inspector, [`gizmo-engine`](https://crates.io/crates/gizmo-engine) for the facade.
//!
//! The engine now lives at <https://github.com/bdrtr/Gizmo>.
#![deprecated(note = "DEPRECATED — the editor is no longer published to crates.io; it ships with the engine repository.")]
#![no_std]
