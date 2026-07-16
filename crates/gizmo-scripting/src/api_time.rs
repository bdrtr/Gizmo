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

#[cfg(test)]
mod tests {
    use super::*;
    use mlua::Lua;

    fn eval_f32(lua: &Lua, expr: &str) -> f32 {
        lua.load(format!("return {expr}")).eval().unwrap()
    }

    /// Kayıt sonrası varsayılanlar: dt=0, elapsed=0, fps=60. Getter'lar bunları okumalı.
    #[test]
    fn defaults_after_register() {
        let lua = Lua::new();
        register_time_api(&lua).unwrap();
        assert_eq!(eval_f32(&lua, "time.dt()"), 0.0);
        assert_eq!(eval_f32(&lua, "time.elapsed()"), 0.0);
        assert_eq!(eval_f32(&lua, "time.fps()"), 60.0);
    }

    /// update_time_api getter'lara yansımalı ve önceki değerleri EZMELİ (overwrite).
    #[test]
    fn update_reflects_and_overwrites() {
        let lua = Lua::new();
        register_time_api(&lua).unwrap();

        update_time_api(&lua, 0.016, 1.5, 62.5).unwrap();
        assert!((eval_f32(&lua, "time.dt()") - 0.016).abs() < 1e-6);
        assert!((eval_f32(&lua, "time.elapsed()") - 1.5).abs() < 1e-6);
        assert!((eval_f32(&lua, "time.fps()") - 62.5).abs() < 1e-6);

        // İkinci güncelleme öncekini tamamen ezmeli.
        update_time_api(&lua, 0.033, 9.0, 30.0).unwrap();
        assert!((eval_f32(&lua, "time.dt()") - 0.033).abs() < 1e-6);
        assert!((eval_f32(&lua, "time.elapsed()") - 9.0).abs() < 1e-6);
        assert!((eval_f32(&lua, "time.fps()") - 30.0).abs() < 1e-6);
    }
}
