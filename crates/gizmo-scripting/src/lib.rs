//! Gizmo Scripting — Lua tabanlı oyun mantığı scriptleme sistemi
//!
//! ## Kullanım
//! ```rust,ignore
//! let mut script_engine = ScriptEngine::new().unwrap();
//! script_engine.load_script("scripts/player.lua").unwrap();
//!
//! // Her frame:
//! script_engine.update(&world, &input, dt).unwrap();
//! script_engine.flush_commands(&mut world);
//! ```
//!
//! ## Lua API
//! - `entity` — Position, rotation, scale, velocity okuma/yazma, spawn/destroy
//! - `input` — Tuş ve fare durumu sorgulama
//! - `physics` — Kuvvet/impuls uygulama
//! - `scene` — Sahne kaydetme/yükleme, entity arama
//! - `audio` — 2D/3D ses çalma
//! - `time` — Delta time, elapsed, FPS

pub mod api_audio;
pub mod api_entity;
pub mod api_input;
pub mod api_physics;
pub mod api_scene;
pub mod api_time;
pub mod api_vehicle;
pub mod commands;
pub mod engine;

pub use commands::{CommandQueue, ScriptCommand};
pub use engine::{Script, ScriptContext, ScriptEngine, ScriptResult};
