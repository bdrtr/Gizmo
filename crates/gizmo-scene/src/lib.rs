//! Gizmo Scene — Sahne serileştirme ve yönetim sistemi
//!
//! Sahne dosyalarını JSON olarak kaydetme/yükleme yeteneği sağlar.
//! Editor, Lua scripting ve runtime tarafından kullanılır.

pub mod scene;
pub mod registry;

pub use scene::{EntityData, MaterialData, SceneData};
pub use registry::SceneRegistry;
