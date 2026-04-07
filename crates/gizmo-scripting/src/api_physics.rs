//! Physics API — Lua'ya sunulan fizik sistemi fonksiyonları
//!
//! Kuvvet uygulama, raycast ve yerçekimi ayarı gibi işlemler için kullanılır.

use mlua::prelude::*;
use std::sync::Arc;
use crate::commands::{CommandQueue, ScriptCommand};
use gizmo_math::Vec3;

/// Physics API fonksiyonlarını Lua'ya kaydeder
pub fn register_physics_api(lua: &Lua, command_queue: Arc<CommandQueue>) -> Result<(), LuaError> {
    let physics_table = lua.create_table()?;

    // === KUVVET UYGULA ===
    {
        let cq = command_queue.clone();
        physics_table.set("apply_force", lua.create_function(move |_, (id, fx, fy, fz): (u32, f32, f32, f32)| {
            cq.push(ScriptCommand::ApplyForce(id, Vec3::new(fx, fy, fz)));
            Ok(())
        })?)?;
    }

    // === İMPULS UYGULA ===
    {
        let cq = command_queue.clone();
        physics_table.set("apply_impulse", lua.create_function(move |_, (id, ix, iy, iz): (u32, f32, f32, f32)| {
            cq.push(ScriptCommand::ApplyImpulse(id, Vec3::new(ix, iy, iz)));
            Ok(())
        })?)?;
    }

    lua.globals().set("physics", physics_table)?;
    Ok(())
}
