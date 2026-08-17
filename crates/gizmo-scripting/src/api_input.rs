//! The input API — the input-querying functions exposed to Lua.
//!
//! Used from Lua scripts to ask about key, mouse and gamepad state. It is a read-only API and
//! never writes to the command queue.

use gizmo_core::input::Input;
use mlua::prelude::*;

/// Registers the input API functions with Lua.
pub fn register_input_api(lua: &Lua) -> Result<(), LuaError> {
    crate::api_table::register_protected(lua, "input", |input_table| {

    // Placeholder fonksiyonlar - her frame update_input_api ile güncellenir
    input_table.raw_set("_keys", lua.create_table()?)?;
    input_table.raw_set("_just_keys", lua.create_table()?)?;
    input_table.raw_set("_mouse_x", 0.0f32)?;
    input_table.raw_set("_mouse_y", 0.0f32)?;
    input_table.raw_set("_mouse_dx", 0.0f32)?;
    input_table.raw_set("_mouse_dy", 0.0f32)?;
    input_table.raw_set("_mouse_left", false)?;
    input_table.raw_set("_mouse_right", false)?;
    input_table.raw_set("_mouse_middle", false)?;

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
    input_table.raw_set("_key_map", key_map)?;

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

        -- Gamepad. With nothing plugged in `input._pad` is nil and all of these answer
        -- false/0, so a script never has to branch on "is there a pad".
        function input.gamepad_connected()
            return input._pad ~= nil
        end

        function input.gamepad_name()
            return input._pad and input._pad.name or nil
        end

        function input.gamepad_pressed(button)
            return input._pad ~= nil and input._pad.held[button] == true
        end

        function input.gamepad_just_pressed(button)
            return input._pad ~= nil and input._pad.just[button] == true
        end

        function input.gamepad_just_released(button)
            return input._pad ~= nil and input._pad.released[button] == true
        end

        -- The DEADZONED value: `left_stick_x/y` and `right_stick_x/y` for the sticks,
        -- `left_trigger`/`right_trigger` for the triggers.
        function input.gamepad_axis(axis)
            if input._pad == nil then return 0.0 end
            return input._pad.axes[axis] or 0.0
        end
    "#,
        )
        .exec()
    })
}

/// Mirrors the Input state into Lua, every frame.
#[tracing::instrument(skip_all, name = "script_input_read")]
pub fn update_input_api(lua: &Lua, input: &Input) -> Result<(), LuaError> {
    // The real table, not the global: the global is a read-only proxy so a script cannot rewrite
    // the API (see `api_table`), and the engine's own per-frame writes go behind it.
    let input_table = crate::api_table::raw(lua, "input")?;

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

    input_table.raw_set("_keys", keys)?;
    input_table.raw_set("_just_keys", just_keys)?;

    let (mx, my) = input.mouse_position();
    input_table.raw_set("_mouse_x", mx)?;
    input_table.raw_set("_mouse_y", my)?;

    let (dx, dy) = input.mouse_delta();
    input_table.raw_set("_mouse_dx", dx)?;
    input_table.raw_set("_mouse_dy", dy)?;

    input_table.raw_set(
        "_mouse_left",
        input.is_mouse_button_pressed(gizmo_core::input::mouse::LEFT),
    )?;
    input_table.raw_set(
        "_mouse_right",
        input.is_mouse_button_pressed(gizmo_core::input::mouse::RIGHT),
    )?;
    input_table.raw_set(
        "_mouse_middle",
        input.is_mouse_button_pressed(gizmo_core::input::mouse::MIDDLE),
    )?;

    // Gamepad. `nil` when nothing is plugged in — the helpers above lean on that, so a script
    // never has to ask twice, and the table is rebuilt each frame rather than mutated so a
    // disconnect cannot leave a stale button behind.
    match input.gamepad() {
        None => input_table.raw_set("_pad", mlua::Value::Nil)?,
        Some(pad) => {
            let held = lua.create_table()?;
            let just = lua.create_table()?;
            let released = lua.create_table()?;
            // Names come from `gizmo_core`'s table, not from a transcription here. The key API
            // above carries the scar from doing it the other way.
            for (name, button) in gizmo_core::input::NAMED_GAMEPAD_BUTTONS {
                if pad.is_pressed(*button) {
                    held.raw_set(*name, true)?;
                }
                if pad.is_just_pressed(*button) {
                    just.raw_set(*name, true)?;
                }
                if pad.is_just_released(*button) {
                    released.raw_set(*name, true)?;
                }
            }

            // Axes are the DEADZONED reading, because a script asking for the stick wants what
            // the player meant, not the sensor. `axis()` is still reachable from Rust for the
            // rare case that wants the raw value.
            let axes = lua.create_table()?;
            let (lx, ly) = pad.left_stick();
            let (rx, ry) = pad.right_stick();
            axes.raw_set("left_stick_x", lx)?;
            axes.raw_set("left_stick_y", ly)?;
            axes.raw_set("right_stick_x", rx)?;
            axes.raw_set("right_stick_y", ry)?;
            axes.raw_set("left_trigger", pad.left_trigger())?;
            axes.raw_set("right_trigger", pad.right_trigger())?;

            let table = lua.create_table()?;
            table.raw_set("name", pad.name())?;
            table.raw_set("connected", pad.is_connected())?;
            table.raw_set("held", held)?;
            table.raw_set("just", just)?;
            table.raw_set("released", released)?;
            table.raw_set("axes", axes)?;
            input_table.raw_set("_pad", table)?;
        }
    }

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
    ///
    /// Written through the registry rather than from Lua, because the API table is read-only to
    /// scripts now — and that is the honest path anyway: this is where real input arrives from.
    fn press(lua: &Lua, held: &[&str], just: &[&str]) {
        let table = |names: &[&str]| {
            let t = lua.create_table().unwrap();
            for n in names {
                t.raw_set(code_from_name(n).expect("known key"), true).unwrap();
            }
            t
        };
        let input_table = crate::api_table::raw(lua, "input").unwrap();
        input_table.raw_set("_keys", table(held)).unwrap();
        input_table.raw_set("_just_keys", table(just)).unwrap();
    }

    /// The gamepad half goes through the real `update_input_api`, not through a hand-built
    /// table: what it has to prove is that an `Input` a game would actually hold turns into the
    /// values a script reads — including the two that are easy to get wrong, the deadzoned
    /// stick and the trigger that is a button *and* an axis.
    #[test]
    fn a_script_reads_the_pad_the_engine_holds() {
        use gizmo_core::input::{GamepadAxis, GamepadButton, GamepadId, Input};

        let lua = Lua::new();
        register_input_api(&lua).unwrap();

        // Nothing plugged in: every query must answer, and answer "no".
        let mut input = Input::new();
        update_input_api(&lua, &input).unwrap();
        lua.load(
            r#"
            assert(input.gamepad_connected() == false, "kol yokken bagli gorunmemeli")
            assert(input.gamepad_pressed("south") == false, "kol yokken tus basili degil")
            assert(input.gamepad_axis("left_stick_x") == 0.0, "kol yokken eksen sifir")
            assert(input.gamepad_name() == nil, "kol yokken isim yok")
            "#,
        )
        .exec()
        .unwrap();

        let pad = GamepadId::new(0);
        input.on_gamepad_connected(pad, "Test Pad");
        input.on_gamepad_button_pressed(pad, GamepadButton::South);
        input.on_gamepad_axis(pad, GamepadAxis::LeftStickX, 1.0);
        input.on_gamepad_axis(pad, GamepadAxis::RightTrigger, 1.0);
        // Inside the deadzone: the script must see a centred stick, not a drifting one.
        input.on_gamepad_axis(pad, GamepadAxis::RightStickY, 0.05);
        update_input_api(&lua, &input).unwrap();

        lua.load(
            r#"
            assert(input.gamepad_connected() == true, "kol bagli olmali")
            assert(input.gamepad_name() == "Test Pad", "kolun adi gelmeli")
            assert(input.gamepad_pressed("south") == true, "south basili")
            assert(input.gamepad_just_pressed("south") == true, "south bu frame basildi")
            assert(input.gamepad_pressed("north") == false, "north basili DEGIL")
            assert(input.gamepad_axis("left_stick_x") > 0.99, "sol cubuk tam sagda")
            assert(input.gamepad_axis("right_trigger") > 0.99, "sag tetik tam cekili")
            assert(input.gamepad_axis("right_stick_y") == 0.0, "olu bolge icindeki kayma sifirlanmali")
            assert(input.gamepad_axis("no_such_axis") == 0.0, "bilinmeyen eksen sifir doner")
            "#,
        )
        .exec()
        .unwrap();

        // And the pad going away must not leave a stale table behind.
        input.begin_frame();
        input.on_gamepad_disconnected(pad);
        input.begin_frame();
        update_input_api(&lua, &input).unwrap();
        lua.load(
            r#"
            assert(input.gamepad_connected() == false, "kol cikinca bagli kalmamali")
            assert(input.gamepad_pressed("south") == false, "cikan kolun tusu basili kalmamali")
            "#,
        )
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

    /// The mouse helpers: position/delta must return tables, and is_mouse_pressed must answer
    /// correctly for left/right/middle and for an unknown button.
    #[test]
    fn mouse_helpers_read_snapshot() {
        let lua = Lua::new();
        register_input_api(&lua).unwrap();
        let t = crate::api_table::raw(&lua, "input").unwrap();
        for (k, v) in [("_mouse_x", 120.0f32), ("_mouse_y", 45.0), ("_mouse_dx", -3.0), ("_mouse_dy", 7.0)] {
            t.raw_set(k, v).unwrap();
        }
        for (k, v) in [("_mouse_left", true), ("_mouse_right", false), ("_mouse_middle", true)] {
            t.raw_set(k, v).unwrap();
        }
        lua.load(
            r#"
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

    /// update_input_api must mirror a real Input state into Lua correctly: the held key, the
    /// mouse position/delta and the mouse buttons.
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
