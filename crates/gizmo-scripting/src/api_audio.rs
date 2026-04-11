//! Audio API — Lua'ya sunulan ses yönetim fonksiyonları

use crate::commands::{CommandQueue, ScriptCommand};
use gizmo_math::Vec3;
use mlua::prelude::*;
use std::sync::Arc;

/// Audio API fonksiyonlarını Lua'ya kaydeder
pub fn register_audio_api(lua: &Lua, command_queue: Arc<CommandQueue>) -> Result<(), LuaError> {
    let audio_table = lua.create_table()?;

    // === SES ÇALMA ===
    {
        let cq = command_queue.clone();
        audio_table.set(
            "play",
            lua.create_function(move |_, sound_name: String| {
                cq.push(ScriptCommand::PlaySound(sound_name));
                Ok(())
            })?,
        )?;
    }

    // === 3D SES ÇALMA ===
    {
        let cq = command_queue.clone();
        audio_table.set(
            "play_3d",
            lua.create_function(move |_, (sound_name, x, y, z): (String, f32, f32, f32)| {
                cq.push(ScriptCommand::PlaySound3D(sound_name, Vec3::new(x, y, z)));
                Ok(())
            })?,
        )?;
    }

    // === SES DURDURMA ===
    {
        let cq = command_queue.clone();
        audio_table.set(
            "stop",
            lua.create_function(move |_, sound_name: String| {
                cq.push(ScriptCommand::StopSound(sound_name));
                Ok(())
            })?,
        )?;
    }

    lua.globals().set("audio", audio_table)?;
    Ok(())
}
