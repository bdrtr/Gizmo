//! AI API — Lua'ya sunulan Yapay Zeka navigasyon fonksiyonları
//!
//! Lua scriptlerinden NPC ve ajanlara gitmeleri gereken hedefleri ayarlamayı sağlar.

use crate::commands::{CommandQueue, ScriptCommand};
use gizmo_math::Vec3;
use mlua::prelude::*;
use std::sync::Arc;

/// AI API fonksiyonlarını Lua'ya kaydeder
pub fn register_ai_api(lua: &Lua, command_queue: Arc<CommandQueue>) -> Result<(), LuaError> {
    let ai_table = lua.create_table()?;

    // === SET TARGET ===
    {
        let cq = command_queue.clone();
        ai_table.set(
            "set_target",
            lua.create_function(move |_, (id, x, y, z): (u32, f32, f32, f32)| {
                cq.push(ScriptCommand::SetAiTarget(id, Vec3::new(x, y, z)));
                Ok(())
            })?,
        )?;
    }

    // === CLEAR TARGET ===
    {
        let cq = command_queue.clone();
        ai_table.set(
            "clear_target",
            lua.create_function(move |_, id: u32| {
                cq.push(ScriptCommand::ClearAiTarget(id));
                Ok(())
            })?,
        )?;
    }

    // === ADD AGENT ===
    {
        let cq = command_queue.clone();
        ai_table.set(
            "add_agent",
            lua.create_function(move |_, id: u32| {
                cq.push(ScriptCommand::AddNavAgent(id));
                Ok(())
            })?,
        )?;
    }

    lua.globals().set("ai", ai_table)?;
    Ok(())
}
