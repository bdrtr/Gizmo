// ──── Modüller (alfabetik) ────
pub mod component;
pub mod entity;
pub mod event;
pub mod input;
pub mod logger;
pub mod query;
pub mod registry;
pub mod system;
pub mod time;
pub mod window;
pub mod world;

// ──── Explicit re-exports ────
pub use component::{Component, ComponentStorage, EntityName, IsHidden, PrefabRequest, SparseSet};
pub use entity::Entity;
pub use event::Events;
pub use input::{ActionMap, Input, InputBinding};
pub use query::{FetchComponent, Query, WorldQuery};
pub use registry::ComponentRegistry;
pub use system::{IntoSystem, Res, ResMut, Schedule, System, SystemParam};
pub use time::Time;
pub use window::WindowInfo;
pub use world::World;

// ──── Prelude ────
/// Tek bir `use gizmo_core::prelude::*;` ile tüm temel tiplere erişim.
pub mod prelude {
    pub use super::{
        Component, ComponentStorage, Entity, EntityName, Events, Input, ActionMap, InputBinding,
        IntoSystem, IsHidden, PrefabRequest, Query, Res, ResMut, Schedule, SparseSet, System,
        SystemParam, Time, WindowInfo, World,
    };
    pub use super::input::mouse;
}
