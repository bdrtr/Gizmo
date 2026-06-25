//! # gizmo-core
//!
//! The core of the Gizmo game engine: a pure-Rust, archetype-based Entity
//! Component System (ECS) together with the scheduler that drives it.
//!
//! ## Overview
//!
//! - [`World`] is the central container holding all entities, components and
//!   resources.
//! - An [`Entity`] is a lightweight, generational handle. Data lives in
//!   [`Component`]s stored column-wise inside [`Archetype`]s for cache-friendly
//!   iteration.
//! - [`Query`] borrows components in bulk for reading or mutation, while
//!   [`Res`]/[`ResMut`] borrow global resources.
//! - [`System`]s are ordinary functions turned into systems via [`IntoSystem`];
//!   they are grouped into [`Phase`]s and run by a [`Schedule`] which resolves
//!   parallelism from the access patterns of each system.
//! - [`Commands`] queue deferred, structural changes (spawn/despawn, add/remove
//!   component) that are applied after a system finishes.
//!
//! ## Usage
//!
//! Most users pull in the common types through the prelude:
//!
//! ```no_run
//! use gizmo_core::prelude::*;
//! ```

// ──── Modüller (alfabetik) ────
pub mod archetype;
pub mod asset;
pub mod commands;
pub mod component;
pub mod cvar;
pub mod entity;
pub mod event;
pub mod hierarchy;
pub mod input;
pub mod observer;
pub mod logger;
pub mod pool;
pub mod profiler;
pub mod query;
pub mod registry;

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
pub use hierarchy::HierarchyExt;
pub use input::{ActionMap, Input, InputBinding};
pub use pool::{PoolManager, Pooled};
pub use profiler::FrameProfiler;
pub use query::{Changed, FetchComponent, Mut, Or, Query, With, Without, WorldQuery};
pub use registry::{ComponentRegistry, RegistryError};
pub use state::{in_state, State};

pub use system::{
    IntoSystem, IntoSystemConfig, Phase, Res, ResMut, Schedule, System, SystemConfig, SystemParam,
};
pub use time::{PhysicsTime, Time};
pub use window::WindowInfo;
pub use world::World;

pub type StorageView<'w, T> = crate::query::Query<'w, &'w T>;
pub type StorageViewMut<'w, T> = crate::query::Query<'w, crate::query::Mut<'w, T>>;

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
    #[cfg(feature = "reflect")]
    pub use bevy_reflect::Reflect;
}
pub mod state;

/// Re-export of `bevy_reflect`, available only with the `reflect` feature.
#[cfg(feature = "reflect")]
pub use bevy_reflect as reflect;
