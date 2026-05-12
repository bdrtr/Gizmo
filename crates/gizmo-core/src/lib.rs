// ──── Modüller (alfabetik) ────
pub mod archetype;
pub mod asset;
pub mod commands;
pub mod component;
pub mod cvar;
pub mod entity;
pub mod event;
pub mod input;
pub mod logger;
pub mod pool;
pub mod profiler;
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
pub use component::{Bundle, BundleExt, Component, EntityName, IsHidden, IsDeleted, PrefabRequest};
pub use cvar::{CVarRegistry, CVarValue, DevConsoleState};
pub use entity::Entity;
pub use event::{EventReader, EventWriter, Events};
pub use input::{ActionMap, Input, InputBinding};
pub use pool::{PoolManager, Pooled};
pub use profiler::FrameProfiler;
pub use query::{Changed, FetchComponent, Mut, Or, Query, With, Without, WorldQuery};
pub use registry::ComponentRegistry;
pub use state::{in_state, State};
pub use storage::{StorageView, StorageViewMut};
pub use system::{
    IntoSystem, IntoSystemConfig, Phase, Res, ResMut, Schedule, System, SystemConfig, SystemParam,
};
pub use time::{PhysicsTime, Time};
pub use window::WindowInfo;
pub use world::World;

// ──── Prelude ────
/// Tek bir `use gizmo_core::prelude::*;` ile tüm temel tiplere erişim.
pub mod prelude {
    pub use super::input::mouse;
    pub use super::{
        ActionMap, Bundle, Changed, CommandQueue, Commands, Component, Entity, EntityName,
        EventReader, EventWriter, Events, FrameProfiler, Input, InputBinding, IntoSystem,
        IntoSystemConfig, IsHidden, IsDeleted, Mut, Phase, PhysicsTime, PoolManager, Pooled, PrefabRequest,
        Query, Res, ResMut, Schedule, StorageView, StorageViewMut, System, SystemConfig,
        SystemParam, Time, WindowInfo, World,
    };
}
pub mod state;
