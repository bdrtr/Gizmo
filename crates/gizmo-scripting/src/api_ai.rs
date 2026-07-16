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

#[cfg(test)]
mod tests {
    use super::*;
    use mlua::Lua;

    /// ai.add_agent / set_target / clear_target doğru komutları kuyruğa yazmalı.
    #[test]
    fn ai_calls_push_expected_commands() {
        let lua = Lua::new();
        let cq = Arc::new(CommandQueue::new());
        register_ai_api(&lua, cq.clone()).unwrap();

        lua.load(
            r#"
            ai.add_agent(5)
            ai.set_target(5, 10.0, 0.0, -4.0)
            ai.clear_target(5)
            "#,
        )
        .exec()
        .unwrap();

        let cmds = cq.drain();
        assert_eq!(cmds.len(), 3);
        assert!(matches!(cmds[0], ScriptCommand::AddNavAgent(5)));
        match &cmds[1] {
            ScriptCommand::SetAiTarget(id, target) => {
                assert_eq!(*id, 5);
                assert_eq!(*target, Vec3::new(10.0, 0.0, -4.0));
            }
            other => panic!("beklenen SetAiTarget, gelen {other:?}"),
        }
        assert!(matches!(cmds[2], ScriptCommand::ClearAiTarget(5)));
    }
}
