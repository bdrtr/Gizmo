use gizmo_core::input::Input;
use gizmo_core::World;
use mlua::prelude::*;
use mlua::RegistryKey;
use std::collections::HashMap;
use std::sync::Arc;

use crate::api_ai;
use crate::api_audio;
use crate::api_entity;
use crate::api_input;
use crate::api_physics;
use crate::api_scene;
use crate::api_time;
use crate::api_vehicle;
use crate::commands::{CommandQueue, ScriptCommand};

/// Lua Scripting Motoru — Genişletilmiş API ile oyun mantığını yönetir
pub struct ScriptEngine {
    lua: Lua,
    loaded_scripts: HashMap<String, (String, RegistryKey)>,
    command_queue: Arc<CommandQueue>,
    elapsed_time: f32,
    pub log_queue: Arc<std::sync::Mutex<Vec<(String, String)>>>, // (Level, Message)
}

unsafe impl Send for ScriptEngine {}
unsafe impl Sync for ScriptEngine {}

/// ECS Componenti: Varlığın üzerine hangi Lua script'inin takılı olduğunu tutar
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Script {
    pub file_path: String,
    #[serde(default, skip)]
    pub initialized: bool, // on_init çağrıldı mı?
}

impl Script {
    pub fn new(path: &str) -> Self {
        Self {
            file_path: path.to_string(),
            initialized: false,
        }
    }
}

/// Lua'ya geçirilecek entity verisi (geriye dönük uyumluluk için)
#[derive(Clone, Debug)]
pub struct ScriptContext {
    pub entity_id: u32,
    pub dt: f32,
    pub position: [f32; 3],
    pub velocity: [f32; 3],
    pub key_w: bool,
    pub key_a: bool,
    pub key_s: bool,
    pub key_d: bool,
    pub key_space: bool,
    pub key_up: bool,
    pub key_down: bool,
    pub key_left: bool,
    pub key_right: bool,
}

/// Lua'dan dönen değişiklikler (geriye dönük uyumluluk)
#[derive(Clone, Debug, Default)]
pub struct ScriptResult {
    pub new_position: Option<[f32; 3]>,
    pub new_velocity: Option<[f32; 3]>,
}

impl ScriptEngine {
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
                            s.to_str().unwrap_or("").to_string()
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
        api_input::register_input_api(&lua)?;
        api_physics::register_physics_api(&lua, command_queue.clone())?;
        api_scene::register_scene_api(&lua, command_queue.clone())?;
        api_audio::register_audio_api(&lua, command_queue.clone())?;
        api_time::register_time_api(&lua)?;
        api_vehicle::register_vehicle_api(&lua, command_queue.clone())?;
        api_ai::register_ai_api(&lua, command_queue.clone())?;

        Ok(Self {
            lua,
            loaded_scripts: HashMap::new(),
            command_queue,
            elapsed_time: 0.0,
            log_queue,
        })
    }

    pub fn load_script(&mut self, path: &str) -> Result<(), String> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("Script okunamadı {}: {}", path, e))?;

        let env = self.lua.create_table().map_err(|e| e.to_string())?;

        // Link to _G via metatable
        let meta = self.lua.create_table().map_err(|e| e.to_string())?;
        meta.set("__index", self.lua.globals()).unwrap();
        env.set_metatable(Some(meta));

        // Script'i İzole env içinde çalıştır
        self.lua
            .load(&content)
            .set_environment(env.clone())
            .exec()
            .map_err(|e| format!("Lua hata {}: {}", path, e))?;

        let key = self
            .lua
            .create_registry_value(env)
            .map_err(|e| e.to_string())?;

        // Replace existing key if it exists to free old memory
        if let Some((_, old_key)) = self.loaded_scripts.insert(path.to_string(), (content, key)) {
            let _ = self.lua.remove_registry_value(old_key);
        }

        tracing::info!("🔧 ScriptEngine: Yüklendi ve İzole Edildi → {}", path);
        Ok(())
    }

    /// Her frame çağrılan güncelleme — World verilerini Lua'ya aktarır, scriptleri çalıştırır
    pub fn update(&mut self, world: &World, input: &Input, dt: f32) -> Result<(), String> {
        self.elapsed_time += dt;

        // 1. World verilerini Lua'ya aktar (read snapshot)
        api_entity::update_entity_read_api(&self.lua, world)
            .map_err(|e| format!("Entity API güncelleme hatası: {}", e))?;
        api_input::update_input_api(&self.lua, input)
            .map_err(|e| format!("Input API güncelleme hatası: {}", e))?;
        api_scene::update_scene_api(&self.lua, world)
            .map_err(|e| format!("Scene API güncelleme hatası: {}", e))?;
        api_time::update_time_api(&self.lua, dt, self.elapsed_time, 1.0 / dt.max(0.0001))
            .map_err(|e| format!("Time API güncelleme hatası: {}", e))?;
        api_physics::update_physics_api(&self.lua, world)
            .map_err(|e| format!("Physics API güncelleme hatası: {}", e))?;

        // 2. on_update callback'ini çağır (varsa)
        let globals = self.lua.globals();
        if let Ok(func) = globals.get::<_, LuaFunction>("on_update") {
            let ctx_table = self.lua.create_table().map_err(|e| e.to_string())?;
            ctx_table.set("dt", dt).map_err(|e| e.to_string())?;
            ctx_table
                .set("elapsed", self.elapsed_time)
                .map_err(|e| e.to_string())?;

            func.call::<_, ()>(ctx_table)
                .map_err(|e| format!("Lua on_update hatası: {}", e))?;
        }

        Ok(())
    }

    /// Per-entity script güncelleme — Script component'i olan entity'ler için izole ortamda çalıştırır
    pub fn update_entity(
        &mut self,
        entity_id: u32,
        script_path: &str,
        dt: f32,
    ) -> Result<(), String> {
        if let Some((_, key)) = self.loaded_scripts.get(script_path) {
            let env: mlua::Table = self.lua.registry_value(key).map_err(|e| e.to_string())?;

            // on_entity_update(entity_id, dt) çağır (varsa)
            if let Ok(func) = env.get::<_, LuaFunction>("on_entity_update") {
                func.call::<_, ()>((entity_id, dt)).map_err(|e| {
                    format!(
                        "Lua on_entity_update hatası (entity {} mod {}): {}",
                        entity_id, script_path, e
                    )
                })?;
            }
        }
        Ok(())
    }

    /// Komut kuyruğundaki tüm komutları World'e uygular ve oyun mantığı için kalan komutları döndürür
    pub fn flush_commands(&self, world: &mut World, dt: f32) -> Vec<ScriptCommand> {
        let commands = self.command_queue.drain();
        let mut unhandled = Vec::new();

        for cmd in commands {
            match cmd {
                ScriptCommand::SetPosition(id, pos) => {
                    let mut transforms = world.borrow_mut::<gizmo_physics::components::Transform>();
                    if let Some(t) = transforms.get_mut(id) {
                        t.position = pos;
                    }
                }
                ScriptCommand::SetRotation(id, rot) => {
                    let mut transforms = world.borrow_mut::<gizmo_physics::components::Transform>();
                    if let Some(t) = transforms.get_mut(id) {
                        t.rotation = rot;
                    }
                }
                ScriptCommand::SetScale(id, scale) => {
                    let mut transforms = world.borrow_mut::<gizmo_physics::components::Transform>();
                    if let Some(t) = transforms.get_mut(id) {
                        t.scale = scale;
                    }
                }
                ScriptCommand::SetVelocity(id, vel) => {
                    let mut velocities = world.borrow_mut::<gizmo_physics::components::Velocity>();
                    if let Some(v) = velocities.get_mut(id) {
                        v.linear = vel;
                    }
                }
                ScriptCommand::SetAngularVelocity(id, ang_vel) => {
                    let mut velocities = world.borrow_mut::<gizmo_physics::components::Velocity>();
                    if let Some(v) = velocities.get_mut(id) {
                        v.angular = ang_vel;
                    }
                }
                ScriptCommand::ApplyForce(id, force) => {
                    let rbs = world.borrow::<gizmo_physics::components::RigidBody>();
                    if let Some(rb) = rbs.get(id) {
                        if rb.mass > 0.0 {
                            let accel = force * (1.0 / rb.mass);
                            drop(rbs);
                            let mut vels =
                                world.borrow_mut::<gizmo_physics::components::Velocity>();
                            if let Some(v) = vels.get_mut(id) {
                                v.linear += accel * dt;
                            }
                        }
                    }
                }
                ScriptCommand::ApplyImpulse(id, impulse) => {
                    let rbs = world.borrow::<gizmo_physics::components::RigidBody>();
                    if let Some(rb) = rbs.get(id) {
                        if rb.mass > 0.0 {
                            let delta_v = impulse * (1.0 / rb.mass);
                            drop(rbs);
                            let mut vels =
                                world.borrow_mut::<gizmo_physics::components::Velocity>();
                            if let Some(v) = vels.get_mut(id) {
                                v.linear += delta_v;
                            }
                        }
                    }
                }
                ScriptCommand::AddRigidBody {
                    id,
                    mass,
                    restitution,
                    friction,
                    use_gravity,
                } => {
                    let entity = world
                        .iter_alive_entities()
                        .into_iter()
                        .find(|e| e.id() == id);
                    if let Some(e) = entity {
                        let rb = gizmo_physics::components::RigidBody::new(
                            mass,
                            restitution,
                            friction,
                            use_gravity,
                        );
                        world.add_component(e, rb);
                        // Make sure velocity exists so it can move
                        if world
                            .borrow::<gizmo_physics::components::Velocity>()
                            .get(id)
                            .is_none()
                        {
                            world.add_component(
                                e,
                                gizmo_physics::components::Velocity::new(gizmo_math::Vec3::ZERO),
                            );
                        }
                    }
                }
                ScriptCommand::AddBoxCollider { id, hx, hy, hz } => {
                    let entity = world
                        .iter_alive_entities()
                        .into_iter()
                        .find(|e| e.id() == id);
                    if let Some(e) = entity {
                        let col =
                            gizmo_physics::shape::Collider::aabb(gizmo_math::Vec3::new(hx, hy, hz));
                        world.add_component(e, col);
                    }
                }
                ScriptCommand::AddSphereCollider { id, radius } => {
                    let entity = world
                        .iter_alive_entities()
                        .into_iter()
                        .find(|e| e.id() == id);
                    if let Some(e) = entity {
                        let col = gizmo_physics::shape::Collider::sphere(radius);
                        world.add_component(e, col);
                    }
                }

                ScriptCommand::SetVehicleEngineForce(_id, _force) => {}
                ScriptCommand::SetVehicleSteering(_id, _angle) => {}
                ScriptCommand::SetVehicleBrake(_id, _force) => {}

                ScriptCommand::SpawnEntity { name, position } => {
                    let entity = world.spawn();
                    world.add_component(entity, gizmo_core::EntityName::new(&name));
                    world
                        .add_component(entity, gizmo_physics::components::Transform::new(position));
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
                        .add_component(entity, gizmo_physics::components::Transform::new(position));
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
                    if let Some(n) = names.get_mut(id) {
                        n.0 = name;
                    }
                }
                ScriptCommand::AddNavAgent(id) => {
                    let entity = world
                        .iter_alive_entities()
                        .into_iter()
                        .find(|e| e.id() == id);
                    if let Some(e) = entity {
                        world.add_component(e, gizmo_ai::components::NavAgent::default());
                    }
                }
                ScriptCommand::SetAiTarget(id, target) => {
                    let mut agents = world.borrow_mut::<gizmo_ai::components::NavAgent>();
                    if let Some(agent) = agents.get_mut(id) {
                        agent.set_target(target);
                    }
                }
                ScriptCommand::ClearAiTarget(id) => {
                    let mut agents = world.borrow_mut::<gizmo_ai::components::NavAgent>();
                    if let Some(agent) = agents.get_mut(id) {
                        agent.clear_path();
                    }
                }
                ScriptCommand::SaveScene(_)
                | ScriptCommand::ShowDialogue { .. }
                | ScriptCommand::HideDialogue
                | ScriptCommand::TriggerCutscene(_)
                | ScriptCommand::EndCutscene
                | ScriptCommand::AddCheckpoint { .. }
                | ScriptCommand::ActivateCheckpoint(_)
                | ScriptCommand::StartRace
                | ScriptCommand::FinishRace { .. }
                | ScriptCommand::ResetRace
                | ScriptCommand::SetCameraTarget(_)
                | ScriptCommand::SetCameraFov(_) => {
                    // Ignored
                }
                other => {
                    unhandled.push(other);
                }
            }
        }

        unhandled
    }

    /// Runtime'da bekleyen ses/sahne komutlarını döndürür (demo tarafında ele alınır)
    pub fn get_pending_audio_scene_commands(&self) -> Vec<ScriptCommand> {
        // Flush zaten çağrıldıysa bu boş dönecek
        // Alternatif: flush'tan önce çağrılmalı
        Vec::new()
    }

    /// Script'in hot-reload edilip edilmeyeceğini kontrol eder
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

    /// Belirli bir isimdeki Lua fonksiyonunun var olup olmadığını kontrol eder
    pub fn has_function(&self, path: &str, name: &str) -> bool {
        if let Some((_, key)) = self.loaded_scripts.get(path) {
            if let Ok(env) = self.lua.registry_value::<mlua::Table>(key) {
                return env.get::<_, LuaFunction>(name).is_ok();
            }
        }
        false
    }

    /// Belirli bir isimdeki Lua fonksiyonunu çağırır (per-entity scriptler için)
    pub fn run_entity_update(
        &self,
        path: &str,
        func_name: &str,
        ctx: &ScriptContext,
    ) -> Result<ScriptResult, String> {
        let env: mlua::Table = if let Some((_, key)) = self.loaded_scripts.get(path) {
            self.lua.registry_value(key).map_err(|e| e.to_string())?
        } else {
            return Err(format!("Script not loaded: {}", path));
        };

        let func: LuaFunction = match env.get(func_name) {
            Ok(f) => f,
            Err(_) => return Ok(ScriptResult::default()),
        };

        let ctx_table = self.lua.create_table().map_err(|e| e.to_string())?;
        ctx_table
            .set("entity_id", ctx.entity_id)
            .map_err(|e| e.to_string())?;
        ctx_table.set("dt", ctx.dt).map_err(|e| e.to_string())?;
        ctx_table
            .set("elapsed", self.elapsed_time)
            .map_err(|e| e.to_string())?;

        let pos = self.lua.create_table().map_err(|e| e.to_string())?;
        pos.set("x", ctx.position[0]).map_err(|e| e.to_string())?;
        pos.set("y", ctx.position[1]).map_err(|e| e.to_string())?;
        pos.set("z", ctx.position[2]).map_err(|e| e.to_string())?;
        ctx_table.set("position", pos).map_err(|e| e.to_string())?;

        let vel = self.lua.create_table().map_err(|e| e.to_string())?;
        vel.set("x", ctx.velocity[0]).map_err(|e| e.to_string())?;
        vel.set("y", ctx.velocity[1]).map_err(|e| e.to_string())?;
        vel.set("z", ctx.velocity[2]).map_err(|e| e.to_string())?;
        ctx_table.set("velocity", vel).map_err(|e| e.to_string())?;

        let input = self.lua.create_table().map_err(|e| e.to_string())?;
        input.set("w", ctx.key_w).map_err(|e| e.to_string())?;
        input.set("a", ctx.key_a).map_err(|e| e.to_string())?;
        input.set("s", ctx.key_s).map_err(|e| e.to_string())?;
        input.set("d", ctx.key_d).map_err(|e| e.to_string())?;
        input
            .set("space", ctx.key_space)
            .map_err(|e| e.to_string())?;
        input.set("up", ctx.key_up).map_err(|e| e.to_string())?;
        input.set("down", ctx.key_down).map_err(|e| e.to_string())?;
        input.set("left", ctx.key_left).map_err(|e| e.to_string())?;
        input
            .set("right", ctx.key_right)
            .map_err(|e| e.to_string())?;
        ctx_table.set("input", input).map_err(|e| e.to_string())?;

        let result_table: LuaTable = func
            .call(ctx_table)
            .map_err(|e| format!("Lua runtime: {}", e))?;

        let mut result = ScriptResult::default();

        if let Ok(pos) = result_table.get::<_, LuaTable>("position") {
            let x: f32 = pos.get("x").unwrap_or(0.0);
            let y: f32 = pos.get("y").unwrap_or(0.0);
            let z: f32 = pos.get("z").unwrap_or(0.0);
            result.new_position = Some([x, y, z]);
        }

        if let Ok(vel) = result_table.get::<_, LuaTable>("velocity") {
            let x: f32 = vel.get("x").unwrap_or(0.0);
            let y: f32 = vel.get("y").unwrap_or(0.0);
            let z: f32 = vel.get("z").unwrap_or(0.0);
            result.new_velocity = Some([x, y, z]);
        }

        Ok(result)
    }

    /// Komut kuyruğuna doğrudan erişim (internals)
    pub fn command_queue(&self) -> &Arc<CommandQueue> {
        &self.command_queue
    }
}

gizmo_core::impl_component!(Script);
