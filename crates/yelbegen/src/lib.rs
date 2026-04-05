pub mod prelude;

// === Motor Alt Sistemleri ===
pub use yelbegen_core as core;
pub use yelbegen_math as math;
pub use yelbegen_renderer as renderer;
pub use yelbegen_window as window;
pub use yelbegen_physics as physics;
pub use yelbegen_app as app;

// === Opsiyonel Modüller ===
#[cfg(feature = "audio")]
pub use yelbegen_audio as audio;

#[cfg(feature = "editor")]
pub use yelbegen_editor as editor;

#[cfg(feature = "scripting")]
pub use yelbegen_scripting as scripting;

// === 3. Parti Re-Export (Kullanıcının ayrıca eklemesine gerek kalmasın) ===
pub use winit;
pub use wgpu;
pub use egui;
pub use bytemuck;
