//! Gizmo Scripting — a Lua-based game-logic scripting layer for the Gizmo engine.
//!
//! Scripts run inside a sandboxed [`mlua`] Lua 5.4 VM. Because Lua callbacks
//! cannot borrow and mutate the ECS `World` directly, they enqueue changes as
//! [`ScriptCommand`]s into a [`CommandQueue`]; the [`ScriptEngine`] later drains
//! and applies those commands at a controlled point in the frame.
//!
//! ## Usage
//! ```rust,ignore
//! let mut script_engine = ScriptEngine::new().unwrap();
//! script_engine.load_script("scripts/player.lua").unwrap();
//!
//! // Each frame:
//! script_engine.update(&world, &input, dt).unwrap();
//! script_engine.flush_commands(&mut world);
//! ```
//!
//! ## Lua API surface
//! - `entity` — read/write position, rotation, scale, velocity; spawn/destroy
//! - `input` — query key and mouse state
//! - `physics` — apply forces and impulses
//! - `scene` — save/load scenes, look up entities
//! - `audio` — play 2D/3D sounds
//! - `time` — delta time, elapsed time, FPS

pub mod api_ai;
pub mod api_audio;
pub mod api_entity;
pub mod api_fighter;
pub mod api_input;
pub mod api_physics;
pub mod api_scene;
pub mod api_time;
pub mod api_vehicle;
pub mod commands;

#[cfg(target_arch = "wasm32")]
pub mod dummy_engine;
pub mod engine;

pub use commands::{CommandQueue, ScriptCommand};

pub use engine::{Script, ScriptContext, ScriptEngine, ScriptResult};

#[cfg(target_arch = "wasm32")]
pub use dummy_engine::{
    Script as DummyScript, ScriptContext as DummyContext, ScriptEngine as DummyEngine,
    ScriptResult as DummyResult,
};
