// ──── Modüller (alfabetik) ────
pub mod archetype;
pub mod commands;
pub mod component;
pub mod entity;
pub mod event;
pub mod input;
pub mod logger;
pub mod query;
pub mod registry;
pub mod storage;
pub mod system;
pub mod time;
pub mod window;
pub mod world;

// ──── Explicit re-exports ────
pub use archetype::{Archetype, ComponentInfo, EntityLocation};
pub use commands::{CommandQueue, Commands, EntityCommands};
pub use component::{Component, EntityName, IsHidden, PrefabRequest};
pub use entity::Entity;
pub use event::Events;
pub use input::{ActionMap, Input, InputBinding};
pub use query::{Changed, FetchComponent, Mut, Query, WorldQuery};
pub use registry::ComponentRegistry;
pub use storage::{StorageView, StorageViewMut};
pub use system::{IntoSystem, Res, ResMut, Schedule, System, SystemParam};
pub use time::Time;
pub use window::WindowInfo;
pub use world::World;

// ──── Prelude ────
/// Tek bir `use gizmo_core::prelude::*;` ile tüm temel tiplere erişim.
pub mod prelude {
    pub use super::input::mouse;
    pub use super::{
        ActionMap, Changed, CommandQueue, Commands, Component, Entity, EntityName, Events, Input,
        InputBinding, IntoSystem, IsHidden, Mut, PrefabRequest, Query, Res, ResMut, Schedule,
        StorageView, StorageViewMut, System, SystemParam, Time, WindowInfo, World,
    };
}
