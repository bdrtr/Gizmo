//! # Gizmo Engine
//!
//! `gizmo-engine` is the all-in-one facade crate of the Gizmo game engine. It
//! re-exports the individual subsystem crates (core ECS, math, app loop,
//! physics, renderer, windowing, audio, scene, editor, UI, animation and AI)
//! and adds an ergonomic, Bevy-like convenience layer on top: [`Color`],
//! ready-made [`bundles`], a [`spawner`] API and prefabricated scene helpers.
//!
//! Note that the published *package* is named `gizmo-engine`, while the *library*
//! (and thus the crate path used in `use` statements and examples) is simply
//! `gizmo`.
//!
//! ## Feature flags
//!
//! Subsystems are gated behind Cargo features so you only compile what you need:
//!
//! - `window` — windowing via `winit`.
//! - `render` — the `wgpu`-based renderer (implies `window`).
//! - `audio` — audio playback.
//! - `physics`, `physics-dynamics`, `physics-soft` — physics subsystems.
//! - `scene` — scene (de)serialization.
//! - `editor` — the `egui`-based in-engine editor (implies `render`).
//! - `ui` — the UI subsystem.
//! - `animation` — skeletal/property animation.
//! - `scripting` — scripting support.
//! - `headless` — run the app loop without a window (e.g. for servers/tests).
//!
//! The `default` feature enables a full desktop game setup (`window`, `render`,
//! `audio`, `physics`, `scene`, `editor`, `ui`, `animation`).
//!
//! ## Re-exported third-party crates
//!
//! For convenience the facade re-exports the external crates that appear in its
//! public API so downstream users do not have to add them separately:
//! [`wgpu`] and [`bytemuck`] (with `render`), [`egui`] (with `editor`) and
//! [`winit`] (with `window`).

pub mod asset_server;
pub mod bundles;
pub mod color;
pub mod plugins;
pub mod prelude;
pub mod spawner;
pub mod systems;

// === Motor Alt Sistemleri ===
pub use gizmo_ai as ai;
pub use gizmo_app as app;
pub use gizmo_core as core;
pub use gizmo_math as math;
pub mod physics;
#[cfg(feature = "render")]
pub use gizmo_renderer as renderer;

#[cfg(feature = "window")]
pub use gizmo_window as window;

// Sık kullanılan matematik tiplerini lib.rs'ten doğrudan aç:
pub use math::{Mat4, Quat, Vec2, Vec3, Vec4};

#[cfg(feature = "window")]
pub mod simple;
#[cfg(feature = "window")]
pub use simple::*;

// === Opsiyonel Modüller ===
#[cfg(feature = "audio")]
pub use gizmo_audio as audio;

#[cfg(feature = "editor")]
pub use gizmo_editor as editor;

#[cfg(feature = "scripting")]
pub use gizmo_scripting as scripting;

#[cfg(feature = "scene")]
pub use gizmo_scene as scene;
#[cfg(feature = "scene")]
pub use gizmo_scene::ron;

#[cfg(feature = "ui")]
pub use gizmo_ui as ui;

#[cfg(feature = "animation")]
pub use gizmo_animation as animation;

// === 3. Parti Re-Export (Kullanıcının ayrıca eklemesine gerek kalmasın) ===
pub use gizmo_core::gizmo_log;

#[cfg(feature = "render")]
pub use bytemuck;

#[cfg(feature = "editor")]
pub use egui;

#[cfg(feature = "render")]
pub use wgpu;

#[cfg(feature = "window")]
pub use winit;
