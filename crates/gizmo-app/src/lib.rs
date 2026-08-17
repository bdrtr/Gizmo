#![deny(clippy::undocumented_unsafe_blocks)]
#![warn(missing_docs)]
//! (`undocumented_unsafe_blocks` is a RATCHET: this crate carries no `unsafe` block without a
//! `// SAFETY:` line stating why it is sound, and the lint keeps it that way. Every crate in the
//! workspace except `gizmo-core` is at zero and denies it; `gizmo-core`'s ECS internals are the
//! measured remainder — see docs/ENGINE.md.)
//! Application skeleton for the Gizmo engine.
//!
//! This crate provides the top-level [`App`] builder that wires together the
//! ECS [`World`](gizmo_core::world::World), a system
//! [`Schedule`](gizmo_core::system::Schedule), user lifecycle hooks and the
//! main loop. It also defines the [`Plugin`] trait used to bundle reusable
//! setup logic.
//!
//! # Feature-gated `App`
//!
//! Two different `App` types are exported depending on the enabled features:
//!
//! - With the `window` feature (default), [`windowed::App`] is re-exported.
//!   It opens a real window, drives a winit event loop and (with the `render`
//!   / `editor` features) integrates the renderer and editor UI.
//! - Without the `window` feature, [`headless::App`] is re-exported instead.
//!   It runs a minimal update loop with no window or GPU.
//!
//! The two variants have different hook signatures (for example, the windowed
//! `set_setup` receives a renderer reference while the headless one does not),
//! so code that targets both must account for the active feature set.
//!
//! Builder methods are typically chained, ending with `run`, in the order
//! `new` -> `set_setup` -> `set_update` -> optional render/UI hooks -> `run`.

/// The path↔UUID bridge between an asset registry and the scene format (see the module docs).
///
/// Needs both halves — `gizmo-renderer`'s `AssetManager` and `gizmo-scene`'s `AssetIdentity` — so
/// it lives here, at the first layer that has them, rather than in the facade above: this way the
/// app's own initial-scene load uses it, and every game gets identity repair without wiring it.
#[cfg(all(feature = "scene", feature = "render"))]
pub mod asset_identity;
/// The in-engine developer console: a `~`-style overlay that reads and writes
/// [`gizmo_core::cvar`] variables and prints the engine's log buffer. Drawn by the windowed
/// loop when the `egui` feature is on, over whatever the game is rendering.
#[cfg(feature = "egui")]
pub mod dev_console;
/// Generic immediate-mode overlay UI runtime (egui integration).
#[cfg(feature = "egui")]
pub mod egui_ctx;
/// Per-frame editor integration (scene/game RTT + scene save/load), kept out of
/// the windowed event loop.
#[cfg(feature = "editor")]
pub mod editor_runtime;
/// Per-frame simulation stepping — the fixed-timestep loop and the once-per-frame
/// update schedule. Dependency-free, so it is available in every configuration.
pub mod frame;
/// Physical game controllers → `gizmo_core::input::Input`, on every target.
///
/// The browser included: gilrs's wasm backend reads the Web Gamepad API and turns the browser's
/// *polled* snapshot into the same event stream the native backends produce, so nothing above
/// this module knows which one it is talking to.
#[cfg(feature = "gamepad")]
pub mod gamepad;
/// The registry scene save/load uses. NOT `gizmo_scene::registry::default_scene_registry`
/// — see the module docs for why.
#[cfg(feature = "scene")]
pub mod scene_registry;
/// High-level gameplay physics systems (vehicle / character controllers) wired
/// into the app schedule. Requires the `physics` feature.
#[cfg(feature = "physics")]
pub mod gameplay;
/// The plugin trait and the `AppLike` abstraction over the two runtimes.
///
/// A [`Plugin`] is how a subsystem installs itself — resources, systems, schedules — without the
/// app knowing what it is. It speaks `AppLike` rather than a concrete `App`, which is what lets
/// the same plugin be applied to the windowed and the headless runtime.
pub mod plugin;

pub use plugin::{AppLike, Plugin};

/// Errors that can occur while building and running an [`App`].
///
/// This is the concrete error surface for the application entry points
/// (`App::run` and friends). It is marked `#[non_exhaustive]` so new failure
/// modes can be added without breaking downstream `match` arms.
#[derive(Debug)]
#[non_exhaustive]
pub enum AppError {
    /// No setup hook was assigned before [`App::run`] was called.
    ///
    /// Call `set_setup` (or configure a runner) before running the app.
    MissingSetup,
    /// The windowing event loop could not be created.
    #[cfg(feature = "window")]
    EventLoopCreation(winit::error::EventLoopError),
    /// The application window could not be created.
    #[cfg(feature = "window")]
    WindowCreation(winit::error::OsError),
    /// A resource that was expected to be present in the world was missing.
    ///
    /// Carries the (type) name of the missing resource. This generally
    /// indicates an internal invariant violation rather than user error.
    MissingResource(&'static str),
    /// The event loop returned an error while running.
    #[cfg(feature = "window")]
    EventLoop(winit::error::EventLoopError),
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppError::MissingSetup => write!(
                f,
                "setup hook was not assigned; call set_setup() before run()"
            ),
            #[cfg(feature = "window")]
            AppError::EventLoopCreation(_) => write!(f, "failed to create the event loop"),
            #[cfg(feature = "window")]
            AppError::WindowCreation(_) => write!(f, "failed to create the application window"),
            AppError::MissingResource(name) => {
                write!(f, "required resource `{}` was missing from the world", name)
            }
            #[cfg(feature = "window")]
            AppError::EventLoop(_) => write!(f, "the event loop returned an error"),
        }
    }
}

impl std::error::Error for AppError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            #[cfg(feature = "window")]
            AppError::EventLoopCreation(e) => Some(e),
            #[cfg(feature = "window")]
            AppError::WindowCreation(e) => Some(e),
            #[cfg(feature = "window")]
            AppError::EventLoop(e) => Some(e),
            _ => None,
        }
    }
}

// The two app runtimes coexist; they are NOT mutually exclusive.
//
// This used to read `#[cfg(feature = "window")] pub mod windowed; ... #[cfg(not(feature =
// "window"))] pub mod headless;` — i.e. enabling `window` *deleted* `headless::App`. Cargo
// unifies features across the whole dependency graph, so a headless simulation server could
// be broken by an unrelated crate that happened to turn `window` on: its `App` silently
// became the windowed one and every `set_setup`/`run` call stopped type-checking. A feature
// that removes API is non-additive, which is exactly the pattern Cargo tells you to avoid.
//
// Now each module is gated only on what it actually needs, and both are always addressable
// by path (`gizmo_app::windowed::App`, `gizmo_app::headless::App`). The glob re-export at
// the crate root still resolves `App` to whichever runtime is available, preferring the
// windowed one when both are — so existing `gizmo_app::App` code keeps compiling.

/// The windowed runtime (winit event loop + wgpu surface). Needs `render`: the loop owns the
/// surface, drives the renderer and reconfigures on resize/surface-loss, so there is no
/// meaningful "window without renderer" build.
#[cfg(all(feature = "window", feature = "render"))]
pub mod windowed;

/// The headless runtime — a fixed-timestep loop with no window and no GPU. This is what a
/// dedicated simulation/game server uses, and it stays available no matter which other
/// features are on.
pub mod headless;

#[cfg(all(feature = "window", feature = "render"))]
pub use windowed::*;
// Only glob the headless runtime into the crate root when the windowed one is absent,
// otherwise the two `App` types would collide. Both remain reachable by full path.
#[cfg(not(all(feature = "window", feature = "render")))]
pub use headless::*;

/// Installs the Gizmo engine panic hook.
///
/// On native targets this logs the panic location and message, captures a
/// backtrace and (with the `window` feature) shows an error dialog. On
/// `wasm32` it wires up `console_error_panic_hook` and console/tracing
/// logging. Safe to call more than once.
pub fn setup_panic_hook() {
    #[cfg(target_arch = "wasm32")]
    {
        console_error_panic_hook::set_once();
        let _ = console_log::init_with_level(log::Level::Debug);
        let _ = tracing_wasm::try_set_as_global_default();
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::panic::set_hook(Box::new(|panic_info| {
            let message = if let Some(s) = panic_info.payload().downcast_ref::<&str>() {
                *s
            } else if let Some(s) = panic_info.payload().downcast_ref::<String>() {
                s.as_str()
            } else {
                "Bilinmeyen hata"
            };

            let location = if let Some(loc) = panic_info.location() {
                format!("{}:{}", loc.file(), loc.line())
            } else {
                "Bilinmeyen konum".to_string()
            };

            let error_msg = format!("Gizmo Engine Coktu!\n\nKonum: {}\nHata: {}\n", location, message);
            tracing::error!("{}", error_msg);

            #[cfg(feature = "window")]
            {
                let backtrace = backtrace::Backtrace::new();
                tracing::info!("--- BACKTRACE ---\n{:?}", backtrace);
                rfd::MessageDialog::new()
                    .set_title("Gizmo Engine Fatal Error")
                    .set_description(&error_msg)
                    .set_level(rfd::MessageLevel::Error)
                    .show();
            }
        }));
    }
}
