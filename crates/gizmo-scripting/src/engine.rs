use gizmo_core::input::Input;
use gizmo_core::World;
use mlua::prelude::*;
use mlua::RegistryKey;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, error, info, trace, warn};

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

/// Lua Scripting Motoru — Genişletilmiş API ile oyun mantığını yönetir
pub struct ScriptEngine {
    lua: Lua,
    loaded_scripts: HashMap<String, (String, RegistryKey)>,
    command_queue: Arc<CommandQueue>,
    elapsed_time: f32,
    /// Log messages emitted from Lua (`print`), stored as `(level, message)` pairs.
    pub log_queue: Arc<std::sync::Mutex<Vec<(String, String)>>>, // (Level, Message)
}

unsafe impl Send for ScriptEngine {}
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
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
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
            loaded_scripts: HashMap::new(),
            command_queue,
            elapsed_time: 0.0,
            log_queue,
        })
    }

    #[tracing::instrument(skip_all, name = "script_load", fields(path = %path))]
    pub fn load_script(&mut self, path: &str) -> Result<(), String> {
        let content = std::fs::read_to_string(path).map_err(|e| {
            error!(path, error = %e, "[Scripting] Script dosyası okunamadı");
            format!("Script okunamadı {}: {}", path, e)
        })?;
        let byte_len = content.len();

        let env = self.lua.create_table().map_err(|e| e.to_string())?;

        // Link to _G via metatable
        let meta = self.lua.create_table().map_err(|e| e.to_string())?;
        meta.set("__index", self.lua.globals())
            .map_err(|e| e.to_string())?;
        env.set_metatable(Some(meta));

        // Script'i İzole env içinde çalıştır
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

    /// Her frame çağrılan güncelleme — World verilerini Lua'ya aktarır, scriptleri çalıştırır
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

        for (path, (_, key)) in &self.loaded_scripts {
            let env: mlua::Table = self.lua.registry_value(key).map_err(|e| e.to_string())?;
            if let Ok(func) = env.get::<_, LuaFunction>("on_update") {
                func.call::<_, ()>(ctx_table.clone()).map_err(|e| {
                    warn!(path = %path, error = %e, "[Scripting] on_update çalışma-zamanı hatası");
                    format!("Lua on_update hatası ({}): {}", path, e)
                })?;
            }
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

    /// Komut kuyruğundaki tüm komutları World'e uygular ve oyun mantığı için kalan komutları döndürür
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
                    let mut velocities = world.borrow_mut::<gizmo_physics_rigid::components::Velocity>();
                    if let Some(mut v) = velocities.get_mut(id) {
                        v.linear = vel;
                    } else {
                        trace!(entity = id, "[Scripting] SetVelocity: hedefte Velocity yok, komut atlandı");
                    }
                }
                ScriptCommand::SetAngularVelocity(id, ang_vel) => {
                    let mut velocities = world.borrow_mut::<gizmo_physics_rigid::components::Velocity>();
                    if let Some(mut v) = velocities.get_mut(id) {
                        v.angular = ang_vel;
                    } else {
                        trace!(entity = id, "[Scripting] SetAngularVelocity: hedefte Velocity yok, komut atlandı");
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
                            let mut vels =
                                world.borrow_mut::<gizmo_physics_rigid::components::Velocity>();
                            if let Some(mut v) = vels.get_mut(id) {
                                v.linear += accel * dt;
                            }
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
                            let mut vels =
                                world.borrow_mut::<gizmo_physics_rigid::components::Velocity>();
                            if let Some(mut v) = vels.get_mut(id) {
                                v.linear += delta_v;
                            }
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

                ScriptCommand::SetVehicleEngineForce(_id, _force) => {}
                ScriptCommand::SetVehicleSteering(_id, _angle) => {}
                ScriptCommand::SetVehicleBrake(_id, _force) => {}

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
                ScriptCommand::SetFighterMove { id, name, startup, active, recovery, damage } => {
                    let mut fighters = world.borrow_mut::<gizmo_physics_core::components::FighterController>();
                    if let Some(mut fighter) = fighters.get_mut(id) {
                        let mut frame_data =
                            gizmo_physics_core::components::fighter::FrameData::default();
                        frame_data.startup = startup;
                        frame_data.active = active;
                        frame_data.recovery = recovery;
                        frame_data.damage = damage;
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
                | ScriptCommand::SetCameraFov(_)
                | ScriptCommand::SetFightCamera { .. } => {
                    // Bu komutlar flush_commands'ın dönüş değerinde (unhandled) zaten yer alacak
                }
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
            Err(e) => {
                trace!(path, func_name, error = %e, "[Scripting] run_entity_update: fonksiyon alınamadı, varsayılan sonuç");
                return Ok(ScriptResult::default());
            }
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

        let result_table: LuaTable = func.call(ctx_table).map_err(|e| {
            warn!(path, func_name, error = %e, "[Scripting] run_entity_update: Lua çalışma-zamanı hatası");
            format!("Lua runtime: {}", e)
        })?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_math::{Quat, Vec3};
    use gizmo_physics_core::{Collider, ColliderShape, Transform};
    use gizmo_physics_rigid::components::{RigidBody, Velocity};

    /// Paralel test koşumlarında çakışmayan benzersiz geçici script yolu üretir.
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

    /// Regression: RigidBody var ama Velocity yoksa ApplyForce sessizce
    /// kaybolmamalı; Velocity oluşturulup ivme uygulanmalı.
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

    /// Regression: RigidBody var ama Velocity yoksa ApplyImpulse sessizce
    /// kaybolmamalı; Velocity oluşturulup delta-v uygulanmalı.
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

    /// Transform yazma komutları (SetPosition/SetScale/SetRotation) mevcut bir
    /// Transform'a uygulanmalı.
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

    /// SetVelocity/SetAngularVelocity mevcut Velocity'nin linear/angular alanlarını ayarlamalı.
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

    /// Kütlesi sıfır (statik) bir gövdeye kuvvet uygulanınca Velocity OLUŞTURULMAMALI —
    /// `mass > 0.0` koruması sonsuz ivmeyi engeller.
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

    /// Aynı flush içinde birden çok kuvvet birikimli (superposition) uygulanmalı.
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

    /// AddRigidBody hareket edebilmesi için beraberinde bir Velocity de oluşturmalı.
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

    /// AddBoxCollider/AddSphereCollider doğru şekilli Collider bileşenleri oluşturmalı.
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

    /// SpawnEntity: isimli, Transform'lu bir entity oluşturmalı ve log kuyruğuna kayıt düşmeli.
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

    /// DestroyEntity var olan bir entity'yi despawn etmeli (artık canlı olmamalı).
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

    /// SetEntityName mevcut EntityName'i yeniden adlandırmalı.
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

    /// AddNavAgent + SetAiTarget hedefi ayarlamalı; ClearAiTarget hedefi (yalnız yolu değil) temizlemeli.
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

    /// flush_commands ses ve LoadScene komutlarını demo katmanına 'unhandled' olarak
    /// döndürmeli; SaveScene ve araç komutlarını ise tüketmeli (döndürmemeli).
    #[test]
    fn flush_returns_audio_and_loadscene_but_consumes_savescene_and_vehicle() {
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

        assert_eq!(unhandled.len(), 4, "yalnız ses(3) + LoadScene(1) döndürülmeli");
        assert!(unhandled.iter().any(|c| matches!(c, ScriptCommand::PlaySound(n) if n == "boom")));
        assert!(unhandled.iter().any(|c| matches!(c, ScriptCommand::PlaySound3D(n, _) if n == "bird")));
        assert!(unhandled.iter().any(|c| matches!(c, ScriptCommand::StopSound(n) if n == "music")));
        assert!(unhandled.iter().any(|c| matches!(c, ScriptCommand::LoadScene(n) if n == "level.scene")));
        assert!(!unhandled.iter().any(|c| matches!(c, ScriptCommand::SaveScene(_))));
        assert!(!unhandled.iter().any(|c| matches!(c, ScriptCommand::SetVehicleBrake(..))));
    }

    /// flush_commands kuyruğu tüketmeli (drain): çağrı sonrası kuyruk boş olmalı.
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

    /// Script::new her zaman initialized=false ile başlar (on_init henüz çağrılmadı).
    #[test]
    fn script_new_starts_uninitialized() {
        let s = Script::new("scripts/player.lua");
        assert_eq!(s.file_path, "scripts/player.lua");
        assert!(!s.initialized);
    }

    /// Script serde round-trip: `initialized` alanı `#[serde(default, skip)]` olduğundan
    /// serileştirmede yer almaz ve deserialize sonrası daima false olur — böylece sahne
    /// yüklendiğinde on_init yeniden çalışır. file_path korunmalı.
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

    /// Güvenlik: motor tehlikeli global'leri (os/io/require/dofile/loadfile/package/
    /// debug/load/loadstring) devre dışı bırakmalı.
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

    /// Motorun kaydettiği Lua matematik yardımcıları (vec3_*, clamp, lerp) doğru çalışmalı.
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

    /// Hata yolu: var olmayan bir script yüklenince açıklayıcı bir hata dönmeli (panik değil).
    #[test]
    fn load_missing_file_returns_error() {
        let mut engine = ScriptEngine::new().unwrap();
        let err = engine
            .load_script("/nonexistent/gizmo/definitely_missing_5f2a.lua")
            .unwrap_err();
        assert!(err.contains("okunamadı"), "okuma hatası mesajı beklenir, gelen: {err}");
    }

    /// Hata yolu: yüklenmemiş bir script için run_entity_update 'not loaded' hatası vermeli.
    #[test]
    fn run_entity_update_on_unloaded_script_errors() {
        let engine = ScriptEngine::new().unwrap();
        let ctx = ScriptContext::default();
        let err = engine
            .run_entity_update("never_loaded.lua", "on_entity_update", &ctx)
            .unwrap_err();
        assert!(err.contains("not loaded"), "mesaj: {err}");
    }

    /// run_entity_update ctx'i (pozisyon + dt) Lua'ya geçirmeli ve dönen position tablosunu
    /// ScriptResult.new_position olarak çıkarmalı. Var olmayan fonksiyon default döndürmeli.
    #[test]
    fn run_entity_update_marshals_position_and_extracts_result() {
        let mut engine = ScriptEngine::new().unwrap();
        let path = unique_temp("marshal_pos");
        std::fs::write(
            &path,
            "function mv(ctx)\n  return { position = { x = ctx.position.x + ctx.dt, y = ctx.position.y, z = ctx.position.z } }\nend\n",
        )
        .unwrap();
        engine.load_script(&path).unwrap();

        let ctx = ScriptContext {
            entity_id: 42,
            dt: 0.5,
            position: [10.0, -1.0, 2.0],
            ..Default::default()
        };

        let result = engine.run_entity_update(&path, "mv", &ctx).unwrap();
        assert_eq!(result.new_position, Some([10.5, -1.0, 2.0]));
        assert_eq!(result.new_velocity, None, "script velocity döndürmedi");

        // Var olmayan fonksiyon → default (her ikisi None).
        let empty = engine.run_entity_update(&path, "yok_boyle_fn", &ctx).unwrap();
        assert_eq!(empty.new_position, None);
        assert_eq!(empty.new_velocity, None);

        let _ = std::fs::remove_file(&path);
    }

    /// run_entity_update girdi (input) bayraklarını Lua ctx.input'a geçirmeli; script
    /// bunlara göre velocity döndürebilmeli.
    #[test]
    fn run_entity_update_marshals_input_flags() {
        let mut engine = ScriptEngine::new().unwrap();
        let path = unique_temp("marshal_input");
        std::fs::write(
            &path,
            "function ctl(ctx)\n  local vx = 0\n  if ctx.input.d then vx = 1 end\n  if ctx.input.a then vx = vx - 1 end\n  return { velocity = { x = vx, y = 0, z = 0 } }\nend\n",
        )
        .unwrap();
        engine.load_script(&path).unwrap();

        let mut ctx = ScriptContext {
            key_d: true, // sağa
            ..Default::default()
        };
        let r = engine.run_entity_update(&path, "ctl", &ctx).unwrap();
        assert_eq!(r.new_velocity, Some([1.0, 0.0, 0.0]));

        ctx.key_d = false;
        ctx.key_a = true; // sola
        let r2 = engine.run_entity_update(&path, "ctl", &ctx).unwrap();
        assert_eq!(r2.new_velocity, Some([-1.0, 0.0, 0.0]));

        let _ = std::fs::remove_file(&path);
    }

    /// has_function yalnız yüklü script'te tanımlı fonksiyonlar için true dönmeli.
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

    /// reload_if_changed: içerik değişmediyse false (yeniden yükleme yok), değişince true;
    /// sonra tekrar değişmezse yine false — hot-reload durum makinesi.
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
