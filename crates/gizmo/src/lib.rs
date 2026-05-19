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
