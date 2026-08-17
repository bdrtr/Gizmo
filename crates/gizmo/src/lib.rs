#![deny(clippy::undocumented_unsafe_blocks)]
//! (`undocumented_unsafe_blocks` is a RATCHET: this crate carries no `unsafe` block without a
//! `// SAFETY:` line stating why it is sound, and the lint keeps it that way. Every crate in the
//! workspace except `gizmo-core` is at zero and denies it; `gizmo-core`'s ECS internals are the
//! measured remainder — see docs/ENGINE.md.)
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
//! `gizmo`. That is declared by `[lib] name = "gizmo"` in the manifest, so
//! `cargo add gizmo-engine` followed by `use gizmo::prelude::*;` works with no
//! rename in your own `Cargo.toml`:
//!
//! ```
//! use gizmo::prelude::*;
//!
//! // The path in every example on this page is the real one.
//! let mut world = World::new();
//! let e = world.spawn();
//! world.add_component(e, Transform::new(Vec3::new(0.0, 1.0, 0.0)));
//! assert_eq!(
//!     world.query::<&Transform>().unwrap().get(e.id()).unwrap().position.y,
//!     1.0
//! );
//! ```
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
//! - `network` — networking / P2P deterministic rollback via `gizmo-net`.
//! - `headless` — run the app loop without a window (e.g. for servers/tests).
//!
//! The `default` feature enables a full desktop game setup (`window`, `render`,
//! `audio`, `physics`, `scene`, `editor`, `ui`, `animation`, `network`).
//!
//! ## Re-exported third-party crates
//!
//! For convenience the facade re-exports the external crates that appear in its
//! public API so downstream users do not have to add them separately:
//! [`wgpu`] and [`bytemuck`] (with `render`), [`egui`] (with `editor`) and
//! [`winit`] (with `window`).

// Feature gating rule for the facade's own modules:
//
// These used to be unconditional `pub mod`s whose bodies referenced the *optional*
// `gizmo-renderer` / `gizmo-physics-*` dependencies unconditionally, so `gizmo-engine`
// only ever compiled with `render` AND `physics` on — including under its own advertised
// `headless` feature. Gate at the narrowest level that still compiles: a whole module
// where every item needs the dependency, individual items where the split is inside.
//
// `Transform` lives in `gizmo-physics-core`, so anything touching transforms needs the
// `physics` feature — that is why several purely-logical modules are gated on it.

/// The path↔UUID bridge between an asset registry and the scene format. Defined in `gizmo-app`,
/// the first layer that holds both halves; re-exported here so `gizmo::asset_identity` works.
#[cfg(all(feature = "scene", feature = "render"))]
pub use gizmo_app::asset_identity;
/// GPU asset loading — requires a renderer.
#[cfg(feature = "render")]
pub mod asset_server;
/// Ready-made component bundles. Light/camera/mesh bundles need `render`; the rigid-body
/// bundle needs `physics` (see the per-item gates inside).
#[cfg(any(feature = "render", feature = "physics"))]
pub mod bundles;
pub mod color;
pub mod plugins;
pub mod prelude;
/// Entity spawning helpers built on the renderer's mesh/material pipeline. They also spawn
/// rigid bodies, hence the `physics` half of the gate.
#[cfg(all(feature = "render", feature = "physics"))]
pub mod spawner;
pub mod systems;
#[cfg(test)]
mod test_gpu;

// === Motor Alt Sistemleri ===
pub use gizmo_ai as ai;
#[cfg(feature = "analysis")]
pub use gizmo_analysis as analysis;
pub use gizmo_app as app;
pub use gizmo_core as core;
pub use gizmo_math as math;
/// Re-exports of the split physics crates under one `gizmo::physics` path.
#[cfg(feature = "physics")]
pub mod physics;
#[cfg(feature = "render")]
pub use gizmo_renderer as renderer;

#[cfg(feature = "window")]
pub use gizmo_window as window;

// Sık kullanılan matematik tiplerini lib.rs'ten doğrudan aç:
pub use math::{Mat4, Quat, Vec2, Vec3, Vec4};

#[cfg(all(feature = "window", feature = "render", feature = "physics"))]
pub mod simple;
#[cfg(all(feature = "window", feature = "render", feature = "physics"))]
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
// `pub use gizmo_scene::ron;` used to sit here. It went away with `gizmo-scene`'s own
// re-export (2026-08-09): the RON parser is an implementation detail of the scene file
// format, not API. What it was there for — turning a hand-written RON level string into a
// scene — is `scene::SceneData::from_ron_str` / `to_ron_string` now. This facade is Stage B
// and could have kept leaking the parser (docs/ENGINE.md §4), but only by taking a direct
// dependency on it and pinning that pin in lock-step with `gizmo-scene`'s, where a drift
// between the two would hand callers a parser type the `From` impls in `gizmo-scene` do not
// accept.

/// A [`scene::registry::SceneRegistry`] holding every component the enabled feature set
/// can round-trip — not just the physics ones.
///
/// `gizmo-scene` deliberately depends on neither the renderer nor the scripting layer, so
/// that scene save/load works in a GPU-free headless build. The cost is that
/// [`scene::registry::default_scene_registry`] can only register what physics owns:
/// transforms, bodies, colliders and the fighter components. Everything a scene visibly
/// consists of — lights, cameras, audio emitters — lived outside its reach, so saving a
/// scene from the editor and loading it back returned the physics and dropped the rest,
/// silently.
///
/// This is the facade's job, because the facade is the layer that can see all of them. Use
/// it wherever you would otherwise call `default_scene_registry`.
///
/// To round-trip your *own* components, register them on the result — anything that is
/// `Component + Serialize + DeserializeOwned` qualifies:
///
/// ```no_run
/// # use gizmo::scene::scene::SceneData;
/// # #[derive(Clone, serde::Serialize, serde::Deserialize)]
/// # struct Health(f32);
/// # gizmo::core::impl_component!(Health);
/// let mut registry = gizmo::full_scene_registry();
/// registry
///     .register_serializable::<Health>("Health")
///     .expect("name must not collide with a built-in");
/// // `registry` now round-trips Health alongside everything the engine registers.
/// ```
#[cfg(feature = "scene")]
pub fn full_scene_registry() -> scene::registry::SceneRegistry {
    app::scene_registry::full_scene_registry()
}

#[cfg(feature = "ui")]
pub use gizmo_ui as ui;

#[cfg(feature = "animation")]
pub use gizmo_animation as animation;

#[cfg(feature = "network")]
pub use gizmo_net as net;

// === 3. Parti Re-Export (Kullanıcının ayrıca eklemesine gerek kalmasın) ===
pub use gizmo_core::gizmo_log;

/// 1.0 contract: the external graphics/window types below (`wgpu`, `bytemuck`,
/// `egui`, `winit`) are deliberately part of the public API. Their versions
/// depend on the semver of the relevant renderer/window crate; a major version
/// bump in these external crates counts as breaking for the facade too.
#[cfg(feature = "render")]
pub use bytemuck;

/// 1.0 contract: this external graphics type is deliberately part of the public
/// API; its version depends on the semver of the relevant UI crate. It is enabled
/// with the `egui` feature (overlay UI / editor).
#[cfg(feature = "egui")]
pub use egui;

/// 1.0 contract: this external graphics type is deliberately part of the public
/// API; its version depends on the semver of the renderer crate.
#[cfg(feature = "render")]
pub use wgpu;

/// 1.0 contract: this external window type is deliberately part of the public
/// API; its version depends on the semver of the window crate.
#[cfg(feature = "window")]
pub use winit;
