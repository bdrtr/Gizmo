#![deny(clippy::undocumented_unsafe_blocks)]
#![warn(missing_docs)]
//! (`undocumented_unsafe_blocks` is a RATCHET: this crate carries no `unsafe` block without a
//! `// SAFETY:` line stating why it is sound, and the lint keeps it that way. Every crate in the
//! workspace except `gizmo-core` is at zero and denies it; `gizmo-core`'s ECS internals are the
//! measured remainder — see docs/ENGINE.md.)
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
pub mod api_table;
pub mod api_audio;
pub mod api_entity;
pub mod api_fighter;
pub mod api_input;
pub mod api_physics;
pub mod api_scene;
pub mod api_time;
pub mod api_vehicle;
pub mod commands;

/// The Lua VM, the `Script` component and the per-entity update path.
pub mod engine;

pub use commands::{CommandQueue, ScriptCommand};

pub use engine::{Script, ScriptEngine, ScriptValue};

/// Registers the scripting layer's serializable scene components (currently
/// [`Script`]) into a scene `ComponentRegistry`.
///
/// Call this from the layer that wires both scenes and scripting together (the
/// app / editor / facade) so that `gizmo-scene` itself stays free of any
/// dependency on `gizmo-scripting`. Without this call a scene round-trips fine,
/// it simply won't (de)serialize `Script` components.
pub fn register_script_components(reg: &mut gizmo_core::registry::ComponentRegistry) {
    reg.register_serializable::<Script>("Script")
        .expect("built-in component 'Script' registration must not conflict");
}

// THIS CRATE DOES NOT EXIST ON wasm32, and the `cfg(target_arch = "wasm32")` arm that used to sit
// here was never compiled by anything. Measured 2026-08-18, not assumed:
// `cargo check -p gizmo-scripting --target wasm32-unknown-unknown` dies in mlua-sys's build
// script — "don't know how to build Lua for wasm32-unknown-unknown" — so the one configuration
// that arm existed for is a configuration in which the crate cannot be built at all. Its
// consumers already know that: `gizmo-app` and `gizmo-editor` both list this crate under
// `[target.'cfg(not(target_arch = "wasm32"))'.dependencies]`, and `cargo tree -p demo-web
// --target wasm32-unknown-unknown` shows zero occurrences of it.
//
// The proof it never compiled is in what it had drifted into: `dummy_engine::update_entity` took
// three arguments where the real one takes four, under a comment promising that identical
// signatures let "calling code compile unchanged on both targets".
//
// The pattern that IS load-bearing is per-item, in the consumer: `gizmo-editor` keeps two bodies
// for `draw_script_section` and cfg's the script inspector out. That one is linted on wasm, which
// is the difference.
//
// TRIGGER for reviving a stub here: a browser build that wants `Script` components to survive a
// scene round-trip, or an mlua backend that builds for wasm32. Either way the first step is
// target-gating `mlua` in this crate's manifest, so the crate can compile at all on the target
// the stub is for.
