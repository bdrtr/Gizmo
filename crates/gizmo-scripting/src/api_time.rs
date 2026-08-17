//! The time API — the timing functions exposed to Lua.
//!
//! Gives access to delta time, total elapsed time and FPS.

use mlua::prelude::*;

/// Registers the time API functions with Lua.
pub fn register_time_api(lua: &Lua) -> Result<(), LuaError> {
    crate::api_table::register_protected(lua, "time", |time_table| {

    time_table.raw_set("_dt", 0.0f32)?;
    time_table.raw_set("_elapsed", 0.0f32)?;
    time_table.raw_set("_fps", 60.0f32)?;


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
    })
}

/// Refreshes the timing data every frame.
pub fn update_time_api(lua: &Lua, dt: f32, elapsed: f32, fps: f32) -> Result<(), LuaError> {
    // The real table, not the global: the global is a read-only proxy so a script cannot
    // rewrite the API (see `api_table`), and the engine's per-frame writes go behind it.
    let time_table = crate::api_table::raw(lua, "time")?;
    time_table.raw_set("_dt", dt)?;
    time_table.raw_set("_elapsed", elapsed)?;
    time_table.raw_set("_fps", fps)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mlua::Lua;

    fn eval_f32(lua: &Lua, expr: &str) -> f32 {
        lua.load(format!("return {expr}")).eval().unwrap()
    }

    /// The defaults after registration: dt=0, elapsed=0, fps=60. The getters must read them.
    #[test]
    fn defaults_after_register() {
        let lua = Lua::new();
        register_time_api(&lua).unwrap();
        assert_eq!(eval_f32(&lua, "time.dt()"), 0.0);
        assert_eq!(eval_f32(&lua, "time.elapsed()"), 0.0);
        assert_eq!(eval_f32(&lua, "time.fps()"), 60.0);
    }

    /// update_time_api must show up in the getters and must OVERWRITE the previous values.
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
