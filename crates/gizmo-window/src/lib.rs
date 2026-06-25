//! A thin wrapper around [`winit`] for creating an OS window.
//!
//! This crate exposes a minimal [`AppWindow`] type and a [`run_window`]
//! helper that opens a window and runs a basic event loop. It is intended
//! for quick experiments and simple demos.
//!
//! For the full, rendering-integrated windowing path used by the engine,
//! see the `gizmo-app` crate instead.

use winit::{
    application::ApplicationHandler,
    error::{EventLoopError, OsError},
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::{Window, WindowId},
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
    /// Creates a new window on the given **active** event loop.
    ///
    /// Since winit 0.30, OS windows can only be created once the event loop is
    /// active (i.e. from inside [`ApplicationHandler::resumed`]), so this takes
    /// an [`ActiveEventLoop`] rather than the `EventLoop` itself.
    ///
    /// # Errors
    ///
    /// Returns [`WindowError::Os`] if the underlying platform window cannot
    /// be created.
    pub fn new(
        title: &str,
        width: u32,
        height: u32,
        event_loop: &ActiveEventLoop,
    ) -> Result<Self, WindowError> {
        let attributes = Window::default_attributes()
            .with_title(title)
            .with_inner_size(winit::dpi::LogicalSize::new(width, height));
        let window = event_loop.create_window(attributes)?;

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
    event_loop.set_control_flow(ControlFlow::Poll);

    let mut app = WindowApp {
        title: title.to_string(),
        width,
        height,
        window: None,
        deferred_error: None,
    };
    event_loop.run_app(&mut app)?;

    // Surface a window-creation failure that happened inside `resumed`.
    if let Some(err) = app.deferred_error {
        return Err(err);
    }
    Ok(())
}

/// Minimal [`ApplicationHandler`] backing [`run_window`].
///
/// Since winit 0.30 the event loop is driven through this trait rather than a
/// closure: the window is created in [`resumed`](ApplicationHandler::resumed)
/// (the first point at which an [`ActiveEventLoop`] is available) and events
/// are dispatched through [`window_event`](ApplicationHandler::window_event).
struct WindowApp {
    title: String,
    width: u32,
    height: u32,
    window: Option<AppWindow>,
    /// A window-creation error captured in `resumed`, surfaced by `run_window`
    /// after the loop exits (the trait methods cannot return `Result`).
    deferred_error: Option<WindowError>,
}

impl ApplicationHandler for WindowApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        match AppWindow::new(&self.title, self.width, self.height, event_loop) {
            Ok(window) => {
                tracing::info!(
                    "{} {}x{} çözünürlüğünde başlatıldı. Ekranda bir pencere görmelisin!",
                    self.title,
                    self.width,
                    self.height
                );
                self.window = Some(window);
            }
            Err(err) => {
                self.deferred_error = Some(err);
                event_loop.exit();
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        if let WindowEvent::CloseRequested = event {
            event_loop.exit();
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        // Her frame'de yapılacak işler...
    }
}
