#![deny(clippy::undocumented_unsafe_blocks)]
//! (`undocumented_unsafe_blocks` is a RATCHET, and this crate was the last one to earn it: the
//! ECS's archetype / query / world internals hold most of the workspace's `unsafe`, and every
//! block there now states the invariant it relies on — the `Component: Send + Sync` bound behind
//! the storage `Send`/`Sync` impls, the caller-side aliasing contract behind the `UnsafeCell`
//! column access, and the row-liveness argument behind the raw pointers. Whole workspace: zero.)
#![warn(missing_docs)]
//! (`missing_docs` is a RATCHET, not a suggestion. The CI lint gate runs with `-D warnings`,
//! so every public item in this crate must carry a doc comment or the build fails. This crate
//! is Stage A — the dependency-light core that goes to 1.x first — and its documented surface
//! is part of that promise. Do not silence this with `#[allow]`; write the doc.)

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
// Sequential fallback for rayon on wasm (no OS threads); native uses rayon.
#[cfg(target_arch = "wasm32")]
mod parallel_compat;
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

pub mod relationship;
pub mod system;
pub mod time;
pub mod timer;
pub mod window;
pub mod world;

// ──── Explicit re-exports ────
pub use archetype::{Archetype, ComponentInfo, EntityLocation};
pub use commands::{CommandQueue, Commands, EntityCommands};
pub use component::{Bundle, BundleExt, Component, EntityName, IsHidden, IsDeleted, PrefabRequest};
pub use cvar::{CVarRegistry, CVarValue, DevConsoleState};
pub use entity::Entity;
pub use relationship::{OnTargetDespawn, Relationship, RelationshipTarget};
pub use event::{EventMutator, EventReader, EventWriter, Events};
pub use hierarchy::HierarchyExt;
pub use input::{ActionMap, Input, InputBinding};
pub use pool::{PoolManager, Pooled};
pub use profiler::FrameProfiler;
pub use query::{
    Changed, FetchComponent, Mut, Or, Query, QueryData, QueryFilter, QuerySingleError, ReadOnlyQuery, With, Without,
    WorldQuery,
};
pub use registry::{ComponentRegistry, RegistryError};
pub use state::{in_state, DespawnOnExit, State};

pub use system::{
    IntoSystem, IntoSystemConfig, Local, Phase, Res, ResMut, Schedule, System, SystemConfig,
    SystemParam,
};
pub use time::{PhysicsTime, Time};
pub use timer::{Timer, TimerMode};
pub use window::WindowInfo;
pub use world::World;


// ──── Prelude ────
/// Access to all the basic types with a single `use gizmo_core::prelude::*;`.
pub mod prelude {
    pub use super::input::mouse;
    pub use super::input::{
        AxisDirection, Gamepad, GamepadAxis, GamepadButton, GamepadDeadzone, GamepadId, Gamepads, HoldRepeat,
    };
    pub use super::{
        ActionMap, Bundle, Changed, CommandQueue, Commands, Component, Entity, EntityName,
        EventMutator, EventReader, EventWriter, Events, FrameProfiler, Input, InputBinding, IntoSystem,
        IntoSystemConfig, IsHidden, IsDeleted, Local, Mut, Phase, PhysicsTime, PoolManager, Pooled,
        PrefabRequest,
        Query, QueryData, QueryFilter, ReadOnlyQuery, Res, ResMut, Schedule, System,
        SystemConfig, SystemParam, Time, Timer, TimerMode, WindowInfo, World,
    };
    #[cfg(feature = "reflect")]
    pub use bevy_reflect::Reflect;
}
pub mod state;

/// Re-export of `bevy_reflect`, available only with the `reflect` feature.
#[cfg(feature = "reflect")]
pub use bevy_reflect as reflect;
