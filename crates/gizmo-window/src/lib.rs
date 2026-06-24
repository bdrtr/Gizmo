//! A thin wrapper around [`winit`] for creating an OS window.
//!
//! This crate exposes a minimal [`AppWindow`] type and a [`run_window`]
//! helper that opens a window and runs a basic event loop. It is intended
//! for quick experiments and simple demos.
//!
//! For the full, rendering-integrated windowing path used by the engine,
//! see the `gizmo-app` crate instead.

use winit::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::{Window, WindowBuilder},
};

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
    /// # Panics
    ///
    /// Panics if the underlying platform window cannot be created.
    pub fn new(title: &str, width: u32, height: u32, event_loop: &EventLoop<()>) -> Self {
        let window = WindowBuilder::new()
            .with_title(title)
            .with_inner_size(winit::dpi::LogicalSize::new(width, height))
            .build(event_loop)
            .expect("HATA: Pencere oluşturulamadı!");

        Self { window }
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
/// # Panics
///
/// Panics if the event loop or the window cannot be created.
pub fn run_window(title: &str, width: u32, height: u32) {
    let event_loop = EventLoop::new().expect("Event Loop başlatılamadı");
    let _app = AppWindow::new(title, width, height, &event_loop);

    event_loop.set_control_flow(ControlFlow::Poll);

    tracing::info!(
        "{} {}x{} çözünürlüğünde başlatıldı. Ekranda bir pencere görmelisin!",
        title, width, height
    );

    let _ = event_loop.run(move |event, elwt| {
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
    });
}
