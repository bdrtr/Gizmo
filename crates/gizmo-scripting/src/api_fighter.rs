//! Fighter API — Lua'ya sunulan dövüş sistemi fonksiyonları
//!
//! Lua scriptlerinden kombo sorgulama, hitstop/hitstun uygulama ve saldırı başlatma için kullanılır.

use crate::commands::{CommandQueue, ScriptCommand};
use gizmo_core::World;
use mlua::prelude::*;
use std::sync::Arc;

pub fn register_fighter_api(lua: &Lua, command_queue: Arc<CommandQueue>) -> Result<(), LuaError> {
    let fighter_table = lua.create_table()?;

    // Oku-Yaz tablosu
    fighter_table.set("_buffers", lua.create_table()?)?;
    fighter_table.set("_is_locked", lua.create_table()?)?;

    // === SET FIGHTER MOVE ===
    {
        let cq = command_queue.clone();
        fighter_table.set(
            "set_move",
            lua.create_function(
                move |_, (id, name, startup, active, recovery, damage): (u32, String, u32, u32, u32, f32)| {
                    cq.push(ScriptCommand::SetFighterMove {
                        id,
                        name,
                        startup,
                        active,
                        recovery,
                        damage,
                    });
                    Ok(())
                },
            )?,
        )?;
    }

    // === APPLY HITSTOP ===
    {
        let cq = command_queue.clone();
        fighter_table.set(
            "apply_hitstop",
            lua.create_function(move |_, (id, frames): (u32, u32)| {
                cq.push(ScriptCommand::ApplyHitstop(id, frames));
                Ok(())
            })?,
        )?;
    }

    // === APPLY HITSTUN ===
    {
        let cq = command_queue.clone();
        fighter_table.set(
            "apply_hitstun",
            lua.create_function(move |_, (id, frames): (u32, u32)| {
                cq.push(ScriptCommand::ApplyHitstun(id, frames));
                Ok(())
            })?,
        )?;
    }

    lua.globals().set("fighter", fighter_table)?;

    // Lua tarafında kombo kontrol eden yardımcı fonksiyon
    lua.load(
        r#"
        function fighter.is_locked(id)
            return fighter._is_locked[id] or false
        end

        function fighter.check_combo(id, combo, max_gap)
            local buffer = fighter._buffers[id]
            if not buffer then return false end

            local combo_idx = #combo
            if combo_idx == 0 then return false end

            local gap_counter = 0
            
            for i = 1, #buffer do
                local frame = buffer[i]
                local target_input = combo[combo_idx]

                if frame.just_pressed[target_input] then
                    combo_idx = combo_idx - 1
                    gap_counter = 0
                    if combo_idx == 0 then
                        return true
                    end
                elseif gap_counter > max_gap then
                    return false
                else
                    gap_counter = gap_counter + 1
                end
            end
            
            return false
        end
    "#,
    )
    .exec()?;

    Ok(())
}

pub fn update_fighter_read_api(lua: &Lua, world: &World) -> Result<(), LuaError> {
    let fighter_table: LuaTable = lua.globals().get("fighter")?;

    let buffers = lua.create_table()?;
    let is_locked = lua.create_table()?;

    let controllers = world.borrow::<gizmo_physics::components::fighter::FighterController>();
    for (eid, _) in controllers.iter() {
        if let Some(fighter) = controllers.get(eid) {
            is_locked.set(eid, fighter.is_locked())?;

            let frames_table = lua.create_table()?;
            for (i, frame) in fighter.input_buffer.frames.iter().enumerate() {
                let frame_table = lua.create_table()?;
                
                let jp_table = lua.create_table()?;
                for k in &frame.just_pressed {
                    jp_table.set(k.clone(), true)?;
                }
                
                let p_table = lua.create_table()?;
                for k in &frame.pressed {
                    p_table.set(k.clone(), true)?;
                }
                
                frame_table.set("just_pressed", jp_table)?;
                frame_table.set("pressed", p_table)?;
                
                frames_table.set(i + 1, frame_table)?;
            }
            buffers.set(eid, frames_table)?;
        }
    }

    fighter_table.set("_buffers", buffers)?;
    fighter_table.set("_is_locked", is_locked)?;

    Ok(())
}
