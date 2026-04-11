pub mod color;
pub mod default_systems;
pub mod prelude;
pub mod spawner;

// === Motor Alt Sistemleri ===
pub use default_systems as default_plugins;
pub use gizmo_ai as ai;
pub use gizmo_app as app;
pub use gizmo_core as core;
pub use gizmo_math as math;
pub use gizmo_physics as physics;
pub use gizmo_renderer as renderer;
pub use gizmo_window as window;

// === Opsiyonel Modüller ===
#[cfg(feature = "audio")]
pub use gizmo_audio as audio;

#[cfg(feature = "editor")]
pub use gizmo_editor as editor;

#[cfg(feature = "scripting")]
pub use gizmo_scripting as scripting;

#[cfg(feature = "scene")]
pub use gizmo_scene as scene;

// === 3. Parti Re-Export (Kullanıcının ayrıca eklemesine gerek kalmasın) ===
pub use gizmo_core::gizmo_log;

pub use bytemuck;
pub use egui;
pub use wgpu;
pub use winit;
