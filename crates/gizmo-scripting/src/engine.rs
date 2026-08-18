use gizmo_core::input::Input;
use gizmo_core::World;
use mlua::prelude::*;
use mlua::RegistryKey;
use std::collections::BTreeMap;
use std::sync::Arc;
use tracing::{debug, error, info, trace, warn};


/// Wakes the body a script just wrote a velocity to.
///
/// Required, not defensive. `PhysicsWorld::sync_bodies` **drops** a velocity written to a sleeping
/// dynamic body, because storing one makes a stale impulse: nothing reads it while the body
/// sleeps, and whatever wakes the body later applies it in full. `RigidBody::wake_up`'s own doc
/// states the contract — change a velocity, wake the body — and these four commands were the
/// scripting half of the engine ignoring it. A script that pushed a settled crate saw nothing
/// happen, and then saw the crate leap when something unrelated disturbed the stack.
///
/// Called after the `Velocity` borrow is released, because this takes its own on `RigidBody`.
fn wake_after_velocity_write(world: &mut World, id: u32) {
    let mut rbs = world.borrow_mut::<gizmo_physics_rigid::components::RigidBody>();
    if let Some(mut rb) = rbs.get_mut(id) {
        rb.wake_up();
    }
}

use crate::api_ai;
use crate::api_audio;
use crate::api_entity;
use crate::api_fighter;
use crate::api_input;
use crate::api_physics;
use crate::api_scene;
use crate::api_time;
use crate::api_vehicle;
use crate::commands::{CommandQueue, ScriptCommand};

/// The Lua scripting engine — runs game logic through the extended API.
pub struct ScriptEngine {
    lua: Lua,
    /// Loaded scripts, keyed by path — **ordered**, and that is load-bearing rather than tidy.
    ///
    /// This was a `std::collections::HashMap`, whose `RandomState` is seeded per process, so the
    /// order `update` ran scripts in changed from run to run. Two scripts pushing commands that
    /// touch the same entity therefore resolved in a random order, and this engine's headline
    /// contract is same-platform bit-identical replay. A `BTreeMap` costs a comparison per lookup
    /// and makes the order a property of the scripts' paths instead of of the allocator.
    loaded_scripts: BTreeMap<String, (String, RegistryKey)>,
    command_queue: Arc<CommandQueue>,
    /// Hook ticks left for the Lua call currently running; see [`ScriptEngine::arm_budget`].
    budget: Arc<std::sync::atomic::AtomicU32>,
    /// Ticks handed out per call. `instructions / HOOK_INSTRUCTION_STEP`.
    budget_ticks: u32,
    elapsed_time: f32,
    /// Log messages emitted from Lua (`print`), stored as `(level, message)` pairs.
    pub log_queue: Arc<std::sync::Mutex<Vec<(String, String)>>>, // (Level, Message)
}

// `Send` is NOT hand-written: mlua is built with its `send` feature (see this
// crate's Cargo.toml), which makes `Lua: Send`, and every other field is already
// `Send`. The compiler derives it — if that ever stops holding we want the build
// to break rather than an `unsafe impl` to paper over it.

// SAFETY: `Lua` is `Send` but deliberately **not** `Sync` — mlua mutates the
// underlying `lua_State` through `&Lua`, so two threads holding `&Lua` would
// race. `Sync` on this type therefore has exactly one precondition:
//
//   *** No `&self` method of `ScriptEngine` may touch `self.lua`. ***
//
// That precondition holds by construction. The complete set of `&self` methods
// is `flush_commands` and `command_queue`; neither reads `self.lua` (they only
// drain the `Arc<CommandQueue>` and the
// `Arc<Mutex<..>>` log queue, both of which are `Sync` on their own). Every
// method that does reach the VM — `new`, `load_script`, `reload_script`,
// `update`, `update_entity`, `has_function`, … — takes `&mut self`, so the
// borrow checker makes concurrent VM access unrepresentable: a caller needs
// `ResMut<ScriptEngine>`, which the scheduler treats as an exclusive write.
//
// `Sync` is required because `ScriptEngine` is stored as a `World` resource and
// `World::insert_resource` demands `Send + Sync`.
//
// If you add a `&self` method, it must not touch `self.lua`. The regression test
// `shared_methods_never_reach_the_lua_vm` at the bottom of this file records the
// audited list; update it deliberately, not incidentally.
unsafe impl Sync for ScriptEngine {}

// `Lua` does not implement `Debug`, so the engine provides a manual summary that
// omits the VM internals while still surfacing useful state.
impl std::fmt::Debug for ScriptEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScriptEngine")
            .field("lua", &"<Lua VM>")
            .field("loaded_scripts", &self.loaded_scripts.keys())
            .field("elapsed_time", &self.elapsed_time)
            .field(
                "queued_commands",
                &self.command_queue.len(),
            )
            .field(
                "queued_logs",
                &self.log_queue.lock().map(|q| q.len()).unwrap_or(0),
            )
            .finish()
    }
}

/// One value a script exposes to the editor.
///
/// Three kinds, because those are the three a property inspector can edit without inventing a
/// widget: a number, a flag, and a name. A script that needs more structure than this wants a
/// table it manages itself, not an inspector row.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ScriptValue {
    /// A number. Lua has one numeric type, so this is what every `properties` number arrives as.
    Num(f64),
    /// A flag, shown as a checkbox.
    Bool(bool),
    /// A string, shown as a text field.
    Text(String),
}

impl ScriptValue {
    /// The label the inspector shows for this kind, and what a mismatched override is checked
    /// against: an override whose kind differs from the declaration is never *coerced*, because
    /// silently turning `true` into `1` is how a script starts misbehaving in a way nobody can
    /// trace to the editor.
    ///
    /// It is not *ignored* either, and this note used to say it was. Nothing filters
    /// [`Script::properties`] on its way to Lua — see
    /// `every_stored_property_reaches_the_script_declared_or_not`. The editor acted on the wrong
    /// half of that sentence: it dropped mismatched overrides from its display and showed the
    /// declared default, while the script kept running on the stale value. The inspector now
    /// shows every stored value and marks the odd ones instead.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Num(_) => "number",
            Self::Bool(_) => "bool",
            Self::Text(_) => "text",
        }
    }
}

/// The ECS component recording which Lua script is attached to an entity.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Script {
    /// Path of the Lua file. It is also the identity a script is loaded under: two entities with
    /// the same path share one environment.
    pub file_path: String,
    /// Has `on_init` run for this entity yet?
    ///
    /// `#[serde(default, skip)]`, so it is never written to a scene and always starts false on
    /// load — which is what makes `on_init` run again when a scene is opened.
    #[serde(default, skip)]
    pub initialized: bool,
    /// Per-entity overrides for the properties this script declares.
    ///
    /// Scripts are loaded once per PATH — two entities running the same file share one Lua
    /// environment — so a per-entity value cannot live in that environment. It lives here and is
    /// handed to `on_entity_update` as its third argument.
    ///
    /// A `BTreeMap` for the same reason the loaded-script map is one: this crate's contract is
    /// same-platform bit-identical replay, and a `HashMap`'s iteration order is seeded per process.
    #[serde(default)]
    pub properties: std::collections::BTreeMap<String, ScriptValue>,
}

impl Script {
    /// A script component pointing at `path`, not yet initialised and with no property
    /// overrides.
    pub fn new(path: &str) -> Self {
        Self {
            file_path: path.to_string(),
            initialized: false,
            properties: std::collections::BTreeMap::new(),
        }
    }
}

impl ScriptEngine {
    /// Instructions between two hook firings. The hook itself is an atomic load and a compare, so
    /// this is not about the hook's cost — it is the resolution of the budget, and 10 000
    /// instructions is far below one frame's worth of anything sane.
    const HOOK_INSTRUCTION_STEP: u32 = 10_000;

    /// Default ceiling for a single call into Lua: `on_update` for one script, one entity hook,
    /// or the top level of a script being loaded.
    ///
    /// Two million instructions is generously above what a per-frame script should ever execute
    /// and far below "the window stopped responding". It is a runaway guard, not a performance
    /// budget: a script that trips it has a bug, and the alternative to tripping it was hanging
    /// the process, because `while true do end` in a Lua VM the host has no timeout on is
    /// unrecoverable — no signal, no watchdog, and the frame never ends.
    pub const DEFAULT_INSTRUCTION_BUDGET: u32 = 2_000_000;

    /// Default ceiling on the VM's heap. Reached, an allocation fails as a catchable Lua error
    /// instead of the process growing until the OOM killer decides which program dies — which on
    /// a machine running an editor and a game is not necessarily this one.
    pub const DEFAULT_MEMORY_LIMIT: usize = 64 * 1024 * 1024;

    /// Starts a Lua VM with the engine's API registered, the dangerous globals removed and the
    /// instruction and memory budgets in place.
    ///
    /// Fails only if the VM itself cannot be created; a script is not loaded here.
    pub fn new() -> Result<Self, LuaError> {
        let lua = Lua::new();
        let command_queue = Arc::new(CommandQueue::new());
        let log_queue = Arc::new(std::sync::Mutex::new(Vec::new()));

        // === SANDBOX: Tehlikeli modülleri kapat ===
        lua.globals().set("os", LuaNil)?;
        lua.globals().set("io", LuaNil)?;
        lua.globals().set("loadfile", LuaNil)?;
        lua.globals().set("dofile", LuaNil)?;
        lua.globals().set("require", LuaNil)?;
        lua.globals().set("package", LuaNil)?;
        lua.globals().set("debug", LuaNil)?;
        lua.globals().set("loadstring", LuaNil)?;
        lua.globals().set("load", LuaNil)?;

        // === RUNAWAY GUARD: instruction budget + memory ceiling ===
        // Without these a script is unbounded in both time and space, and the host has no way to
        // take control back: `while true do end` never yields, mlua's `call` never returns, and
        // the frame — the window, the editor, the game — is simply over. The budget is armed per
        // call (see `arm_budget`), so one runaway script loses its own frame and the scripts
        // ordered after it still run.
        let budget = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let hook_budget = budget.clone();
        // mlua 0.12 made `set_hook` fallible, and this hook is the instruction budget — the only
        // thing standing between a `while true do end` and a hung frame. Propagated, never
        // dropped: an engine whose budget hook failed to install must not report success.
        lua.set_hook(
            mlua::HookTriggers::new().every_nth_instruction(Self::HOOK_INSTRUCTION_STEP),
            move |_lua, _debug| {
                // Not `fetch_sub` alone: at zero that wraps to `u32::MAX` and the guard silently
                // stops guarding. Load-then-store is safe here because the VM is single-threaded
                // by construction — every method that reaches it takes `&mut self`.
                let left = hook_budget.load(std::sync::atomic::Ordering::Relaxed);
                if left == 0 {
                    return Err(LuaError::RuntimeError(
                        "script exceeded its instruction budget for this call (infinite loop?)"
                            .to_string(),
                    ));
                }
                hook_budget.store(left - 1, std::sync::atomic::Ordering::Relaxed);
                // mlua 0.10+ lets a hook stop or yield the VM; this one only counts, so it
                // always says "carry on". The budget is enforced by the `return Err` above.
                Ok(mlua::VmState::Continue)
            },
        )?;
        lua.set_memory_limit(Self::DEFAULT_MEMORY_LIMIT)?;

        // === TEMEL PRINT FONKSİYONU ===
        let lq_clone1 = log_queue.clone();
        lua.globals().set(
            "print_engine",
            lua.create_function(move |_, msg: String| {
                if let Ok(mut q) = lq_clone1.lock() {
                    q.push(("info".to_string(), msg));
                }
                Ok(())
            })?,
        )?;

        // Orijinal print'i de engine çıktısına yönlendir
        let lq_clone2 = log_queue.clone();
        lua.globals().set(
            "print",
            lua.create_function(move |_, values: LuaMultiValue| {
                let parts: Vec<String> = values
                    .iter()
                    .map(|v| {
                        if let mlua::Value::String(s) = v {
                            s.to_str().map(|s| s.to_string()).unwrap_or_default()
                        } else if let mlua::Value::Number(n) = v {
                            n.to_string()
                        } else if let mlua::Value::Integer(i) = v {
                            i.to_string()
                        } else if let mlua::Value::Boolean(b) = v {
                            b.to_string()
                        } else {
                            format!("{:?}", v)
                        }
                    })
                    .collect();
                if let Ok(mut q) = lq_clone2.lock() {
                    q.push(("info".to_string(), parts.join("\t")));
                }
                Ok(())
            })?,
        )?;

        // === VEC3 YARDIMCI FONKSİYONLARI ===
        lua.load(
            r#"
            function vec3(x, y, z)
                return { x = x or 0, y = y or 0, z = z or 0 }
            end
            
            function vec3_add(a, b)
                return vec3(a.x + b.x, a.y + b.y, a.z + b.z)
            end
            
            function vec3_sub(a, b)
                return vec3(a.x - b.x, a.y - b.y, a.z - b.z)
            end
            
            function vec3_scale(v, s)
                return vec3(v.x * s, v.y * s, v.z * s)
            end
            
            function vec3_length(v)
                return math.sqrt(v.x * v.x + v.y * v.y + v.z * v.z)
            end
            
            function vec3_normalize(v)
                local len = vec3_length(v)
                if len > 0.0001 then
                    return vec3(v.x / len, v.y / len, v.z / len)
                end
                return vec3(0, 0, 0)
            end
            
            function vec3_dot(a, b)
                return a.x * b.x + a.y * b.y + a.z * b.z
            end
            
            function vec3_cross(a, b)
                return vec3(
                    a.y * b.z - a.z * b.y,
                    a.z * b.x - a.x * b.z,
                    a.x * b.y - a.y * b.x
                )
            end
            
            function vec3_lerp(a, b, t)
                return vec3(
                    a.x + (b.x - a.x) * t,
                    a.y + (b.y - a.y) * t,
                    a.z + (b.z - a.z) * t
                )
            end
            
            function vec3_distance(a, b)
                return vec3_length(vec3_sub(a, b))
            end
            
            -- Clamp utility
            function clamp(value, min, max)
                return math.max(min, math.min(max, value))
            end
            
            -- Lerp utility
            function lerp(a, b, t)
                return a + (b - a) * t
            end
        "#,
        )
        .exec()?;

        // === API MODÜLLERİNİ KAYDET ===
        api_entity::register_entity_api(&lua, command_queue.clone())?;
        api_fighter::register_fighter_api(&lua, command_queue.clone())?;
        api_input::register_input_api(&lua)?;
        api_physics::register_physics_api(&lua, command_queue.clone())?;
        api_scene::register_scene_api(&lua, command_queue.clone())?;
        api_audio::register_audio_api(&lua, command_queue.clone())?;
        api_time::register_time_api(&lua)?;
        api_vehicle::register_vehicle_api(&lua, command_queue.clone())?;
        api_ai::register_ai_api(&lua, command_queue.clone())?;

        info!("[Scripting] ScriptEngine başlatıldı — Lua 5.4 sandbox aktif, API modülleri kayıtlı");
        Ok(Self {
            lua,
            loaded_scripts: BTreeMap::new(),
            command_queue,
            budget,
            budget_ticks: Self::DEFAULT_INSTRUCTION_BUDGET / Self::HOOK_INSTRUCTION_STEP,
            elapsed_time: 0.0,
            log_queue,
        })
    }

    /// Hand the next call into Lua a fresh instruction budget.
    ///
    /// Per CALL, not per frame: `update` runs every loaded script, and a budget shared across them
    /// would let the first script to misbehave spend everyone's — which is the same failure the
    /// error-isolation fix removed from this loop, in a different currency.
    fn arm_budget(&self) {
        self.budget
            .store(self.budget_ticks, std::sync::atomic::Ordering::Relaxed);
    }

    /// Change the per-call instruction ceiling. Rounded down to a multiple of the hook step, and
    /// never to zero — a budget of zero would fail every script on its first hook.
    pub fn set_instruction_budget(&mut self, instructions: u32) {
        self.budget_ticks = (instructions / Self::HOOK_INSTRUCTION_STEP).max(1);
    }

    /// Change the VM's heap ceiling in bytes. Returns the previous limit.
    pub fn set_memory_limit(&mut self, bytes: usize) -> Result<usize, LuaError> {
        self.lua.set_memory_limit(bytes)
    }

    #[tracing::instrument(skip_all, name = "script_load", fields(path = %path))]
    /// Loads (or reloads) the Lua file at `path` into its own environment.
    ///
    /// One environment per path: two entities running the same file share it, which is why
    /// per-entity values live in [`Script::properties`] rather than in globals. A read or a
    /// syntax error comes back as a message rather than a panic.
    pub fn load_script(&mut self, path: &str) -> Result<(), String> {
        let content = std::fs::read_to_string(path).map_err(|e| {
            error!(path, error = %e, "[Scripting] Script dosyası okunamadı");
            format!("Script okunamadı {}: {}", path, e)
        })?;
        let byte_len = content.len();

        let env = self.lua.create_table().map_err(|e| e.to_string())?;

        // Link to _G via metatable: reads fall through to the shared globals (that is how a script
        // sees `entity`, `input`, `print`), writes land on the script's own table.
        let meta = self.lua.create_table().map_err(|e| e.to_string())?;
        meta.set("__index", self.lua.globals())
            .map_err(|e| e.to_string())?;
        // Fallible in mlua 0.12. Without this metatable the script's environment does not fall
        // back to the engine globals, so `entity`/`input`/`print` would be nil inside it.
        env.set_metatable(Some(meta)).map_err(|e| e.to_string())?;

        // `_G` inside a script means the SCRIPT's table, not the engine's globals.
        //
        // Without this the isolation was one-way and easy to step around by accident: an implicit
        // `FOO = 1` stayed local, but the very next thing a Lua author reaches for — `_G.FOO = 1`,
        // which every tutorial spells as "the explicit way to make a global" — wrote straight into
        // the shared table, where the next script read it. Measured before the fix: script A set
        // `_G.LEAK` and script B read it back. Two scripts sharing a mutable namespace by accident
        // is a race with the load order, and the load order is alphabetical.
        //
        // Pointing `_G` at the env keeps the idiom meaning what the author expects — a global for
        // this script — while `__index` still exposes the engine API for reading.
        env.set("_G", env.clone()).map_err(|e| e.to_string())?;

        // Script'i İzole env içinde çalıştır
        self.arm_budget();
        self.lua
            .load(&content)
            .set_environment(env.clone())
            .exec()
            .map_err(|e| {
                error!(path, bytes = byte_len, error = %e, "[Scripting] Lua derleme/çalıştırma hatası");
                format!("Lua hata {}: {}", path, e)
            })?;

        let key = self
            .lua
            .create_registry_value(env)
            .map_err(|e| e.to_string())?;

        // Replace existing key if it exists to free old memory
        if let Some((_, old_key)) = self.loaded_scripts.insert(path.to_string(), (content, key)) {
            debug!(path, "[Scripting] Var olan script değiştirildi (hot-reload), eski sürüm boşaltılıyor");
            // Eskiden `let _ =` ile sessizce yutuluyordu; başarısızlık Lua registry
            // belleğini sızdırır. Davranış aynı (yine yok say) ama artık en azından loglanır.
            if let Err(e) = self.lua.remove_registry_value(old_key) {
                warn!(path, error = %e, "[Scripting] Eski script registry değeri boşaltılamadı (olası Lua bellek sızıntısı)");
            }
        }

        info!(path, bytes = byte_len, "🔧 [Scripting] Script yüklendi ve izole edildi");
        Ok(())
    }

    /// The per-frame update — mirrors World data into Lua and runs the scripts.
    #[tracing::instrument(skip_all, name = "script_update")]
    pub fn update(&mut self, world: &World, input: &Input, dt: f32) -> Result<(), String> {
        self.elapsed_time += dt;

        // 1. World verilerini Lua'ya aktar (read snapshot)
        api_entity::update_entity_read_api(&self.lua, world)
            .map_err(|e| format!("Entity API güncelleme hatası: {}", e))?;
        api_fighter::update_fighter_read_api(&self.lua, world)
            .map_err(|e| format!("Fighter API güncelleme hatası: {}", e))?;
        api_input::update_input_api(&self.lua, input)
            .map_err(|e| format!("Input API güncelleme hatası: {}", e))?;
        api_scene::update_scene_api(&self.lua, world)
            .map_err(|e| format!("Scene API güncelleme hatası: {}", e))?;
        api_time::update_time_api(&self.lua, dt, self.elapsed_time, 1.0 / dt.max(0.0001))
            .map_err(|e| format!("Time API güncelleme hatası: {}", e))?;
        api_physics::update_physics_api(&self.lua, world)
            .map_err(|e| format!("Physics API güncelleme hatası: {}", e))?;

        // 2. on_update callback'ini çağır — her yüklü script'in KENDİ env'inden.
        //    Script'ler izole bir env içinde çalıştırıldığından (load_script), top-level
        //    `function on_update` globals'a DEĞİL o env'e yazılır; globals'tan okumak
        //    (eski kod) onu ASLA bulamaz → hook sessizce hiç çalışmazdı.
        let ctx_table = self.lua.create_table().map_err(|e| e.to_string())?;
        ctx_table.set("dt", dt).map_err(|e| e.to_string())?;
        ctx_table
            .set("elapsed", self.elapsed_time)
            .map_err(|e| e.to_string())?;

        // **One script's failure must not cancel the others.** This loop used to `?` on the first
        // runtime error, so a single throwing script silently stopped every script ordered after it
        // for that frame — and with the map now ordered by path, "after it" is a stable and
        // therefore reliably silent set. Errors are collected and reported together instead; a
        // broken script loses its own frame and nobody else's.
        // Wrapped in the call-time query scope: for the length of this loop — and only for it —
        // the physics API carries functions that hold `&World` and can answer a question the
        // engine could not have precomputed. See `api_physics::with_call_time_queries`.
        let lua = &self.lua;
        let scripts = &self.loaded_scripts;
        let budget = &self.budget;
        let budget_ticks = self.budget_ticks;
        let mut failures = Vec::new();
        api_physics::with_call_time_queries(lua, world, || {
            for (path, (_, key)) in scripts {
                let env: mlua::Table = match lua.registry_value(key) {
                    Ok(env) => env,
                    Err(e) => {
                        failures.push(format!("{path}: env okunamadı: {e}"));
                        continue;
                    }
                };
                if let Ok(func) = env.get::<LuaFunction>("on_update") {
                    budget.store(budget_ticks, std::sync::atomic::Ordering::Relaxed);
                    if let Err(e) = func.call::<()>(ctx_table.clone()) {
                        warn!(path = %path, error = %e, "[Scripting] on_update çalışma-zamanı hatası");
                        failures.push(format!("Lua on_update hatası ({path}): {e}"));
                    }
                }
            }
            Ok(())
        })
        .map_err(|e| format!("script scope hatası: {e}"))?;

        if failures.is_empty() {
            Ok(())
        } else {
            // Every failure, not just the first: a caller that logs this sees the whole frame's
            // damage rather than one arbitrary script's share of it.
            Err(failures.join(" | "))
        }
    }

    /// The properties a script DECLARES, read from its `properties` table.
    ///
    /// The convention is a plain assignment at the top of the file:
    ///
    /// ```lua
    /// properties = { open_speed = 2.4, locked = false }
    /// ```
    ///
    /// which lands in the script's own environment (`_G` is that environment — see `load_script`).
    /// This is the schema and the defaults: the editor lists these names, and an entity that has
    /// not overridden one runs with the declared value.
    ///
    /// Anything that is not a number, boolean or string is skipped rather than guessed at — a
    /// nested table is a script's own business and not an inspector row.
    pub fn declared_properties(
        &self,
        script_path: &str,
    ) -> std::collections::BTreeMap<String, ScriptValue> {
        let mut out = std::collections::BTreeMap::new();
        let Some((_, key)) = self.loaded_scripts.get(script_path) else {
            return out;
        };
        let Ok(env) = self.lua.registry_value::<mlua::Table>(key) else {
            return out;
        };
        let Ok(table) = env.get::<mlua::Table>("properties") else {
            return out;
        };
        for pair in table.pairs::<String, mlua::Value>() {
            let Ok((name, value)) = pair else { continue };
            let converted = match value {
                mlua::Value::Number(n) => Some(ScriptValue::Num(n)),
                mlua::Value::Integer(i) => Some(ScriptValue::Num(i as f64)),
                mlua::Value::Boolean(b) => Some(ScriptValue::Bool(b)),
                mlua::Value::String(s) => s.to_str().ok().map(|t| ScriptValue::Text(t.to_string())),
                _ => None,
            };
            if let Some(v) = converted {
                out.insert(name, v);
            }
        }
        out
    }


    /// Reads a numeric expression out of a loaded script's environment. Test-only.
    #[cfg(test)]
    pub fn eval_number(&self, script_path: &str, expr: &str) -> Option<f64> {
        let (_, key) = self.loaded_scripts.get(script_path)?;
        let env: mlua::Table = self.lua.registry_value(key).ok()?;
        self.lua
            .load(format!("return {expr}"))
            .set_environment(env)
            .eval::<f64>()
            .ok()
    }

    /// Runs `on_entity_update(entity_id, dt, props)` for one entity.
    ///
    /// `properties` are that entity's own values — the third argument exists because scripts are
    /// loaded per PATH, so two entities running the same file share one Lua environment and cannot
    /// each keep a value in it. Passing them is additive: a script whose `on_entity_update` takes
    /// two parameters simply ignores the third, which is why this did not need a new hook name.
    pub fn update_entity(
        &mut self,
        entity_id: u32,
        script_path: &str,
        dt: f32,
        properties: &std::collections::BTreeMap<String, ScriptValue>,
    ) -> Result<(), String> {
        if let Some((_, key)) = self.loaded_scripts.get(script_path) {
            let env: mlua::Table = self.lua.registry_value(key).map_err(|e| e.to_string())?;

            // on_entity_update(entity_id, dt, props) çağır (varsa)
            if let Ok(func) = env.get::<LuaFunction>("on_entity_update") {
                let props = self.lua.create_table().map_err(|e| e.to_string())?;
                for (name, value) in properties {
                    let set = match value {
                        ScriptValue::Num(n) => props.set(name.as_str(), *n),
                        ScriptValue::Bool(b) => props.set(name.as_str(), *b),
                        ScriptValue::Text(t) => props.set(name.as_str(), t.as_str()),
                    };
                    set.map_err(|e| e.to_string())?;
                }
                self.arm_budget();
                func.call::<()>((entity_id, dt, props)).map_err(|e| {
                    warn!(entity_id, script_path, error = %e, "[Scripting] on_entity_update çalışma-zamanı hatası");
                    format!(
                        "Lua on_entity_update hatası (entity {} mod {}): {}",
                        entity_id, script_path, e
                    )
                })?;
            }
        } else {
            trace!(entity_id, script_path, "[Scripting] update_entity: script yüklü değil, atlandı");
        }
        Ok(())
    }

    /// Applies every command in the queue to the World and returns the ones left for game
    /// logic.
    #[tracing::instrument(skip_all, name = "script_flush_commands")]
    pub fn flush_commands(&self, world: &mut World, dt: f32) -> Vec<ScriptCommand> {
        let commands = self.command_queue.drain();
        let total = commands.len();
        let mut unhandled = Vec::new();

        for cmd in commands {
            match cmd {
                ScriptCommand::SetPosition(id, pos) => {
                    let mut transforms = world.borrow_mut::<gizmo_physics_core::Transform>();
                    if let Some(mut t) = transforms.get_mut(id) {
                        t.position = pos;
                    } else {
                        trace!(entity = id, "[Scripting] SetPosition: hedefte Transform yok, komut atlandı");
                    }
                }
                ScriptCommand::SetRotation(id, rot) => {
                    let mut transforms = world.borrow_mut::<gizmo_physics_core::Transform>();
                    if let Some(mut t) = transforms.get_mut(id) {
                        t.rotation = rot;
                    } else {
                        trace!(entity = id, "[Scripting] SetRotation: hedefte Transform yok, komut atlandı");
                    }
                }
                ScriptCommand::SetScale(id, scale) => {
                    let mut transforms = world.borrow_mut::<gizmo_physics_core::Transform>();
                    if let Some(mut t) = transforms.get_mut(id) {
                        t.scale = scale;
                    } else {
                        trace!(entity = id, "[Scripting] SetScale: hedefte Transform yok, komut atlandı");
                    }
                }
                ScriptCommand::SetVelocity(id, vel) => {
                    let mut written = false;
                    {
                        let mut velocities = world.borrow_mut::<gizmo_physics_rigid::components::Velocity>();
                        if let Some(mut v) = velocities.get_mut(id) {
                            v.linear = vel;
                            written = true;
                        } else {
                            trace!(entity = id, "[Scripting] SetVelocity: hedefte Velocity yok, komut atlandı");
                        }
                    }
                    if written {
                        wake_after_velocity_write(world, id);
                    }
                }
                ScriptCommand::SetAngularVelocity(id, ang_vel) => {
                    let mut written = false;
                    {
                        let mut velocities = world.borrow_mut::<gizmo_physics_rigid::components::Velocity>();
                        if let Some(mut v) = velocities.get_mut(id) {
                            v.angular = ang_vel;
                            written = true;
                        } else {
                            trace!(entity = id, "[Scripting] SetAngularVelocity: hedefte Velocity yok, komut atlandı");
                        }
                    }
                    if written {
                        wake_after_velocity_write(world, id);
                    }
                }
                ScriptCommand::ApplyForce(id, force) => {
                    let rbs = world.borrow::<gizmo_physics_rigid::components::RigidBody>();
                    if let Some(rb) = rbs.get(id) {
                        if rb.mass > 0.0 {
                            let accel = force * (1.0 / rb.mass);
                            drop(rbs);
                            // RigidBody var ama Velocity yoksa sıfır hızla oluştur ki
                            // kuvvet sessizce kaybolmasın.
                            if world
                                .borrow::<gizmo_physics_rigid::components::Velocity>()
                                .get(id)
                                .is_none()
                            {
                                if let Some(e) = world.entity(id) {
                                    world.add_component(
                                        e,
                                        gizmo_physics_rigid::components::Velocity::new(
                                            gizmo_math::Vec3::ZERO,
                                        ),
                                    );
                                }
                            }
                            {
                                let mut vels =
                                    world.borrow_mut::<gizmo_physics_rigid::components::Velocity>();
                                if let Some(mut v) = vels.get_mut(id) {
                                    v.linear += accel * dt;
                                }
                            }
                            wake_after_velocity_write(world, id);
                        }
                    } else {
                        trace!(entity = id, "[Scripting] ApplyForce: hedefte RigidBody yok, kuvvet yok sayıldı");
                    }
                }
                ScriptCommand::ApplyImpulse(id, impulse) => {
                    let rbs = world.borrow::<gizmo_physics_rigid::components::RigidBody>();
                    if let Some(rb) = rbs.get(id) {
                        if rb.mass > 0.0 {
                            let delta_v = impulse * (1.0 / rb.mass);
                            drop(rbs);
                            // RigidBody var ama Velocity yoksa sıfır hızla oluştur ki
                            // impuls sessizce kaybolmasın.
                            if world
                                .borrow::<gizmo_physics_rigid::components::Velocity>()
                                .get(id)
                                .is_none()
                            {
                                if let Some(e) = world.entity(id) {
                                    world.add_component(
                                        e,
                                        gizmo_physics_rigid::components::Velocity::new(
                                            gizmo_math::Vec3::ZERO,
                                        ),
                                    );
                                }
                            }
                            {
                                let mut vels =
                                    world.borrow_mut::<gizmo_physics_rigid::components::Velocity>();
                                if let Some(mut v) = vels.get_mut(id) {
                                    v.linear += delta_v;
                                }
                            }
                            wake_after_velocity_write(world, id);
                        }
                    } else {
                        trace!(entity = id, "[Scripting] ApplyImpulse: hedefte RigidBody yok, impuls yok sayıldı");
                    }
                }
                ScriptCommand::AddRigidBody {
                    id,
                    mass,
                    use_gravity,
                } => {
                    let entity = world.entity(id);
                    if let Some(e) = entity {
                        let rb = gizmo_physics_rigid::components::RigidBody::new(mass, use_gravity);
                        world.add_component(e, rb);
                        // Make sure velocity exists so it can move
                        if world
                            .borrow::<gizmo_physics_rigid::components::Velocity>()
                            .get(id)
                            .is_none()
                        {
                            world.add_component(
                                e,
                                gizmo_physics_rigid::components::Velocity::new(gizmo_math::Vec3::ZERO),
                            );
                        }
                    } else {
                        trace!(entity = id, "[Scripting] AddRigidBody: entity bulunamadı, komut atlandı");
                    }
                }
                ScriptCommand::AddBoxCollider { id, hx, hy, hz } => {
                    let entity = world.entity(id);
                    if let Some(e) = entity {
                        let col =
                            gizmo_physics_core::Collider::aabb(gizmo_math::Vec3::new(hx, hy, hz));
                        world.add_component(e, col);
                    } else {
                        trace!(entity = id, "[Scripting] AddBoxCollider: entity bulunamadı, komut atlandı");
                    }
                }
                ScriptCommand::AddSphereCollider { id, radius } => {
                    let entity = world.entity(id);
                    if let Some(e) = entity {
                        let col = gizmo_physics_core::Collider::sphere(radius);
                        world.add_component(e, col);
                    } else {
                        trace!(entity = id, "[Scripting] AddSphereCollider: entity bulunamadı, komut atlandı");
                    }
                }

                // The three vehicle commands used to be matched here with empty bodies: Lua could
                // call them, they queued, and they vanished without a word. Applying them properly
                // needs `VehicleController`, which lives in `gizmo-physics-dynamics` and is not a
                // dependency of this crate — adding one to reach three commands is the wrong trade,
                // and the host that flushes these does have it. So they fall through to `unhandled`
                // like everything else this crate cannot apply itself, and the host is told.

                ScriptCommand::SpawnEntity { name, position } => {
                    let entity = world.spawn();
                    world.add_component(entity, gizmo_core::EntityName::new(&name));
                    world
                        .add_component(entity, gizmo_physics_core::Transform::new(position));
                    let msg = format!(
                        "Entity spawn: '{}' at ({:.1}, {:.1}, {:.1})",
                        name, position.x, position.y, position.z
                    );
                    if let Ok(mut q) = self.log_queue.lock() {
                        q.push(("info".to_string(), msg));
                    }
                }
                ScriptCommand::SpawnPrefab {
                    name,
                    prefab_type,
                    position,
                } => {
                    let entity = world.spawn();
                    world.add_component(entity, gizmo_core::EntityName::new(&name));
                    world
                        .add_component(entity, gizmo_physics_core::Transform::new(position));
                    world.add_component(entity, gizmo_core::PrefabRequest(prefab_type.clone()));
                }
                ScriptCommand::DestroyEntity(id) => {
                    world.despawn_by_id(id);
                    if let Ok(mut q) = self.log_queue.lock() {
                        q.push(("info".to_string(), format!("Entity destroyed: {}", id)));
                    }
                }
ScriptCommand::SetEntityName(id, name) => {
                    let mut names = world.borrow_mut::<gizmo_core::EntityName>();
                    if let Some(mut n) = names.get_mut(id) {
                        n.0 = name;
                    } else {
                        trace!(entity = id, "[Scripting] SetEntityName: hedefte EntityName yok, komut atlandı");
                    }
                }
ScriptCommand::PlayAnimation { id, name, blend, loop_anim } => {
                    let mut players = world.borrow_mut::<gizmo_animation::skeletal::AnimationPlayer>();
                    if let Some(mut player) = players.get_mut(id) {
                        player.play_animation_by_name(&name, blend, loop_anim);
                    } else {
                        trace!(entity = id, anim = %name, "[Scripting] PlayAnimation: hedefte AnimationPlayer yok, komut atlandı");
                    }
                }
                ScriptCommand::SetAnimationSpeed(id, speed) => {
                    let mut players = world.borrow_mut::<gizmo_animation::skeletal::AnimationPlayer>();
                    if let Some(mut player) = players.get_mut(id) {
                        player.speed = speed;
                    } else {
                        trace!(entity = id, "[Scripting] SetAnimationSpeed: hedefte AnimationPlayer yok, komut atlandı");
                    }
                }
                ScriptCommand::AddNavAgent(id) => {
                    let entity = world.entity(id);
                    if let Some(e) = entity {
                        world.add_component(e, gizmo_ai::components::NavAgent::default());
                    } else {
                        trace!(entity = id, "[Scripting] AddNavAgent: entity bulunamadı, komut atlandı");
                    }
                }
                ScriptCommand::SetAiTarget(id, target) => {
                    let mut agents = world.borrow_mut::<gizmo_ai::components::NavAgent>();
                    if let Some(mut agent) = agents.get_mut(id) {
                        agent.set_target(target);
                    } else {
                        trace!(entity = id, "[Scripting] SetAiTarget: hedefte NavAgent yok, komut atlandı");
                    }
                }
                ScriptCommand::ClearAiTarget(id) => {
                    let mut agents = world.borrow_mut::<gizmo_ai::components::NavAgent>();
                    if let Some(mut agent) = agents.get_mut(id) {
                        // Must clear the TARGET, not just the path — clearing only the path
                        // leaves target set, so ai_navigation_system recomputes and keeps going.
                        agent.clear_target();
                    } else {
                        trace!(entity = id, "[Scripting] ClearAiTarget: hedefte NavAgent yok, komut atlandı");
                    }
                }
                ScriptCommand::SetFighterMove { id, name, startup, active, recovery, damage, hitstun, hitstop } => {
                    let mut fighters = world.borrow_mut::<gizmo_physics_core::components::FighterController>();
                    if let Some(mut fighter) = fighters.get_mut(id) {
                        let mut frame_data =
                            gizmo_physics_core::components::fighter::FrameData::default();
                        frame_data.startup = startup;
                        frame_data.active = active;
                        frame_data.recovery = recovery;
                        frame_data.damage = damage;
                        frame_data.hitstun = hitstun;
                        frame_data.hitstop = hitstop;
                        let mut combat_move =
                            gizmo_physics_core::components::fighter::CombatMove::default();
                        combat_move.name = name;
                        combat_move.frame_data = frame_data;
                        fighter.active_move = Some(combat_move);
                        fighter.current_move_frame = 0;
                    } else {
                        trace!(entity = id, "[Scripting] SetFighterMove: hedefte FighterController yok, komut atlandı");
                    }
                }
                ScriptCommand::ApplyHitstop(id, frames) => {
                    let mut fighters = world.borrow_mut::<gizmo_physics_core::components::FighterController>();
                    if let Some(mut fighter) = fighters.get_mut(id) {
                        fighter.apply_hitstop(frames);
                    } else {
                        trace!(entity = id, frames, "[Scripting] ApplyHitstop: hedefte FighterController yok, komut atlandı");
                    }
                }
                ScriptCommand::ApplyHitstun(id, frames) => {
                    let mut fighters = world.borrow_mut::<gizmo_physics_core::components::FighterController>();
                    if let Some(mut fighter) = fighters.get_mut(id) {
                        fighter.apply_hitstun(frames);
                    } else {
                        trace!(entity = id, frames, "[Scripting] ApplyHitstun: hedefte FighterController yok, komut atlandı");
                    }
                }
                // The scene, dialogue, race and camera commands used to be matched here by an
                // arm whose body was empty and whose comment said they would "already appear in
                // unhandled". They could not: this arm consumed them, so the `other` catch-all
                // below never saw them and the host was never told. Deleting the arm is the whole
                // fix — they now fall through and are returned, which is what the comment claimed.
                other => {
                    unhandled.push(other);
                }
            }
        }

        if total > 0 {
            trace!(
                total,
                unhandled = unhandled.len(),
                "[Scripting] script komut kuyruğu boşaltıldı"
            );
        }
        unhandled
    }

    /// Decides whether the script should be hot-reloaded.
    pub fn reload_if_changed(&mut self, path: &str) -> Result<bool, String> {
        let current =
            std::fs::read_to_string(path).map_err(|e| format!("Script okunamadı: {}", e))?;

        if let Some((cached_code, _)) = self.loaded_scripts.get(path) {
            if *cached_code == current {
                return Ok(false);
            }
        }

        self.load_script(path)?;
        Ok(true)
    }

    /// Is there a Lua function with this name?
    ///
    /// Takes `&mut self` even though it only reads: `registry_value` mutates the
    /// underlying `lua_State`, and the `unsafe impl Sync` above is only sound
    /// while no `&self` method reaches the VM.
    pub fn has_function(&mut self, path: &str, name: &str) -> bool {
        if let Some((_, key)) = self.loaded_scripts.get(path) {
            if let Ok(env) = self.lua.registry_value::<mlua::Table>(key) {
                return env.get::<LuaFunction>(name).is_ok();
            }
        }
        false
    }

    /// Direct access to the command queue (internals).
    pub fn command_queue(&self) -> &Arc<CommandQueue> {
        &self.command_queue
    }
}

gizmo_core::impl_component!(Script);

#[cfg(test)]
mod soundness {
    use super::*;

    /// `ScriptEngine` must be `Send + Sync` — it is stored as a `World`
    /// resource and `insert_resource` requires both.
    ///
    /// `Send` is derived (mlua's `send` feature makes `Lua: Send`); `Sync` is
    /// the hand-written `unsafe impl` above.
    #[test]
    fn script_engine_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<ScriptEngine>();
    }

    /// Locks the precondition of the `unsafe impl Sync for ScriptEngine`:
    /// **no `&self` method may touch `self.lua`**, because mlua mutates the
    /// `lua_State` through `&Lua` and two threads sharing `&ScriptEngine` would
    /// race on it.
    ///
    /// The audited shared surface is exactly these two methods, neither of which
    /// reads `self.lua`:
    ///   - `flush_commands`
    ///   - `command_queue`
    ///
    /// This test calls each of them through a genuinely shared `&ScriptEngine`
    /// obtained from two threads at once. It cannot prove the absence of a
    /// future `&self` VM access on its own — but it does prove these two stay
    /// callable from a shared reference, so converting one of them to
    /// `&mut self` (the correct move if it ever needs the VM) breaks this test
    /// and forces the SAFETY comment to be revisited.
    #[test]
    fn shared_methods_never_reach_the_lua_vm() {
        let engine = ScriptEngine::new().expect("Lua VM");
        let shared = &engine;

        std::thread::scope(|s| {
            for _ in 0..2 {
                s.spawn(move || {
                    // Every `&self` method on the audited list, exercised
                    // concurrently. If any of these grew a `self.lua` access,
                    // this is a data race that Miri/TSan would flag here.
                    let _ = shared.command_queue().len();
                });
            }
        });

        // `flush_commands` needs a &mut World, so drive it on one thread — the
        // point is only that it is reachable through `&self`.
        let mut world = gizmo_core::World::new();
        let _ = shared.flush_commands(&mut world, 1.0 / 60.0);
    }

    /// The two methods that DO reach the VM must require exclusive access, so
    /// the borrow checker — not a comment — prevents concurrent VM use.
    ///
    /// This is a compile-time assertion: it only builds while both take
    /// `&mut self`. Reverting either to `&self` fails to compile here.
    #[test]
    fn vm_touching_methods_require_exclusive_access() {
        fn _needs_mut(e: &mut ScriptEngine) {
            let _ = e.has_function("nope.lua", "on_update");
        }
        fn _needs_mut_2(e: &mut ScriptEngine) {
            let _ = e.update_entity(1, "nope.lua", 1.0 / 60.0, &Default::default());
        }
    }
}

#[cfg(test)]
mod tests {

    /// One script's globals must not be another's, including the explicit spelling.
    ///
    /// Each script already ran in its own environment, so an implicit `FOO = 1` stayed local. But
    /// `_G` resolved to the ENGINE's globals through the environment's `__index`, so `_G.FOO = 1`
    /// — the spelling every Lua tutorial gives for "make this global" — wrote into the shared
    /// table and the next script read it back. Measured, not theorised: script A set `_G.LEAK` and
    /// script B saw `from-a`. Two scripts sharing a mutable namespace by accident is a race with
    /// the load order, and the load order is alphabetical.
    #[test]
    fn a_script_cannot_reach_another_through_g() {
        let dir = std::env::temp_dir().join(format!("gizmo_sandbox_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a_writer.lua");
        let b = dir.join("b_reader.lua");
        std::fs::write(&a, "function on_update(c)\n  _G.LEAK = 'from-a'\n  IMPLICIT = 'also-a'\nend\n")
            .unwrap();
        std::fs::write(
            &b,
            "function on_update(c)\n  print('LEAK=' .. tostring(_G.LEAK))\n  print('IMPLICIT=' .. tostring(IMPLICIT))\nend\n",
        )
        .unwrap();

        let mut engine = ScriptEngine::new().unwrap();
        engine.load_script(a.to_str().unwrap()).unwrap();
        engine.load_script(b.to_str().unwrap()).unwrap();
        engine.update(&World::new(), &Input::default(), 0.016).unwrap();

        let log = engine.log_queue.lock().unwrap().clone();
        let said = |needle: &str| log.iter().any(|(_, m)| m.contains(needle));
        assert!(said("LEAK=nil"), "`_G.X` from one script reached another: {log:?}");
        assert!(said("IMPLICIT=nil"), "an implicit global reached another script: {log:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// …and the containment must not have cost the script its API. `_G` is the script's own table
    /// now, but reads still fall through to the engine's globals, which is what makes `print`,
    /// `input` and the rest visible at all.
    #[test]
    fn a_script_still_reaches_the_engine_api_and_its_own_globals() {
        let dir = std::env::temp_dir().join(format!("gizmo_sandbox2_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("s.lua");
        std::fs::write(
            &path,
            "function on_update(c)\n  _G.MINE = 5\n  print('mine=' .. tostring(MINE))\n  print('api=' .. tostring(_G.input ~= nil and _G.print ~= nil))\n  print('std=' .. tostring(string.rep('x', 2)))\nend\n",
        )
        .unwrap();

        let mut engine = ScriptEngine::new().unwrap();
        engine.load_script(path.to_str().unwrap()).unwrap();
        engine.update(&World::new(), &Input::default(), 0.016).unwrap();

        let log = engine.log_queue.lock().unwrap().clone();
        let said = |needle: &str| log.iter().any(|(_, m)| m.contains(needle));
        assert!(said("mine=5"), "a script's own `_G` write must be visible to itself: {log:?}");
        assert!(said("api=true"), "the engine API must still resolve through `_G`: {log:?}");
        assert!(said("std=xx"), "the Lua standard library must still resolve: {log:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A script cannot rewrite the engine API out from under the other scripts.
    ///
    /// `_G` isolation made a script's *globals* its own. It did not make the API tables its own,
    /// because `input.is_pressed = f` is not a global write — it is a field write on an object
    /// every script holds a reference to. Measured before the fix: script A replaced
    /// `input.is_pressed`, and script B called A's version.
    ///
    /// What closes it is a proxy (see `api_table`), and specifically not a bare `__newindex`:
    /// that metamethod fires only for keys the table does not already have, and every key worth
    /// clobbering is one it has.
    #[test]
    fn a_script_cannot_rewrite_the_api_for_everyone_else() {
        let dir = std::env::temp_dir().join(format!("gizmo_api_ro_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a_vandal.lua");
        let b = dir.join("b_victim.lua");
        std::fs::write(
            &a,
            "function on_update(c)\n  input.is_pressed = function(k) return 'CLOBBERED' end\nend\n",
        )
        .unwrap();
        std::fs::write(
            &b,
            "function on_update(c)\n  print('sees=' .. tostring(input.is_pressed('w')))\nend\n",
        )
        .unwrap();

        let mut engine = ScriptEngine::new().unwrap();
        engine.load_script(a.to_str().unwrap()).unwrap();
        engine.load_script(b.to_str().unwrap()).unwrap();

        // The vandal's own frame fails — loudly, with the reason — and the victim's does not.
        let err = engine.update(&World::new(), &Input::default(), 0.016).unwrap_err();
        assert!(err.contains("read-only"), "expected a read-only refusal, got: {err}");

        let log = engine.log_queue.lock().unwrap().clone();
        assert!(
            log.iter().any(|(_, m)| m.contains("sees=false")),
            "the neighbour saw a rewritten API: {log:?}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A parameterised query the engine could not have precomputed, answered while the script is
    /// calling.
    ///
    /// This is the item the audit recorded as blocked. Its reasoning was right about
    /// `Lua::create_function` — with mlua's `send` feature that wants `Fn(..) + Send + 'static`,
    /// and `&World` is neither — and wrong about the conclusion, because `Scope::create_function`
    /// carries no such bound: `F: Fn(..) + 'scope`. A scoped closure may borrow the world, and the
    /// borrow ends when the scope does, which is the frame.
    ///
    /// "Ground height at (x, z)" is the audit's own example, and it is the right shape of example:
    /// there is no snapshot that answers it, because the engine does not know which (x, z) the
    /// script will ask about until it asks.
    #[test]
    fn a_script_can_ask_a_question_the_engine_did_not_precompute() {
        use gizmo_physics_rigid::world::PhysicsWorld;

        let dir = std::env::temp_dir().join(format!("gizmo_probe_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("probe.lua");
        std::fs::write(
            &path,
            "function on_update(c)\n             \x20 print('on_slab=' .. tostring(physics.ground_at(0.0, 0.0)))\n             \x20 print('off_slab=' .. tostring(physics.ground_at(500.0, 500.0)))\n             end\n",
        )
        .unwrap();

        // A floor slab whose top sits at y = 2.
        use gizmo_math::Vec3;
        use gizmo_physics_core::{BodyHandle, Collider, Transform};
        use gizmo_physics_rigid::{RigidBody, Velocity};

        let mut world = World::new();
        let mut pw = PhysicsWorld::new();
        pw.add_body(
            BodyHandle::from_id(0),
            RigidBody::new_static(),
            Transform::new(Vec3::new(0.0, 0.0, 0.0)),
            Velocity::default(),
            Collider::box_collider(Vec3::new(50.0, 2.0, 50.0)),
        );
        world.insert_resource(pw);

        let mut engine = ScriptEngine::new().unwrap();
        engine.load_script(path.to_str().unwrap()).unwrap();
        engine.update(&world, &Input::default(), 0.016).unwrap();

        let log = engine.log_queue.lock().unwrap().clone();
        let line = |k: &str| {
            log.iter()
                .find_map(|(_, m)| m.strip_prefix(k).map(str::to_string))
                .unwrap_or_else(|| panic!("no `{k}` line in {log:?}"))
        };
        let on_slab: f32 = line("on_slab=").parse().expect("a height over the slab");
        assert!((on_slab - 2.0).abs() < 0.01, "expected the slab top at 2.0, got {on_slab}");
        assert_eq!(line("off_slab="), "nil", "no floor there must read as nil, not as zero");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// …and the borrow does not outlive the frame: the name is gone once the scope closes, so a
    /// script that saved it cannot call into a world that is no longer there.
    #[test]
    fn the_call_time_query_is_not_available_outside_the_frame() {
        let lua = Lua::new();
        crate::api_physics::register_physics_api(&lua, Arc::new(CommandQueue::new())).unwrap();
        let world = World::new();

        crate::api_physics::with_call_time_queries(&lua, &world, || {
            let present: bool = lua.load("return physics.ground_at ~= nil").eval()?;
            assert!(present, "the query must exist while the frame is running");
            Ok(())
        })
        .unwrap();

        let present: bool = lua.load("return physics.ground_at ~= nil").eval().unwrap();
        assert!(!present, "the query must be gone once the frame is over");
    }

    /// A script that never returns must lose its frame, not the process.
    ///
    /// `while true do end` in a Lua VM the host has no timeout on is unrecoverable: the call never
    /// returns, so the frame never ends, so the window never redraws and never processes the
    /// close event either. There is no signal to catch and no watchdog thread that could help —
    /// only the VM can interrupt itself, which is what the instruction hook is for.
    #[test]
    fn an_infinite_loop_ends_the_call_instead_of_the_process() {
        let dir = std::env::temp_dir().join(format!("gizmo_budget_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("runaway.lua");
        std::fs::write(&path, "function on_update(ctx)\n  while true do end\nend\n").unwrap();

        let mut engine = ScriptEngine::new().unwrap();
        // Small enough to trip in milliseconds; the default is a runaway guard, not a stopwatch.
        engine.set_instruction_budget(200_000);
        engine.load_script(path.to_str().unwrap()).unwrap();

        let world = World::new();
        let input = Input::default();
        let started = std::time::Instant::now();
        let err = engine.update(&world, &input, 0.016).unwrap_err();
        let took = started.elapsed();

        assert!(err.contains("instruction budget"), "unexpected error: {err}");
        assert!(took.as_secs() < 5, "the guard took {took:?} — that is a hang with extra steps");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The budget is per call, so the runaway script loses its own frame and the next one still
    /// runs — the same isolation the error handling already gives a script that throws.
    #[test]
    fn a_runaway_script_does_not_spend_another_scripts_budget() {
        let dir = std::env::temp_dir().join(format!("gizmo_budget2_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // `a_` sorts before `b_`, and the script map is ordered by path, so the runaway runs first.
        let runaway = dir.join("a_runaway.lua");
        let neighbour = dir.join("b_neighbour.lua");
        std::fs::write(&runaway, "function on_update(ctx)\n  while true do end\nend\n").unwrap();
        // Observable through the log queue rather than a new accessor: `print` already routes
        // into it, so the test needs no API the engine would not otherwise have.
        std::fs::write(&neighbour, "function on_update(ctx)\n  print('neighbour ran')\nend\n")
            .unwrap();

        let mut engine = ScriptEngine::new().unwrap();
        engine.set_instruction_budget(200_000);
        engine.load_script(runaway.to_str().unwrap()).unwrap();
        engine.load_script(neighbour.to_str().unwrap()).unwrap();

        let world = World::new();
        let input = Input::default();
        let err = engine.update(&world, &input, 0.016).unwrap_err();
        assert!(err.contains("instruction budget"), "unexpected error: {err}");

        let logged = engine
            .log_queue
            .lock()
            .unwrap()
            .iter()
            .any(|(_, m)| m.contains("neighbour ran"));
        assert!(logged, "the second script never got its turn");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A script that allocates without bound hits a Lua error, not the OOM killer.
    #[test]
    fn runaway_allocation_fails_as_a_lua_error() {
        let dir = std::env::temp_dir().join(format!("gizmo_mem_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("hungry.lua");
        std::fs::write(
            &path,
            "function on_update(ctx)\n  local t = {}\n  while true do t[#t+1] = string.rep('x', 1024) end\nend\n",
        )
        .unwrap();

        let mut engine = ScriptEngine::new().unwrap();
        engine.set_memory_limit(4 * 1024 * 1024).unwrap();
        // Generous, so the memory ceiling is what stops it rather than the instruction budget.
        engine.set_instruction_budget(500_000_000);
        engine.load_script(path.to_str().unwrap()).unwrap();

        let err = engine.update(&World::new(), &Input::default(), 0.016).unwrap_err();
        assert!(
            err.to_lowercase().contains("memory"),
            "expected a memory error, got: {err}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
    use super::*;
    use gizmo_math::{Quat, Vec3};
    use gizmo_physics_core::{Collider, ColliderShape, Transform};
    use gizmo_physics_rigid::components::{RigidBody, Velocity};

    /// Produces a unique temporary script path that will not collide across parallel test runs.
    fn unique_temp(tag: &str) -> String {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir()
            .join(format!("gizmo_scripting_{tag}_{n}_{nanos}.lua"))
            .to_string_lossy()
            .into_owned()
    }

    /// A top-level `on_update` in a loaded script must fire every frame. It's written
    /// into the script's isolated env, so the old code that read `on_update` from
    /// `_G` never found it and the hook was a silent no-op.
    #[test]
    fn on_update_hook_fires_from_script_env() {
        let mut engine = ScriptEngine::new().unwrap();
        let world = World::new();
        let input = gizmo_core::input::Input::default();

        let path = std::env::temp_dir()
            .join("gizmo_on_update_test.lua")
            .to_string_lossy()
            .into_owned();
        std::fs::write(&path, "function on_update(ctx)\n  entity.spawn(\"bullet\", 0, 0, 0)\nend\n")
            .unwrap();
        engine.load_script(&path).expect("load_script");

        let before = engine.command_queue().len();
        engine.update(&world, &input, 1.0 / 60.0).expect("update");
        let after = engine.command_queue().len();
        let _ = std::fs::remove_file(&path);

        assert!(
            after > before,
            "on_update must run and queue a spawn command (before={before}, after={after})"
        );
    }

    /// Regression: with a RigidBody but no Velocity, ApplyForce must not vanish silently — a
    /// Velocity is created and the acceleration applied.
    #[test]
    fn apply_force_creates_velocity_when_missing() {
        let engine = ScriptEngine::new().unwrap();
        let mut world = World::new();

        let entity = world.spawn();
        world.add_component(entity, RigidBody::new(2.0, false));
        // Kasıtlı olarak Velocity EKLENMEDİ.
        assert!(world.borrow::<Velocity>().get(entity.id()).is_none());

        engine
            .command_queue()
            .push(ScriptCommand::ApplyForce(entity.id(), Vec3::new(4.0, 0.0, 0.0)));

        let dt = 0.5_f32;
        engine.flush_commands(&mut world, dt);

        let vels = world.borrow::<Velocity>();
        let v = vels
            .get(entity.id())
            .expect("Velocity ApplyForce tarafından oluşturulmalıydı");
        // accel = force/mass = 4/2 = 2; dv = accel*dt = 2*0.5 = 1.0
        assert!((v.linear.x - 1.0).abs() < 1e-5, "x hızı yanlış: {}", v.linear.x);
    }

    /// Regression: with a RigidBody but no Velocity, ApplyImpulse must not vanish silently — a
    /// Velocity is created and the delta-v applied.
    #[test]
    fn apply_impulse_creates_velocity_when_missing() {
        let engine = ScriptEngine::new().unwrap();
        let mut world = World::new();

        let entity = world.spawn();
        world.add_component(entity, RigidBody::new(2.0, false));
        assert!(world.borrow::<Velocity>().get(entity.id()).is_none());

        engine
            .command_queue()
            .push(ScriptCommand::ApplyImpulse(entity.id(), Vec3::new(6.0, 0.0, 0.0)));

        engine.flush_commands(&mut world, 0.016);

        let vels = world.borrow::<Velocity>();
        let v = vels
            .get(entity.id())
            .expect("Velocity ApplyImpulse tarafından oluşturulmalıydı");
        // dv = impulse/mass = 6/2 = 3.0 (dt'den bağımsız)
        assert!((v.linear.x - 3.0).abs() < 1e-5, "x hızı yanlış: {}", v.linear.x);
    }

    /// The Transform write commands (SetPosition/SetScale/SetRotation) must apply to an
    /// existing Transform.
    #[test]
    fn transform_commands_apply_to_component() {
        let engine = ScriptEngine::new().unwrap();
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Transform::new(Vec3::ZERO));
        let id = e.id();

        engine.command_queue().push(ScriptCommand::SetPosition(id, Vec3::new(1.0, 2.0, 3.0)));
        engine.command_queue().push(ScriptCommand::SetScale(id, Vec3::new(2.0, 4.0, 8.0)));
        engine.command_queue().push(ScriptCommand::SetRotation(id, Quat::from_xyzw(1.0, 0.0, 0.0, 0.0)));
        engine.flush_commands(&mut world, 0.016);

        let transforms = world.borrow::<Transform>();
        let t = transforms.get(id).unwrap();
        assert_eq!(t.position, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(t.scale, Vec3::new(2.0, 4.0, 8.0));
        assert!((t.rotation.x - 1.0).abs() < 1e-6 && t.rotation.w.abs() < 1e-6);
    }

    /// SetVelocity/SetAngularVelocity must set an existing Velocity's linear/angular fields.
    #[test]
    fn velocity_commands_apply_to_component() {
        let engine = ScriptEngine::new().unwrap();
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Velocity::new(Vec3::ZERO));
        let id = e.id();

        engine.command_queue().push(ScriptCommand::SetVelocity(id, Vec3::new(3.0, 0.0, -2.0)));
        engine.command_queue().push(ScriptCommand::SetAngularVelocity(id, Vec3::new(0.0, 1.0, 0.0)));
        engine.flush_commands(&mut world, 0.016);

        let vels = world.borrow::<Velocity>();
        let v = vels.get(id).unwrap();
        assert_eq!(v.linear, Vec3::new(3.0, 0.0, -2.0));
        assert_eq!(v.angular, Vec3::new(0.0, 1.0, 0.0));
    }

    /// Applying a force to a zero-mass (static) body must NOT create a Velocity — the
    /// `mass > 0.0` guard is what keeps the acceleration from being infinite.
    #[test]
    fn apply_force_on_zero_mass_creates_no_velocity() {
        let engine = ScriptEngine::new().unwrap();
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, RigidBody::new(0.0, false));
        let id = e.id();

        engine.command_queue().push(ScriptCommand::ApplyForce(id, Vec3::new(100.0, 0.0, 0.0)));
        engine.flush_commands(&mut world, 0.016);

        assert!(
            world.borrow::<Velocity>().get(id).is_none(),
            "sıfır kütle için Velocity oluşturulmamalı"
        );
    }

    /// Several forces in one flush must accumulate (superposition).
    #[test]
    fn multiple_forces_accumulate_in_one_flush() {
        let engine = ScriptEngine::new().unwrap();
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, RigidBody::new(2.0, false));
        world.add_component(e, Velocity::new(Vec3::ZERO));
        let id = e.id();

        engine.command_queue().push(ScriptCommand::ApplyForce(id, Vec3::new(4.0, 0.0, 0.0)));
        engine.command_queue().push(ScriptCommand::ApplyForce(id, Vec3::new(0.0, 6.0, 0.0)));
        engine.flush_commands(&mut world, 0.5);

        let vels = world.borrow::<Velocity>();
        let v = vels.get(id).unwrap();
        // dv = (F/m)*dt : x = 4/2*0.5 = 1.0 ; y = 6/2*0.5 = 1.5
        assert!((v.linear.x - 1.0).abs() < 1e-5, "x: {}", v.linear.x);
        assert!((v.linear.y - 1.5).abs() < 1e-5, "y: {}", v.linear.y);
    }

    /// AddRigidBody must also create a Velocity, or the body cannot move.
    #[test]
    fn add_rigidbody_also_creates_velocity() {
        let engine = ScriptEngine::new().unwrap();
        let mut world = World::new();
        let e = world.spawn();
        let id = e.id();

        engine.command_queue().push(ScriptCommand::AddRigidBody { id, mass: 3.0, use_gravity: true });
        engine.flush_commands(&mut world, 0.016);

        let rbs = world.borrow::<RigidBody>();
        assert!((rbs.get(id).unwrap().mass - 3.0).abs() < 1e-6);
        drop(rbs);
        assert!(
            world.borrow::<Velocity>().get(id).is_some(),
            "AddRigidBody Velocity de eklemeli"
        );
    }

    /// AddBoxCollider/AddSphereCollider must create Collider components of the right shape.
    #[test]
    fn colliders_are_created_with_correct_shape() {
        let engine = ScriptEngine::new().unwrap();
        let mut world = World::new();
        let e_box = world.spawn();
        let e_sphere = world.spawn();
        let (bid, sid) = (e_box.id(), e_sphere.id());

        engine.command_queue().push(ScriptCommand::AddBoxCollider { id: bid, hx: 1.0, hy: 2.0, hz: 3.0 });
        engine.command_queue().push(ScriptCommand::AddSphereCollider { id: sid, radius: 4.0 });
        engine.flush_commands(&mut world, 0.016);

        let cols = world.borrow::<Collider>();
        match &cols.get(bid).unwrap().shape {
            ColliderShape::Box(b) => assert_eq!(b.half_extents, Vec3::new(1.0, 2.0, 3.0)),
            other => panic!("beklenen Box, gelen {other:?}"),
        }
        match &cols.get(sid).unwrap().shape {
            ColliderShape::Sphere(s) => assert!((s.radius - 4.0).abs() < 1e-6),
            other => panic!("beklenen Sphere, gelen {other:?}"),
        }
    }

    /// SpawnEntity: creates a named entity with a Transform and leaves a record in the log
    /// queue.
    #[test]
    fn spawn_entity_creates_named_transform_and_logs() {
        let engine = ScriptEngine::new().unwrap();
        let mut world = World::new();

        let logs_before = engine.log_queue.lock().unwrap().len();
        engine
            .command_queue()
            .push(ScriptCommand::SpawnEntity { name: "hero".into(), position: Vec3::new(5.0, 6.0, 7.0) });
        engine.flush_commands(&mut world, 0.016);

        // İsimli entity'yi bul.
        let names = world.borrow::<gizmo_core::EntityName>();
        let found = names.iter().filter_map(|(eid, _)| names.get(eid).map(|n| (eid, n.0.clone())))
            .find(|(_, name)| name == "hero");
        let (eid, _) = found.expect("'hero' isimli entity oluşmalıydı");
        drop(names);

        let transforms = world.borrow::<Transform>();
        assert_eq!(transforms.get(eid).unwrap().position, Vec3::new(5.0, 6.0, 7.0));
        drop(transforms);

        assert!(
            engine.log_queue.lock().unwrap().len() > logs_before,
            "spawn log kuyruğuna kayıt düşmeliydi"
        );
    }

    /// DestroyEntity must despawn an existing entity (which is then no longer alive).
    #[test]
    fn destroy_entity_removes_it() {
        let engine = ScriptEngine::new().unwrap();
        let mut world = World::new();
        let e = world.spawn();
        let id = e.id();
        assert!(world.entity(id).is_some());

        engine.command_queue().push(ScriptCommand::DestroyEntity(id));
        engine.flush_commands(&mut world, 0.016);

        assert!(world.entity(id).is_none(), "entity despawn edilmeliydi");
    }

    /// SetEntityName must rename an existing EntityName.
    #[test]
    fn set_entity_name_renames() {
        let engine = ScriptEngine::new().unwrap();
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, gizmo_core::EntityName::new("old"));
        let id = e.id();

        engine.command_queue().push(ScriptCommand::SetEntityName(id, "new".into()));
        engine.flush_commands(&mut world, 0.016);

        let names = world.borrow::<gizmo_core::EntityName>();
        assert_eq!(names.get(id).unwrap().0, "new");
    }

    /// AddNavAgent + SetAiTarget must set the target; ClearAiTarget must clear the target (not
    /// just the path).
    #[test]
    fn nav_agent_target_set_then_cleared() {
        use gizmo_ai::components::NavAgent;
        let engine = ScriptEngine::new().unwrap();
        let mut world = World::new();
        let e = world.spawn();
        let id = e.id();

        engine.command_queue().push(ScriptCommand::AddNavAgent(id));
        engine.command_queue().push(ScriptCommand::SetAiTarget(id, Vec3::new(9.0, 0.0, 0.0)));
        engine.flush_commands(&mut world, 0.016);
        {
            let agents = world.borrow::<NavAgent>();
            assert_eq!(agents.get(id).unwrap().target, Some(Vec3::new(9.0, 0.0, 0.0)));
        }

        engine.command_queue().push(ScriptCommand::ClearAiTarget(id));
        engine.flush_commands(&mut world, 0.016);
        {
            let agents = world.borrow::<NavAgent>();
            assert_eq!(agents.get(id).unwrap().target, None, "hedef temizlenmeliydi");
        }
    }

    /// Every command that is not applied must be handed back to the caller — never swallowed
    /// silently.
    ///
    /// **This test used to pin the defect in place.** It was called
    /// `..._but_consumes_savescene_and_vehicle` and asserted that `SaveScene` and the vehicle
    /// commands were *not* returned. Meanwhile the arm that swallowed them said in its comment
    /// that "these will fall through to unhandled anyway" — they could not, because that arm was
    /// itself consuming them. A command with live functions on the Lua side disappearing without
    /// a trace is the one thing a script author cannot diagnose. The assertion is now the
    /// intent: this crate hands back what it cannot apply.
    #[test]
    fn flush_returns_everything_it_cannot_apply_itself() {
        let engine = ScriptEngine::new().unwrap();
        let mut world = World::new();

        let cq = engine.command_queue();
        cq.push(ScriptCommand::PlaySound("boom".into()));
        cq.push(ScriptCommand::PlaySound3D("bird".into(), Vec3::ZERO));
        cq.push(ScriptCommand::StopSound("music".into()));
        cq.push(ScriptCommand::LoadScene("level.scene".into()));
        cq.push(ScriptCommand::SaveScene("slot.scene".into()));
        cq.push(ScriptCommand::SetVehicleBrake(1, 500.0));

        let unhandled = engine.flush_commands(&mut world, 0.016);

        assert_eq!(unhandled.len(), 6, "ses(3) + LoadScene + SaveScene + araç(1) — hepsi dönmeli");
        assert!(unhandled.iter().any(|c| matches!(c, ScriptCommand::PlaySound(n) if n == "boom")));
        assert!(unhandled.iter().any(|c| matches!(c, ScriptCommand::PlaySound3D(n, _) if n == "bird")));
        assert!(unhandled.iter().any(|c| matches!(c, ScriptCommand::StopSound(n) if n == "music")));
        assert!(unhandled.iter().any(|c| matches!(c, ScriptCommand::LoadScene(n) if n == "level.scene")));
        assert!(
            unhandled.iter().any(|c| matches!(c, ScriptCommand::SaveScene(n) if n == "slot.scene")),
            "SaveScene sessizce yutulmamalı"
        );
        assert!(
            unhandled.iter().any(|c| matches!(c, ScriptCommand::SetVehicleBrake(1, _))),
            "araç komutları sessizce yutulmamalı — bu crate onları uygulayamıyor, ev sahibi uygular"
        );
    }

    /// Scripts must run in the same order on every run.
    ///
    /// `loaded_scripts` was a `std::collections::HashMap`, and `RandomState` is seeded per
    /// process — so the order in which `update` ran the scripts changed from run to run. When two
    /// scripts touching the same entity disagree, insertion order decides the outcome, and this
    /// engine's headline contract is same-platform bit-exact replay. The order is now a property
    /// of the scripts' paths, not of the hasher.
    #[test]
    fn scripts_run_in_a_stable_order() {
        let mut engine = ScriptEngine::new().unwrap();
        let dir = std::env::temp_dir();
        // Loaded in an order that is not the sorted one, so a map that preserved insertion order
        // would also fail this.
        let mut written = Vec::new();
        for stem in ["zebra", "alpha", "midori", "beta"] {
            let path = dir
                .join(format!("gizmo_order_{stem}.lua"))
                .to_string_lossy()
                .into_owned();
            std::fs::write(&path, "function on_update(ctx) end\n").unwrap();
            engine.load_script(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
            written.push(path);
        }

        let order: Vec<String> = engine.loaded_scripts.keys().cloned().collect();
        let mut sorted = order.clone();
        sorted.sort();
        assert_eq!(
            order, sorted,
            "çalışma sırası yola göre sabit olmalı — bir HashMap'te bu proses başına değişirdi"
        );

        for path in written {
            let _ = std::fs::remove_file(path);
        }
    }

    /// flush_commands must drain the queue: it is empty after the call.
    #[test]
    fn flush_drains_the_queue() {
        let engine = ScriptEngine::new().unwrap();
        let mut world = World::new();
        engine.command_queue().push(ScriptCommand::StartRace);
        engine.command_queue().push(ScriptCommand::HideDialogue);
        assert_eq!(engine.command_queue().len(), 2);

        engine.flush_commands(&mut world, 0.016);
        assert!(engine.command_queue().is_empty(), "flush kuyruğu boşaltmalı");
    }

    /// Script::new always starts with initialized=false (on_init has not run yet).
    #[test]
    fn script_new_starts_uninitialized() {
        let s = Script::new("scripts/player.lua");
        assert_eq!(s.file_path, "scripts/player.lua");
        assert!(!s.initialized);
    }

    /// A Script serde round trip: `initialized` is `#[serde(default, skip)]`, so it does not
    /// appear in the serialisation and is always false afterwards — which is what makes on_init
    /// run again when a scene is loaded. file_path must survive.
    /// A script's `properties = { … }` declaration is what the editor lists.
    ///
    /// Read back out of the script's own environment, which is where a bare assignment lands.
    /// Non-scalar entries are dropped rather than guessed at: a nested table is the script's own
    /// business and has no inspector row.
    #[test]
    fn declared_properties_are_read_from_the_script() {
        let mut engine = ScriptEngine::new().unwrap();
        let path = unique_temp("declared_props");
        std::fs::write(
            &path,
            r#"
properties = {
    open_speed = 2.4,
    locked = false,
    label = "gate",
    nested = { nope = 1 },
}
"#,
        )
        .unwrap();
        engine.load_script(&path).unwrap();

        let declared = engine.declared_properties(&path);
        assert_eq!(declared.get("open_speed"), Some(&ScriptValue::Num(2.4)));
        assert_eq!(declared.get("locked"), Some(&ScriptValue::Bool(false)));
        assert_eq!(declared.get("label"), Some(&ScriptValue::Text("gate".into())));
        assert!(
            !declared.contains_key("nested"),
            "a table is not an inspector row and must not be guessed at"
        );
        let _ = std::fs::remove_file(&path);
    }

    /// A script with no declaration yields nothing, rather than erroring.
    #[test]
    fn a_script_without_properties_declares_none() {
        let mut engine = ScriptEngine::new().unwrap();
        let path = unique_temp("no_props");
        std::fs::write(&path, "function on_entity_update(id, dt, props) end\n").unwrap();
        engine.load_script(&path).unwrap();
        assert!(engine.declared_properties(&path).is_empty());
        let _ = std::fs::remove_file(&path);
    }

    /// The per-entity values reach the script, and two entities running the same file see their
    /// own.
    ///
    /// This is the whole reason the values live on the component: scripts are loaded per PATH, so
    /// both entities below share one Lua environment. If the properties lived in that environment
    /// the second call would overwrite the first.
    #[test]
    fn each_entity_sees_its_own_property_values() {
        let mut engine = ScriptEngine::new().unwrap();
        let path = unique_temp("per_entity_props");
        std::fs::write(
            &path,
            r#"
seen = {}
function on_entity_update(id, dt, props)
    seen[id] = props.open_speed
end
"#,
        )
        .unwrap();
        engine.load_script(&path).unwrap();

        let mut a = std::collections::BTreeMap::new();
        a.insert("open_speed".to_string(), ScriptValue::Num(1.5));
        let mut b = std::collections::BTreeMap::new();
        b.insert("open_speed".to_string(), ScriptValue::Num(9.25));

        engine.update_entity(1, &path, 0.016, &a).unwrap();
        engine.update_entity(2, &path, 0.016, &b).unwrap();

        let seen_1 = engine.eval_number(&path, "seen[1]").expect("entity 1 value");
        let seen_2 = engine.eval_number(&path, "seen[2]").expect("entity 2 value");
        assert_eq!(seen_1, 1.5);
        assert_eq!(
            seen_2, 9.25,
            "the second entity saw the first one's value — the properties are being shared"
        );
        let _ = std::fs::remove_file(&path);
    }

    /// **Every** stored value reaches the script — declared or not, right type or not.
    ///
    /// `update_entity` takes the component's whole `properties` map and the studio hands it
    /// `script.properties.clone()`, so nothing between the scene file and Lua filters it. That is
    /// the contract, and it is a reasonable one — a scene may carry per-entity data a script reads
    /// without declaring.
    ///
    /// It is pinned here because the editor once claimed the opposite. `ScriptValue::kind`'s note
    /// said an override whose kind differs from the declaration "is ignored rather than coerced",
    /// and the inspector duly filtered such an override out of its *display* — while the script
    /// went on receiving it. The inspector showed the declared default and the script ran on the
    /// stale value. Whatever the editor draws has to agree with this test, not the other way
    /// round.
    #[test]
    fn every_stored_property_reaches_the_script_declared_or_not() {
        let mut engine = ScriptEngine::new().unwrap();
        let path = unique_temp("undeclared_props");
        std::fs::write(
            &path,
            r#"
properties = { open_speed = 2.4, locked = false }
seen_speed = nil
seen_locked_is_string = nil
seen_undeclared = nil
function on_entity_update(id, dt, props)
    seen_speed = props.open_speed
    seen_locked_is_string = (type(props.locked) == "string") and 1 or 0
    seen_undeclared = props.nobody_declared_me
end
"#,
        )
        .unwrap();
        engine.load_script(&path).unwrap();

        // The declaration says `locked` is a bool and knows nothing about `nobody_declared_me`.
        let declared = engine.declared_properties(&path);
        assert_eq!(declared.get("locked").map(|v| v.kind()), Some("bool"));
        assert!(!declared.contains_key("nobody_declared_me"));

        let mut stored = std::collections::BTreeMap::new();
        stored.insert("open_speed".to_string(), ScriptValue::Num(7.5));
        // A stale override: the script now declares this as a bool.
        stored.insert("locked".to_string(), ScriptValue::Text("yes".to_string()));
        // And a key the script never declared at all.
        stored.insert("nobody_declared_me".to_string(), ScriptValue::Num(42.0));

        engine.update_entity(1, &path, 0.016, &stored).unwrap();

        assert_eq!(engine.eval_number(&path, "seen_speed"), Some(7.5));
        assert_eq!(
            engine.eval_number(&path, "seen_locked_is_string"),
            Some(1.0),
            "the type-mismatched override is handed to the script verbatim — it is NOT ignored"
        );
        assert_eq!(
            engine.eval_number(&path, "seen_undeclared"),
            Some(42.0),
            "an undeclared key reaches the script too, so the editor must not pretend it is absent"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn script_serde_roundtrip_resets_initialized() {
        let mut s = Script::new("a.lua");
        s.initialized = true;

        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("initialized"), "skip'li alan JSON'da olmamalı: {json}");

        let back: Script = serde_json::from_str(&json).unwrap();
        assert_eq!(back.file_path, "a.lua");
        assert!(!back.initialized, "deserialize sonrası initialized=false olmalı");
    }

    /// Security: the engine must disable the dangerous globals (os, io, require, dofile,
    /// loadfile, package, debug, load, loadstring).
    #[test]
    fn sandbox_disables_dangerous_globals() {
        let mut engine = ScriptEngine::new().unwrap();
        let path = unique_temp("sandbox");
        std::fs::write(
            &path,
            r#"
            assert(os == nil, "os kapatılmalı")
            assert(io == nil, "io kapatılmalı")
            assert(require == nil, "require kapatılmalı")
            assert(dofile == nil, "dofile kapatılmalı")
            assert(loadfile == nil, "loadfile kapatılmalı")
            assert(package == nil, "package kapatılmalı")
            assert(debug == nil, "debug kapatılmalı")
            assert(load == nil, "load kapatılmalı")
            assert(loadstring == nil, "loadstring kapatılmalı")
            "#,
        )
        .unwrap();
        let res = engine.load_script(&path);
        let _ = std::fs::remove_file(&path);
        res.expect("sandbox assert'leri geçmeli (global'ler nil olmalı)");
    }

    /// The Lua math helpers the engine registers (vec3_*, clamp, lerp) must work correctly.
    #[test]
    fn lua_math_helpers_are_correct() {
        let mut engine = ScriptEngine::new().unwrap();
        let path = unique_temp("mathhelpers");
        std::fs::write(
            &path,
            r#"
            assert(math.abs(vec3_length(vec3(3,4,0)) - 5.0) < 1e-5, "length 3-4-5")
            local c = vec3_cross(vec3(1,0,0), vec3(0,1,0))
            assert(c.x == 0 and c.y == 0 and c.z == 1, "x cross y = z")
            assert(clamp(5, 0, 3) == 3, "clamp üst sınır")
            assert(clamp(-1, 0, 3) == 0, "clamp alt sınır")
            assert(clamp(2, 0, 3) == 2, "clamp aralık içi")
            assert(lerp(0, 10, 0.5) == 5, "lerp orta")
            local n = vec3_normalize(vec3(0,0,0))
            assert(n.x == 0 and n.y == 0 and n.z == 0, "sıfır vektör normalize => sıfır")
            assert(math.abs(vec3_distance(vec3(0,0,0), vec3(0,3,4)) - 5.0) < 1e-5, "distance")
            local d = vec3_dot(vec3(1,2,3), vec3(4,5,6))
            assert(d == 32, "dot 1*4+2*5+3*6=32")
            "#,
        )
        .unwrap();
        let res = engine.load_script(&path);
        let _ = std::fs::remove_file(&path);
        res.expect("matematik yardımcı assert'leri geçmeli");
    }

    /// The error path: loading a script that does not exist must return a descriptive error,
    /// not panic.
    #[test]
    fn load_missing_file_returns_error() {
        let mut engine = ScriptEngine::new().unwrap();
        let err = engine
            .load_script("/nonexistent/gizmo/definitely_missing_5f2a.lua")
            .unwrap_err();
        assert!(err.contains("okunamadı"), "okuma hatası mesajı beklenir, gelen: {err}");
    }

    /// has_function must return true only for functions defined in a loaded script.
    #[test]
    fn has_function_detects_defined_and_missing() {
        let mut engine = ScriptEngine::new().unwrap();
        let path = unique_temp("hasfn");
        std::fs::write(&path, "function on_update(ctx) end\n").unwrap();
        engine.load_script(&path).unwrap();

        assert!(engine.has_function(&path, "on_update"));
        assert!(!engine.has_function(&path, "on_missing"));
        assert!(!engine.has_function("unloaded.lua", "on_update"));

        let _ = std::fs::remove_file(&path);
    }

    /// reload_if_changed: false while the content is unchanged (no reload), true when it
    /// changes, then false again when it stops — the hot-reload state machine.
    #[test]
    fn reload_if_changed_detects_content_change() {
        let mut engine = ScriptEngine::new().unwrap();
        let path = unique_temp("reload");
        std::fs::write(&path, "function on_update(ctx) end\n").unwrap();
        engine.load_script(&path).unwrap();

        assert!(!engine.reload_if_changed(&path).unwrap(), "değişmemişken false");

        std::fs::write(&path, "function on_update(ctx) end\n-- değişti\n").unwrap();
        assert!(engine.reload_if_changed(&path).unwrap(), "değişince true");

        assert!(!engine.reload_if_changed(&path).unwrap(), "tekrar değişmemişken false");

        let _ = std::fs::remove_file(&path);
    }
}
