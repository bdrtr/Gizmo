//! Input API — Lua'ya sunulan girdi sorgulama fonksiyonları
//!
//! Lua scriptlerinden tuş ve fare durumunu sorgulamak için kullanılır.
//! Read-only API'dir, komut kuyruğuna yazmaz.

use mlua::prelude::*;
use gizmo_core::input::Input;

/// Input API fonksiyonlarını Lua'ya kaydeder
pub fn register_input_api(lua: &Lua) -> Result<(), LuaError> {
    let input_table = lua.create_table()?;
    
    // Placeholder fonksiyonlar - her frame update_input_api ile güncellenir
    input_table.set("_keys", lua.create_table()?)?;
    input_table.set("_just_keys", lua.create_table()?)?;
    input_table.set("_mouse_x", 0.0f32)?;
    input_table.set("_mouse_y", 0.0f32)?;
    input_table.set("_mouse_dx", 0.0f32)?;
    input_table.set("_mouse_dy", 0.0f32)?;
    input_table.set("_mouse_left", false)?;
    input_table.set("_mouse_right", false)?;
    input_table.set("_mouse_middle", false)?;
    
    lua.globals().set("input", input_table)?;
    
    // Lua helper fonksiyonlarını tanımla
    lua.load(r#"
        -- Tuş adından KeyCode'a eşleme tablosu
        local key_map = {
            w = 17, a = 4, s = 22, d = 7,
            q = 20, e = 8, r = 21, f = 9,
            z = 29, x = 27, c = 6, v = 25,
            space = 44, lshift = 225, rshift = 229,
            lctrl = 224, rctrl = 228,
            tab = 43, escape = 41, enter = 40,
            up = 82, down = 81, left = 80, right = 79,
            ["1"] = 30, ["2"] = 31, ["3"] = 32, ["4"] = 33,
            ["5"] = 34, ["6"] = 35, ["7"] = 36, ["8"] = 37,
            ["9"] = 38, ["0"] = 39,
            i = 12, j = 13, k = 14, l = 15,
            b = 5, n = 17, m = 16,
        }
        
        function input.is_pressed(key_name)
            local code = key_map[string.lower(key_name)]
            if code and input._keys[code] then
                return true
            end
            return false
        end
        
        function input.is_just_pressed(key_name)
            local code = key_map[string.lower(key_name)]
            if code and input._just_keys[code] then
                return true
            end
            return false
        end
        
        function input.mouse_position()
            return { x = input._mouse_x, y = input._mouse_y }
        end
        
        function input.mouse_delta()
            return { x = input._mouse_dx, y = input._mouse_dy }
        end
        
        function input.is_mouse_pressed(button)
            if button == "left" then return input._mouse_left
            elseif button == "right" then return input._mouse_right
            elseif button == "middle" then return input._mouse_middle
            end
            return false
        end
    "#).exec()?;
    
    Ok(())
}

/// Her frame Input durumunu Lua'ya aktarır
pub fn update_input_api(lua: &Lua, input: &Input) -> Result<(), LuaError> {
    let input_table: LuaTable = lua.globals().get("input")?;
    
    // Basılı tuşları Lua table'ına aktar
    let keys = lua.create_table()?;
    let just_keys = lua.create_table()?;
    
    // Yaygın tuş kodlarını kontrol et (winit KeyCode enum değerleri)
    for code in 0..256u32 {
        if input.is_key_pressed(code) {
            keys.set(code, true)?;
        }
        if input.is_key_just_pressed(code) {
            just_keys.set(code, true)?;
        }
    }
    
    input_table.set("_keys", keys)?;
    input_table.set("_just_keys", just_keys)?;
    
    let (mx, my) = input.mouse_position();
    input_table.set("_mouse_x", mx)?;
    input_table.set("_mouse_y", my)?;
    
    let (dx, dy) = input.mouse_delta();
    input_table.set("_mouse_dx", dx)?;
    input_table.set("_mouse_dy", dy)?;
    
    input_table.set("_mouse_left", input.is_mouse_button_pressed(gizmo_core::input::mouse::LEFT))?;
    input_table.set("_mouse_right", input.is_mouse_button_pressed(gizmo_core::input::mouse::RIGHT))?;
    input_table.set("_mouse_middle", input.is_mouse_button_pressed(gizmo_core::input::mouse::MIDDLE))?;
    
    Ok(())
}
