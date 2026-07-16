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

#[cfg(test)]
mod tests {
    use super::*;
    use mlua::Lua;

    /// audio.play / play_3d / stop doğru komutları (ad + 3B konum) kuyruğa yazmalı,
    /// FIFO sırayı ve argüman dönüşümünü koruyarak.
    #[test]
    fn audio_calls_push_expected_commands() {
        let lua = Lua::new();
        let cq = Arc::new(CommandQueue::new());
        register_audio_api(&lua, cq.clone()).unwrap();

        lua.load(
            r#"
            audio.play("jump")
            audio.play_3d("explosion", 1.0, 2.0, 3.0)
            audio.stop("music")
            "#,
        )
        .exec()
        .unwrap();

        let cmds = cq.drain();
        assert_eq!(cmds.len(), 3);
        match &cmds[0] {
            ScriptCommand::PlaySound(name) => assert_eq!(name, "jump"),
            other => panic!("beklenen PlaySound, gelen {other:?}"),
        }
        match &cmds[1] {
            ScriptCommand::PlaySound3D(name, pos) => {
                assert_eq!(name, "explosion");
                assert_eq!(*pos, Vec3::new(1.0, 2.0, 3.0));
            }
            other => panic!("beklenen PlaySound3D, gelen {other:?}"),
        }
        match &cmds[2] {
            ScriptCommand::StopSound(name) => assert_eq!(name, "music"),
            other => panic!("beklenen StopSound, gelen {other:?}"),
        }
    }
}
