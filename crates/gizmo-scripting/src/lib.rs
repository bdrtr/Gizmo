//! Gizmo Scripting — a Lua-based game-logic scripting layer for the Gizmo engine.
//!
//! Scripts run inside a sandboxed [`mlua`] Lua 5.4 VM. Because Lua callbacks
//! cannot borrow and mutate the ECS `World` directly, they enqueue changes as
//! [`ScriptCommand`]s into a [`CommandQueue`]; the [`ScriptEngine`] later drains
//! and applies those commands at a controlled point in the frame.
//!
//! ## Usage
//! ```
//! use gizmo_core::input::Input;
//! use gizmo_core::World;
//! use gizmo_math::Vec3;
//! use gizmo_physics_core::Transform;
//! use gizmo_scripting::ScriptEngine;
//!
//! let mut world = World::new();
//! let player = world.spawn();
//! world.add_component(player, Transform::new(Vec3::ZERO));
//! # // Stand-in for `scripts/player.lua`, written where the doc test can read it:
//! # //   function on_update(ctx) entity.set_position(<player>, 1, 2, 3) end
//! # let script = std::env::temp_dir().join(format!("gizmo_doc_player_{}.lua", std::process::id()));
//! # std::fs::write(
//! #     &script,
//! #     format!("function on_update(ctx)\n  entity.set_position({}, 1.0, 2.0, 3.0)\nend\n", player.id()),
//! # )
//! # .unwrap();
//! # let script_path = script.to_string_lossy().into_owned();
//!
//! let mut script_engine = ScriptEngine::new().unwrap();
//! script_engine.load_script(&script_path).unwrap(); // e.g. "scripts/player.lua"
//!
//! // Each frame:
//! let (input, dt) = (Input::default(), 1.0 / 60.0);
//! script_engine.update(&world, &input, dt).unwrap(); // runs `on_update`; commands are queued
//! script_engine.flush_commands(&mut world, dt);      // the queue is applied to the World here
//! # std::fs::remove_file(&script).ok();
//!
//! // Lua never touched the World itself — the command it enqueued did, at flush time.
//! let pos = world.borrow::<Transform>().get(player.id()).unwrap().position;
//! assert_eq!(pos, Vec3::new(1.0, 2.0, 3.0));
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

/// Registers the scripting layer's serializable scene components (currently
/// [`Script`]) into a scene `ComponentRegistry`.
///
/// Call this from the layer that wires both scenes and scripting together (the
/// app / editor / facade) so that `gizmo-scene` itself stays free of any
/// dependency on `gizmo-scripting`. Without this call a scene round-trips fine,
/// it simply won't (de)serialize `Script` components.
#[cfg(not(target_arch = "wasm32"))]
pub fn register_script_components(reg: &mut gizmo_core::registry::ComponentRegistry) {
    reg.register_serializable::<Script>("Script")
        .expect("built-in component 'Script' registration must not conflict");
}

/// No-op on `wasm32`, where the Lua-backed scripting engine is unavailable.
#[cfg(target_arch = "wasm32")]
pub fn register_script_components(_reg: &mut gizmo_core::registry::ComponentRegistry) {}

#[cfg(target_arch = "wasm32")]
pub use dummy_engine::{
    Script as DummyScript, ScriptContext as DummyContext, ScriptEngine as DummyEngine,
    ScriptResult as DummyResult,
};
