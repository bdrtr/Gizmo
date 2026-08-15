//! API tables a script can read and cannot rewrite.
//!
//! # Why a proxy
//!
//! The engine hands Lua a table per subsystem — `input`, `entity`, `physics` — and every script
//! shares those objects. Sandboxing `_G` fixed one half of that: a script's globals are its own
//! now. It did not fix this half, because `input` is not a global write, it is a *field* write on
//! a shared object. Measured: `input.is_pressed = function() return true end` in one script, and
//! every other script sees the replacement for the rest of the session.
//!
//! A `__newindex` metamethod alone does not close it. `__newindex` fires only for keys the table
//! does **not** already have, and every key worth clobbering — `is_pressed`, `spawn`, `apply_force`
//! — is a key the table already has. Assigning to those writes straight through the metatable.
//!
//! So the global a script sees is an empty proxy. Empty means every read misses and goes through
//! `__index` to the real table, and every write — new key or not — reaches `__newindex`, which
//! raises. The real table lives in the Lua **registry**, which is reachable from Rust and not from
//! Lua at all: no global points at it, so a script cannot walk to it, and `__metatable` blocks
//! `getmetatable` from lifting it out of the proxy.
//!
//! Rust still writes the real table every frame with `raw_set`, which bypasses metamethods by
//! definition. That is the asymmetry the whole arrangement exists to create: the engine writes,
//! the scripts read.
//!
//! # What it does not stop
//!
//! A script can still shadow the name in its own environment (`input = something_else`) — that is
//! what `_G` isolation makes safe, since the shadow is private to that script. And a table the
//! engine hands *out* by value, rather than exposing as a global, is unaffected; this covers the
//! long-lived API surface, not every table that crosses the boundary.

use mlua::prelude::*;

/// Where the real table hides. Not a global: Lua has no way to name the registry.
fn registry_key(name: &str) -> String {
    format!("gizmo_api_raw_{name}")
}

/// Publish `name` as a read-only API table.
///
/// `build` fills the real table — Rust fields and, via `lua.load`, any Lua helper functions, which
/// is why it runs while `name` is still bound to the real table. The proxy replaces it afterwards;
/// from then on the helpers resolve `name` to the proxy and read through it, which is exactly what
/// a script does.
pub fn register_protected(
    lua: &Lua,
    name: &str,
    build: impl FnOnce(&LuaTable) -> Result<(), LuaError>,
) -> Result<(), LuaError> {
    let real = lua.create_table()?;

    // Visible under its real name while `build` runs: the Lua half of an API is written as
    // `function input.is_pressed(...)`, which is a field write and would hit the proxy's guard.
    lua.globals().set(name, real.clone())?;
    build(&real)?;

    lua.set_named_registry_value(&registry_key(name), real.clone())?;

    let proxy = lua.create_table()?;
    let meta = lua.create_table()?;
    meta.set("__index", real)?;
    let owner = name.to_string();
    meta.set(
        "__newindex",
        lua.create_function(move |_, (_, key, _): (LuaTable, LuaValue, LuaValue)| {
            let key = match &key {
                LuaValue::String(s) => s.to_str().unwrap_or("?").to_string(),
                other => format!("{other:?}"),
            };
            Err::<(), _>(LuaError::RuntimeError(format!(
                "`{owner}` is a read-only engine API: assigning to `{owner}.{key}` would change it \
                 for every script in the scene, not just this one"
            )))
        })?,
    )?;
    // Stops `getmetatable(input).__index` from handing the real table back, and `setmetatable`
    // from replacing the guard.
    meta.set("__metatable", false)?;
    proxy.set_metatable(Some(meta));

    lua.globals().set(name, proxy)?;
    Ok(())
}

/// The real table behind a protected API, for the engine's own per-frame writes.
///
/// Callers must use [`LuaTable::raw_set`] on it. A plain `set` would work today — the real table
/// has no metatable — but the point of fetching it here rather than reading the global is that the
/// write path and the read path are deliberately different, and `raw_set` says so at the call site.
pub fn raw<'lua>(lua: &'lua Lua, name: &str) -> Result<LuaTable<'lua>, LuaError> {
    lua.named_registry_value(&registry_key(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine_api(lua: &Lua) {
        register_protected(lua, "demo", |t| {
            t.set("value", 1)?;
            lua.load("function demo.helper() return demo.value end").exec()
        })
        .unwrap();
    }

    #[test]
    fn a_script_cannot_replace_an_api_function() {
        let lua = Lua::new();
        engine_api(&lua);
        let err = lua
            .load("demo.helper = function() return 99 end")
            .exec()
            .expect_err("overwriting an API function must fail");
        assert!(format!("{err}").contains("read-only"), "unexpected error: {err}");
        // …and the original still answers.
        let got: i64 = lua.load("return demo.helper()").eval().unwrap();
        assert_eq!(got, 1);
    }

    /// The case a bare `__newindex` misses: assigning to a key the table already has.
    #[test]
    fn an_existing_key_is_protected_too() {
        let lua = Lua::new();
        engine_api(&lua);
        assert!(lua.load("demo.value = 42").exec().is_err(), "existing keys must be protected");
        let got: i64 = lua.load("return demo.value").eval().unwrap();
        assert_eq!(got, 1, "the write must not have landed");
    }

    #[test]
    fn the_real_table_is_not_reachable_from_lua() {
        let lua = Lua::new();
        engine_api(&lua);
        // `getmetatable` is blocked, so the proxy cannot be unwrapped…
        let meta: LuaValue = lua.load("return getmetatable(demo)").eval().unwrap();
        assert_eq!(meta, LuaValue::Boolean(false), "the metatable must not be readable");
        // …and no global points at the real table.
        let found: bool = lua
            .load("for k, v in pairs(_G) do if v ~= demo and type(v) == 'table' and rawget(v, 'value') == 1 then return true end end return false")
            .eval()
            .unwrap();
        assert!(!found, "the real table is reachable through some global");
    }

    #[test]
    fn the_engine_still_writes_through_the_registry() {
        let lua = Lua::new();
        engine_api(&lua);
        raw(&lua, "demo").unwrap().raw_set("value", 7).unwrap();
        let got: i64 = lua.load("return demo.value").eval().unwrap();
        assert_eq!(got, 7, "a Rust-side raw_set must be visible through the proxy");
    }
}
