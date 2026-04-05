pub mod prelude;

// === Motor Alt Sistemleri ===
pub use gizmo_core as core;
pub use gizmo_math as math;
pub use gizmo_renderer as renderer;
pub use gizmo_window as window;
pub use gizmo_physics as physics;
pub use gizmo_app as app;

// === Opsiyonel Modüller ===
#[cfg(feature = "audio")]
pub use gizmo_audio as audio;

#[cfg(feature = "editor")]
pub use gizmo_editor as editor;

#[cfg(feature = "scripting")]
pub use gizmo_scripting as scripting;

// === 3. Parti Re-Export (Kullanıcının ayrıca eklemesine gerek kalmasın) ===
pub use winit;
pub use wgpu;
pub use egui;
pub use bytemuck;
