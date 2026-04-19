//! Vehicle API — Lua'ya sunulan araç kontrol fonksiyonları
//!
//! Lua scriptlerinden VehicleController bileşenine gaz, fren ve direksiyon komutları yolları sağlar.

use crate::commands::{CommandQueue, ScriptCommand};
use mlua::prelude::*;
use std::sync::Arc;

/// Vehicle API fonksiyonlarını Lua'ya kaydeder
pub fn register_vehicle_api(lua: &Lua, command_queue: Arc<CommandQueue>) -> Result<(), LuaError> {
    let vehicle_table = lua.create_table()?;

    // === ENGINE FORCE (GAZ) ===
    {
        let cq = command_queue.clone();
        vehicle_table.set(
            "set_engine_force",
            lua.create_function(move |_, (id, force): (u32, f32)| {
                cq.push(ScriptCommand::SetVehicleEngineForce(id, force));
                Ok(())
            })?,
        )?;
    }

    // === STEERING (DİREKSİYON) ===
    {
        let cq = command_queue.clone();
        vehicle_table.set(
            "set_steering",
            lua.create_function(move |_, (id, angle): (u32, f32)| {
                cq.push(ScriptCommand::SetVehicleSteering(id, angle));
                Ok(())
            })?,
        )?;
    }

    // === BRAKE (FREN) ===
    {
        let cq = command_queue.clone();
        vehicle_table.set(
            "set_brake",
            lua.create_function(move |_, (id, force): (u32, f32)| {
                cq.push(ScriptCommand::SetVehicleBrake(id, force));
                Ok(())
            })?,
        )?;
    }

    lua.globals().set("vehicle", vehicle_table)?;

    Ok(())
}
