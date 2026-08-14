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

    // The name → key-code table comes from `gizmo_core::input::NAMED_KEYS`, not from a copy
    // written here. The copy this replaces held USB HID usage codes (`w = 17`, `space = 44`)
    // while the engine stores winit `KeyCode` discriminants, so EVERY entry was wrong — and
    // `down`/`right` held each other's codes, which meant a script reading the arrow keys moved
    // the player right when they pressed down. `gizmo-app` carries the test that proves the
    // table, because it is the crate that can see the enum.
    let key_map = lua.create_table()?;
    for (name, code) in gizmo_core::input::NAMED_KEYS {
        key_map.set(*name, *code)?;
    }
    input_table.set("_key_map", key_map)?;

    lua.globals().set("input", input_table)?;

    // Lua helper fonksiyonlarını tanımla
    lua.load(
        r#"
        function input.is_pressed(key_name)
            local code = input._key_map[string.lower(key_name)]
            if code and input._keys[code] then
                return true
            end
            return false
        end

        function input.is_just_pressed(key_name)
            local code = input._key_map[string.lower(key_name)]
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
    use gizmo_core::input::code_from_name;

    /// Marks exactly the named keys as held, by looking their codes up the same way the API does.
    ///
    /// The codes are NOT written out here. They were, and that is how these tests went on passing
    /// while every one of them described the wrong keyboard: the table under test and the table in
    /// the test were the same transcription of USB HID codes, so they agreed with each other and
    /// with nothing else.
    fn press(lua: &Lua, held: &[&str], just: &[&str]) {
        let entries = |names: &[&str]| {
            names
                .iter()
                .map(|n| format!("[{}] = true", code_from_name(n).expect("known key")))
                .collect::<Vec<_>>()
                .join(", ")
        };
        lua.load(format!(
            "input._keys = {{ {} }} input._just_keys = {{ {} }}",
            entries(held),
            entries(just)
        ))
        .exec()
        .unwrap();
    }

    /// Regression: 'n' and 'w' must not share a code, and each must answer on its own.
    #[test]
    fn n_and_w_keys_do_not_collide() {
        let lua = Lua::new();
        register_input_api(&lua).unwrap();

        press(&lua, &["w"], &[]);
        lua.load(
            r#"
            assert(input.is_pressed("w") == true, "w basili olmali")
            assert(input.is_pressed("n") == false, "n basili OLMAMALI (w ile cakisma)")
            "#,
        )
        .exec()
        .unwrap();

        press(&lua, &["n"], &[]);
        lua.load(
            r#"
            assert(input.is_pressed("n") == true, "n kendi keycode'unda basili olmali")
            assert(input.is_pressed("w") == false, "w basili OLMAMALI")
            "#,
        )
        .exec()
        .unwrap();
    }

    /// The arrow keys, because the old table had `down` and `right` holding each other's codes —
    /// a script reading them moved the player right when the player pressed down.
    ///
    /// This checks the *plumbing*: that a name reaches the slot it looked up. It cannot check that
    /// the numbers are right, because it gets them from the same table the API does — that is
    /// `gizmo-app`'s `key_convention` test, which compares them against the winit enum.
    #[test]
    fn the_arrow_keys_are_not_swapped() {
        let lua = Lua::new();
        register_input_api(&lua).unwrap();

        press(&lua, &["down"], &[]);
        lua.load(
            r#"
            assert(input.is_pressed("down") == true, "asagi basili")
            assert(input.is_pressed("right") == false, "sag basili DEGIL")
            assert(input.is_pressed("up") == false and input.is_pressed("left") == false)
            "#,
        )
        .exec()
        .unwrap();

        press(&lua, &["right"], &[]);
        lua.load(
            r#"
            assert(input.is_pressed("right") == true, "sag basili")
            assert(input.is_pressed("down") == false, "asagi basili DEGIL")
            "#,
        )
        .exec()
        .unwrap();
    }

    /// `is_just_pressed` reads `_just_keys` and is independent of `_keys`: a key held from an
    /// earlier frame is pressed but not just-pressed.
    #[test]
    fn is_just_pressed_is_independent_from_held() {
        let lua = Lua::new();
        register_input_api(&lua).unwrap();

        press(&lua, &["space"], &[]);
        lua.load(
            r#"
            assert(input.is_pressed("space") == true, "space surekli basili")
            assert(input.is_just_pressed("space") == false, "space bu frame basilmadi")
            "#,
        )
        .exec()
        .unwrap();

        press(&lua, &["space"], &["space"]);
        lua.load(r#"assert(input.is_just_pressed("space") == true, "space bu frame basildi")"#)
            .exec()
            .unwrap();
    }

    /// Names are case-insensitive, an unknown name is false rather than an error, and the digit
    /// row answers on its own codes.
    #[test]
    fn key_name_casing_unknown_and_digits() {
        let lua = Lua::new();
        register_input_api(&lua).unwrap();

        press(&lua, &["w", "1"], &[]);
        lua.load(
            r#"
            assert(input.is_pressed("W") == true, "buyuk harf W basili sayilmali")
            assert(input.is_pressed("w") == true, "kucuk harf w basili")
            assert(input.is_pressed("1") == true, "rakam tusu 1")
            assert(input.is_pressed("bilinmeyen_tus") == false, "haritada olmayan ad false")
            assert(input.is_pressed("2") == false, "basilmayan rakam false")
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
        input.on_key_pressed(code_from_name("w").unwrap());
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
