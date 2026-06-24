//! A thin wrapper around [`winit`] for creating an OS window.
//!
//! This crate exposes a minimal [`AppWindow`] type and a [`run_window`]
//! helper that opens a window and runs a basic event loop. It is intended
//! for quick experiments and simple demos.
//!
//! For the full, rendering-integrated windowing path used by the engine,
//! see the `gizmo-app` crate instead.

use winit::{
    error::{EventLoopError, OsError},
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::{Window, WindowBuilder},
};

/// Errors that can occur while creating or running an [`AppWindow`].
///
/// This enum wraps the underlying [`winit`] failures so callers can react
/// to windowing problems (e.g. a missing display in a headless/CI
/// environment) instead of panicking. The original error is preserved and
/// available through [`std::error::Error::source`].
#[derive(Debug)]
#[non_exhaustive]
pub enum WindowError {
    /// The platform failed to create the OS window.
    Os(OsError),
    /// The event loop could not be created or failed while running.
    EventLoop(EventLoopError),
}

impl std::fmt::Display for WindowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WindowError::Os(_) => write!(f, "failed to create the OS window"),
            WindowError::EventLoop(_) => write!(f, "failed to create or run the event loop"),
        }
    }
}

impl std::error::Error for WindowError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            WindowError::Os(e) => Some(e),
            WindowError::EventLoop(e) => Some(e),
        }
    }
}

impl From<OsError> for WindowError {
    fn from(e: OsError) -> Self {
        WindowError::Os(e)
    }
}

impl From<EventLoopError> for WindowError {
    fn from(e: EventLoopError) -> Self {
        WindowError::EventLoop(e)
    }
}

/// An owned OS window backed by [`winit`].
///
/// Wraps a single [`winit::window::Window`] resource. Because it owns a
/// native window handle, this type is intentionally neither `Clone` nor
/// `Copy`.
#[derive(Debug)]
pub struct AppWindow {
    window: Window,
}

impl AppWindow {
    /// Creates a new window with the given title and dimensions on the
    /// provided event loop.
    ///
    /// # Errors
    ///
    /// Returns [`WindowError::Os`] if the underlying platform window cannot
    /// be created.
    pub fn new(
        title: &str,
        width: u32,
        height: u32,
        event_loop: &EventLoop<()>,
    ) -> Result<Self, WindowError> {
        let window = WindowBuilder::new()
            .with_title(title)
            .with_inner_size(winit::dpi::LogicalSize::new(width, height))
            .build(event_loop)?;

        Ok(Self { window })
    }

    /// Returns a reference to the underlying [`winit::window::Window`].
    pub fn window(&self) -> &Window {
        &self.window
    }
}

/// Opens a window with the given title and size and runs a minimal event
/// loop until the window is closed.
///
/// This is a convenience helper for simple demos. It blocks the calling
/// thread for the lifetime of the window.
///
/// # Errors
///
/// Returns [`WindowError::EventLoop`] if the event loop cannot be created
/// or fails while running, and [`WindowError::Os`] if the window cannot be
/// created.
pub fn run_window(title: &str, width: u32, height: u32) -> Result<(), WindowError> {
    let event_loop = EventLoop::new()?;
    let _app = AppWindow::new(title, width, height, &event_loop)?;

    event_loop.set_control_flow(ControlFlow::Poll);

    tracing::info!(
        "{} {}x{} çözünürlüğünde başlatıldı. Ekranda bir pencere görmelisin!",
        title, width, height
    );

    event_loop.run(move |event, elwt| {
        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => elwt.exit(),
            Event::AboutToWait => {
                // Her frame'de yapılacak işler...
            }
            _ => (),
        }
    })?;

    Ok(())
}
