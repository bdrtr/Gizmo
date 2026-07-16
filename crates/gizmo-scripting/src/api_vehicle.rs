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

#[cfg(test)]
mod tests {
    use super::*;
    use mlua::Lua;

    /// vehicle.set_engine_force / set_steering / set_brake doğru komutları,
    /// negatif değerleri (geri vites / sola direksiyon) koruyarak kuyruğa yazmalı.
    #[test]
    fn vehicle_calls_push_expected_commands() {
        let lua = Lua::new();
        let cq = Arc::new(CommandQueue::new());
        register_vehicle_api(&lua, cq.clone()).unwrap();

        lua.load(
            r#"
            vehicle.set_engine_force(3, 1500.0)
            vehicle.set_steering(3, -0.35)
            vehicle.set_brake(3, 800.0)
            "#,
        )
        .exec()
        .unwrap();

        let cmds = cq.drain();
        assert_eq!(cmds.len(), 3);
        match cmds[0] {
            ScriptCommand::SetVehicleEngineForce(id, f) => {
                assert_eq!(id, 3);
                assert!((f - 1500.0).abs() < 1e-3);
            }
            ref other => panic!("beklenen SetVehicleEngineForce, gelen {other:?}"),
        }
        match cmds[1] {
            ScriptCommand::SetVehicleSteering(id, a) => {
                assert_eq!(id, 3);
                assert!((a - (-0.35)).abs() < 1e-6, "negatif direksiyon korunmalı");
            }
            ref other => panic!("beklenen SetVehicleSteering, gelen {other:?}"),
        }
        match cmds[2] {
            ScriptCommand::SetVehicleBrake(id, f) => {
                assert_eq!(id, 3);
                assert!((f - 800.0).abs() < 1e-3);
            }
            ref other => panic!("beklenen SetVehicleBrake, gelen {other:?}"),
        }
    }
}
