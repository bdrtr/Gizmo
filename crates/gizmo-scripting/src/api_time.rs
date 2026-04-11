//! Time API — Lua'ya sunulan zaman fonksiyonları
//!
//! Delta time, toplam süre ve FPS bilgilerine erişim sağlar.

use mlua::prelude::*;

/// Time API fonksiyonlarını Lua'ya kaydeder
pub fn register_time_api(lua: &Lua) -> Result<(), LuaError> {
    let time_table = lua.create_table()?;

    time_table.set("_dt", 0.0f32)?;
    time_table.set("_elapsed", 0.0f32)?;
    time_table.set("_fps", 60.0f32)?;

    lua.globals().set("time", time_table)?;

    // Lua helper fonksiyonları
    lua.load(
        r#"
        function time.dt()
            return time._dt
        end
        
        function time.elapsed()
            return time._elapsed
        end
        
        function time.fps()
            return time._fps
        end
    "#,
    )
    .exec()?;

    Ok(())
}

/// Her frame zaman verisini günceller
pub fn update_time_api(lua: &Lua, dt: f32, elapsed: f32, fps: f32) -> Result<(), LuaError> {
    let time_table: LuaTable = lua.globals().get("time")?;
    time_table.set("_dt", dt)?;
    time_table.set("_elapsed", elapsed)?;
    time_table.set("_fps", fps)?;
    Ok(())
}
