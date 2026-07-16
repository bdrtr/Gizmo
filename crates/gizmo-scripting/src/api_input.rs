//! Input API — Lua'ya sunulan girdi sorgulama fonksiyonları
//!
//! Lua scriptlerinden tuş ve fare durumunu sorgulamak için kullanılır.
//! Read-only API'dir, komut kuyruğuna yazmaz.

use gizmo_core::input::Input;
use mlua::prelude::*;

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
    lua.load(
        r#"
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
            b = 5, n = 18, m = 16,
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
    "#,
    )
    .exec()?;

    Ok(())
}

/// Her frame Input durumunu Lua'ya aktarır
#[tracing::instrument(skip_all, name = "script_input_read")]
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

    input_table.set(
        "_mouse_left",
        input.is_mouse_button_pressed(gizmo_core::input::mouse::LEFT),
    )?;
    input_table.set(
        "_mouse_right",
        input.is_mouse_button_pressed(gizmo_core::input::mouse::RIGHT),
    )?;
    input_table.set(
        "_mouse_middle",
        input.is_mouse_button_pressed(gizmo_core::input::mouse::MIDDLE),
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression: 'n' ve 'w' aynı keycode'a (17) eşlenmemeli.
    /// Sadece 'w' basılıyken input.is_pressed("n") false dönmeli.
    #[test]
    fn n_and_w_keys_do_not_collide() {
        let lua = Lua::new();
        register_input_api(&lua).unwrap();

        // Yalnızca keycode 17 (w) basılı olarak işaretle.
        lua.load(
            r#"
            input._keys = { [17] = true }
            assert(input.is_pressed("w") == true, "w basılı olmalı")
            assert(input.is_pressed("n") == false, "n basılı OLMAMALI (w ile çakışma)")
            "#,
        )
        .exec()
        .unwrap();

        // Ayrıca 'n' kendi keycode'unda çalışmalı.
        lua.load(
            r#"
            input._keys = { [18] = true }
            assert(input.is_pressed("n") == true, "n kendi keycode'unda basılı olmalı")
            assert(input.is_pressed("w") == false, "w basılı OLMAMALI")
            "#,
        )
        .exec()
        .unwrap();
    }

    /// is_just_pressed _just_keys tablosundan okumalı ve _keys'ten BAĞIMSIZ olmalı:
    /// sürekli basılı (_keys) ama bu frame basılmamış (_just_keys yok) → is_just_pressed false.
    #[test]
    fn is_just_pressed_is_independent_from_held() {
        let lua = Lua::new();
        register_input_api(&lua).unwrap();
        lua.load(
            r#"
            input._keys = { [44] = true }        -- space sürekli basılı
            input._just_keys = {}                -- ama bu frame basılmadı
            assert(input.is_pressed("space") == true, "space sürekli basılı")
            assert(input.is_just_pressed("space") == false, "space bu frame basılmadı")

            input._just_keys = { [44] = true }   -- şimdi bu frame basıldı
            assert(input.is_just_pressed("space") == true, "space bu frame basıldı")
            "#,
        )
        .exec()
        .unwrap();
    }

    /// Tuş adları büyük/küçük harf duyarsız olmalı; bilinmeyen ad false dönmeli;
    /// rakam tuşları ("1".."0") kendi keycode'larına eşlenmeli.
    #[test]
    fn key_name_casing_unknown_and_digits() {
        let lua = Lua::new();
        register_input_api(&lua).unwrap();
        lua.load(
            r#"
            input._keys = { [17] = true, [30] = true }  -- w ve "1"
            assert(input.is_pressed("W") == true, "büyük harf W basılı sayılmalı")
            assert(input.is_pressed("w") == true, "küçük harf w basılı")
            assert(input.is_pressed("1") == true, "rakam tuşu 1 (keycode 30)")
            assert(input.is_pressed("bilinmeyen_tus") == false, "haritada olmayan ad false")
            assert(input.is_pressed("2") == false, "basılmayan rakam false")
            "#,
        )
        .exec()
        .unwrap();
    }

    /// Fare yardımcıları: pozisyon/delta tablo döndürmeli; is_mouse_pressed sol/sağ/orta
    /// ve bilinmeyen düğme için doğru sonuç vermeli.
    #[test]
    fn mouse_helpers_read_snapshot() {
        let lua = Lua::new();
        register_input_api(&lua).unwrap();
        lua.load(
            r#"
            input._mouse_x = 120.0
            input._mouse_y = 45.0
            input._mouse_dx = -3.0
            input._mouse_dy = 7.0
            input._mouse_left = true
            input._mouse_right = false
            input._mouse_middle = true

            local p = input.mouse_position()
            assert(p.x == 120.0 and p.y == 45.0, "pozisyon")
            local d = input.mouse_delta()
            assert(d.x == -3.0 and d.y == 7.0, "delta")
            assert(input.is_mouse_pressed("left") == true, "sol basılı")
            assert(input.is_mouse_pressed("right") == false, "sağ basılı değil")
            assert(input.is_mouse_pressed("middle") == true, "orta basılı")
            assert(input.is_mouse_pressed("side") == false, "bilinmeyen düğme false")
            "#,
        )
        .exec()
        .unwrap();
    }

    /// update_input_api gerçek bir Input durumunu Lua'ya doğru aktarmalı:
    /// basılı tuş, fare konumu/deltası ve fare düğmeleri.
    #[test]
    fn update_input_api_mirrors_real_input() {
        use gizmo_core::input::{mouse, Input};

        let lua = Lua::new();
        register_input_api(&lua).unwrap();

        let mut input = Input::default();
        input.on_key_pressed(17); // 'w'
        input.set_mouse_position(200.0, 100.0);
        input.on_mouse_delta(5.0, -2.0);
        input.on_mouse_button_pressed(mouse::RIGHT);

        update_input_api(&lua, &input).unwrap();

        lua.load(
            r#"
            assert(input.is_pressed("w") == true, "w World'den aktarılmalı")
            assert(input.is_just_pressed("w") == true, "w bu frame basıldı")
            local p = input.mouse_position()
            assert(p.x == 200.0 and p.y == 100.0, "fare konumu aktarılmalı")
            local d = input.mouse_delta()
            assert(math.abs(d.x - 5.0) < 1e-5 and math.abs(d.y + 2.0) < 1e-5, "fare delta")
            assert(input.is_mouse_pressed("right") == true, "sağ tık aktarılmalı")
            assert(input.is_mouse_pressed("left") == false, "sol tık basılı değil")
            "#,
        )
        .exec()
        .unwrap();
    }
}
