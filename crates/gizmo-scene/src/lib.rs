//! Gizmo Scene — Sahne serileştirme ve yönetim sistemi
//!
//! Sahne dosyalarını JSON olarak kaydetme/yükleme yeteneği sağlar.
//! Editor, Lua scripting ve runtime tarafından kullanılır.

pub mod physics_parser;
pub mod registry;
pub mod scene;
pub mod snapshot;

pub use registry::SceneRegistry;
pub use scene::{EntityData, MaterialData, SceneData};
pub use snapshot::SceneSnapshot;
