use mlua::prelude::*;
use std::collections::HashMap;
use std::sync::Arc;
use gizmo_core::World;
use gizmo_core::input::Input;

use crate::commands::{CommandQueue, ScriptCommand};
use crate::api_entity;
use crate::api_input;
use crate::api_physics;
use crate::api_scene;
use crate::api_audio;
use crate::api_time;

/// Lua Scripting Motoru — Genişletilmiş API ile oyun mantığını yönetir
pub struct ScriptEngine {
    lua: Lua,
    loaded_scripts: HashMap<String, String>, // dosya_yolu -> script içeriği
    command_queue: Arc<CommandQueue>,
    elapsed_time: f32,
}

/// ECS Componenti: Varlığın üzerine hangi Lua script'inin takılı olduğunu tutar
#[derive(Clone, Debug)]
pub struct Script {
    pub file_path: String,
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
        
        // === SANDBOX: Tehlikeli modülleri kapat ===
        lua.globals().set("os", LuaNil)?;
        lua.globals().set("io", LuaNil)?;
        lua.globals().set("loadfile", LuaNil)?;
        lua.globals().set("dofile", LuaNil)?;
        
        // === TEMEL PRINT FONKSİYONU ===
        lua.globals().set("print_engine", lua.create_function(|_, msg: String| {
            println!("[Lua] {}", msg);
            Ok(())
        })?)?;

        // Orijinal print'i de engine çıktısına yönlendir
        lua.globals().set("print", lua.create_function(|_, values: LuaMultiValue| {
            let parts: Vec<String> = values.iter().map(|v| format!("{:?}", v)).collect();
            println!("[Lua] {}", parts.join("\t"));
            Ok(())
        })?)?;
        
        // === VEC3 YARDIMCI FONKSİYONLARI ===
        lua.load(r#"
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
        "#).exec()?;
        
        // === API MODÜLLERİNİ KAYDET ===
        api_entity::register_entity_api(&lua, command_queue.clone())?;
        api_input::register_input_api(&lua)?;
        api_physics::register_physics_api(&lua, command_queue.clone())?;
        api_scene::register_scene_api(&lua, command_queue.clone())?;
        api_audio::register_audio_api(&lua, command_queue.clone())?;
        api_time::register_time_api(&lua)?;
        
        Ok(Self {
            lua,
            loaded_scripts: HashMap::new(),
            command_queue,
            elapsed_time: 0.0,
        })
    }

    /// Lua script dosyasını diskten yükler ve önbelleğe alır
    pub fn load_script(&mut self, path: &str) -> Result<(), String> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("Script okunamadı {}: {}", path, e))?;
        
        // Script'i Lua VM'e yükle ve çalıştır
        self.lua.load(&content).exec()
            .map_err(|e| format!("Lua hata {}: {}", path, e))?;
        
        self.loaded_scripts.insert(path.to_string(), content);
        println!("🔧 ScriptEngine: Yüklendi → {}", path);
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
        
        // 2. on_update callback'ini çağır (varsa)
        let globals = self.lua.globals();
        if let Ok(func) = globals.get::<_, LuaFunction>("on_update") {
            let ctx_table = self.lua.create_table().map_err(|e| e.to_string())?;
            ctx_table.set("dt", dt).map_err(|e| e.to_string())?;
            ctx_table.set("elapsed", self.elapsed_time).map_err(|e| e.to_string())?;
            
            func.call::<_, ()>(ctx_table)
                .map_err(|e| format!("Lua on_update hatası: {}", e))?;
        }
        
        Ok(())
    }

    /// Per-entity script güncelleme — Script component'i olan entity'ler için
    pub fn update_entity(&mut self, entity_id: u32, dt: f32) -> Result<(), String> {
        let globals = self.lua.globals();
        
        // on_entity_update(entity_id, dt) çağır (varsa)
        if let Ok(func) = globals.get::<_, LuaFunction>("on_entity_update") {
            func.call::<_, ()>((entity_id, dt))
                .map_err(|e| format!("Lua on_entity_update hatası (entity {}): {}", entity_id, e))?;
        }
        
        Ok(())
    }

    /// Komut kuyruğundaki tüm komutları World'e uygular ve oyun mantığı için kalan komutları döndürür
    pub fn flush_commands(&self, world: &mut World) -> Vec<ScriptCommand> {
        let commands = self.command_queue.drain();
        let mut unhandled = Vec::new();
        
        for cmd in commands {
            match cmd {
                ScriptCommand::SetPosition(id, pos) => {
                    if let Some(mut transforms) = world.borrow_mut::<gizmo_physics::components::Transform>() {
                        if let Some(t) = transforms.get_mut(id) {
                            t.position = pos;
                        }
                    }
                }
                ScriptCommand::SetRotation(id, rot) => {
                    if let Some(mut transforms) = world.borrow_mut::<gizmo_physics::components::Transform>() {
                        if let Some(t) = transforms.get_mut(id) {
                            t.rotation = rot;
                        }
                    }
                }
                ScriptCommand::SetScale(id, scale) => {
                    if let Some(mut transforms) = world.borrow_mut::<gizmo_physics::components::Transform>() {
                        if let Some(t) = transforms.get_mut(id) {
                            t.scale = scale;
                        }
                    }
                }
                ScriptCommand::SetVelocity(id, vel) => {
                    if let Some(mut velocities) = world.borrow_mut::<gizmo_physics::components::Velocity>() {
                        if let Some(v) = velocities.get_mut(id) {
                            v.linear = vel;
                        }
                    }
                }
                ScriptCommand::SetAngularVelocity(id, ang_vel) => {
                    if let Some(mut velocities) = world.borrow_mut::<gizmo_physics::components::Velocity>() {
                        if let Some(v) = velocities.get_mut(id) {
                            v.angular = ang_vel;
                        }
                    }
                }
                ScriptCommand::ApplyForce(id, force) => {
                    let dt = 1.0 / 60.0;
                    if let Some(rbs) = world.borrow::<gizmo_physics::components::RigidBody>() {
                        if let Some(rb) = rbs.get(id) {
                            if rb.mass > 0.0 {
                                let accel = force * (1.0 / rb.mass);
                                drop(rbs);
                                if let Some(mut vels) = world.borrow_mut::<gizmo_physics::components::Velocity>() {
                                    if let Some(v) = vels.get_mut(id) {
                                        v.linear += accel * dt;
                                    }
                                }
                            }
                        }
                    }
                }
                ScriptCommand::ApplyImpulse(id, impulse) => {
                    if let Some(rbs) = world.borrow::<gizmo_physics::components::RigidBody>() {
                        if let Some(rb) = rbs.get(id) {
                            if rb.mass > 0.0 {
                                let delta_v = impulse * (1.0 / rb.mass);
                                drop(rbs);
                                if let Some(mut vels) = world.borrow_mut::<gizmo_physics::components::Velocity>() {
                                    if let Some(v) = vels.get_mut(id) {
                                        v.linear += delta_v;
                                    }
                                }
                            }
                        }
                    }
                }
                ScriptCommand::AddRigidBody { id, mass, restitution, friction, use_gravity } => {
                    let entity = world.iter_alive_entities().into_iter().find(|e| e.id() == id);
                    if let Some(e) = entity {
                        let rb = gizmo_physics::components::RigidBody::new(mass, restitution, friction, use_gravity);
                        world.add_component(e, rb);
                        // Make sure velocity exists so it can move
                        if world.borrow::<gizmo_physics::components::Velocity>().map(|v| v.get(id).is_none()).unwrap_or(true) {
                            world.add_component(e, gizmo_physics::components::Velocity::new(gizmo_math::Vec3::ZERO));
                        }
                    }
                }
                ScriptCommand::AddBoxCollider { id, hx, hy, hz } => {
                    let entity = world.iter_alive_entities().into_iter().find(|e| e.id() == id);
                    if let Some(e) = entity {
                        let col = gizmo_physics::shape::Collider::new_aabb(hx, hy, hz);
                        world.add_component(e, col);
                    }
                }
                ScriptCommand::AddSphereCollider { id, radius } => {
                    let entity = world.iter_alive_entities().into_iter().find(|e| e.id() == id);
                    if let Some(e) = entity {
                        let col = gizmo_physics::shape::Collider::new_sphere(radius);
                        world.add_component(e, col);
                    }
                }

                ScriptCommand::SpawnEntity { name, position } => {
                    let entity = world.spawn();
                    world.add_component(entity, gizmo_core::EntityName::new(&name));
                    world.add_component(entity, gizmo_physics::components::Transform::new(position));
                    println!("[Lua] Entity spawn: '{}' at ({:.1}, {:.1}, {:.1})", name, position.x, position.y, position.z);
                }
                ScriptCommand::SpawnPrefab { name, prefab_type, position } => {
                    let entity = world.spawn();
                    world.add_component(entity, gizmo_core::EntityName::new(&name));
                    world.add_component(entity, gizmo_physics::components::Transform::new(position));
                    world.add_component(entity, gizmo_core::PrefabRequest(prefab_type.clone()));
                }
                ScriptCommand::DestroyEntity(id) => {
                    world.despawn_by_id(id);
                    println!("[Lua] Entity destroyed: {}", id);
                }
                ScriptCommand::SetEntityName(id, name) => {
                    if let Some(mut names) = world.borrow_mut::<gizmo_core::EntityName>() {
                        if let Some(n) = names.get_mut(id) {
                            n.0 = name;
                        }
                    }
                }
                ScriptCommand::SaveScene(_) |
                ScriptCommand::ShowDialogue { .. } | ScriptCommand::HideDialogue |
                ScriptCommand::TriggerCutscene(_) | ScriptCommand::EndCutscene |
                ScriptCommand::AddCheckpoint { .. } | ScriptCommand::ActivateCheckpoint(_) |
                ScriptCommand::StartRace | ScriptCommand::FinishRace { .. } | ScriptCommand::ResetRace |
                ScriptCommand::SetCameraTarget(_) | ScriptCommand::SetCameraFov(_) => {
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
        let current = std::fs::read_to_string(path)
            .map_err(|e| format!("Script okunamadı: {}", e))?;
        
        if let Some(cached) = self.loaded_scripts.get(path) {
            if *cached == current {
                return Ok(false);
            }
        }
        
        self.load_script(path)?;
        Ok(true)
    }

    /// Belirli bir isimdeki Lua fonksiyonunun var olup olmadığını kontrol eder
    pub fn has_function(&self, name: &str) -> bool {
        self.lua.globals().get::<_, LuaFunction>(name).is_ok()
    }

    /// Belirli bir isimdeki Lua fonksiyonunu çağırır (per-entity scriptler için)
    pub fn run_entity_update(&self, func_name: &str, ctx: &ScriptContext) -> Result<ScriptResult, String> {
        let globals = self.lua.globals();
        
        let func: LuaFunction = match globals.get(func_name) {
            Ok(f) => f,
            Err(_) => return Ok(ScriptResult::default()),
        };

        let ctx_table = self.lua.create_table().map_err(|e| e.to_string())?;
        ctx_table.set("entity_id", ctx.entity_id).map_err(|e| e.to_string())?;
        ctx_table.set("dt", ctx.dt).map_err(|e| e.to_string())?;
        ctx_table.set("elapsed", self.elapsed_time).map_err(|e| e.to_string())?;
        
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
        input.set("space", ctx.key_space).map_err(|e| e.to_string())?;
        input.set("up", ctx.key_up).map_err(|e| e.to_string())?;
        input.set("down", ctx.key_down).map_err(|e| e.to_string())?;
        input.set("left", ctx.key_left).map_err(|e| e.to_string())?;
        input.set("right", ctx.key_right).map_err(|e| e.to_string())?;
        ctx_table.set("input", input).map_err(|e| e.to_string())?;

        let result_table: LuaTable = func.call(ctx_table).map_err(|e| format!("Lua runtime: {}", e))?;
        
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
