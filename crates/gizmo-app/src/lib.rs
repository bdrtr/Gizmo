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

#[cfg(feature = "egui")]
pub mod dev_console;
/// Generic immediate-mode overlay UI runtime (egui integration).
#[cfg(feature = "egui")]
pub mod egui_ctx;
/// Per-frame editor integration (scene/game RTT + scene save/load), kept out of
/// the windowed event loop.
#[cfg(feature = "editor")]
pub mod editor_runtime;
/// High-level gameplay physics systems (vehicle / character controllers) wired
/// into the app schedule. Requires the `physics` feature.
#[cfg(feature = "physics")]
pub mod gameplay;
pub mod plugin;

pub use plugin::Plugin;

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

#[cfg(feature = "window")]
pub mod windowed;
#[cfg(feature = "window")]
pub use windowed::*;

#[cfg(not(feature = "window"))]
pub mod headless;
#[cfg(not(feature = "window"))]
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
