use mlua::prelude::*;
use std::collections::HashMap;

/// Lua Scripting Motoru — Oyun mantığını Lua scriptlerle yönetmeyi sağlar
pub struct ScriptEngine {
    lua: Lua,
    loaded_scripts: HashMap<String, String>, // dosya_yolu -> script içeriği
}

/// ECS Componenti: Varlığın üzerine hangi Lua script'inin takılı olduğunu tutar
#[derive(Clone, Debug)]
pub struct Script {
    pub file_path: String,
}

impl Script {
    pub fn new(path: &str) -> Self {
        Self { file_path: path.to_string() }
    }
}

/// Lua'ya geçirilecek entity verisi (her frame güncellenir)
#[derive(Clone, Debug)]
pub struct ScriptContext {
    pub entity_id: u32,
    pub dt: f32,
    pub position: [f32; 3],
    pub velocity: [f32; 3],
    // Input durumları
    pub key_w: bool,
    pub key_a: bool,
    pub key_s: bool,
    pub key_d: bool,
    pub key_space: bool,
}

/// Lua'dan dönen değişiklikler
#[derive(Clone, Debug, Default)]
pub struct ScriptResult {
    pub new_position: Option<[f32; 3]>,
    pub new_velocity: Option<[f32; 3]>,
}

impl ScriptEngine {
    pub fn new() -> Result<Self, LuaError> {
        let lua = Lua::new();
        
        // Temel matematik fonksiyonlarını Lua'ya aç
        lua.globals().set("print_engine", lua.create_function(|_, msg: String| {
            println!("[Lua] {}", msg);
            Ok(())
        })?)?;

        // Vec3 yardımcı fonksiyonu
        lua.load(r#"
            function vec3(x, y, z)
                return { x = x or 0, y = y or 0, z = z or 0 }
            end
            
            function vec3_add(a, b)
                return vec3(a.x + b.x, a.y + b.y, a.z + b.z)
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
        "#).exec()?;
        
        Ok(Self {
            lua,
            loaded_scripts: HashMap::new(),
        })
    }

    /// Lua script dosyasını diskten yükler ve önbelleğe alır
    pub fn load_script(&mut self, path: &str) -> Result<(), String> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("Script okunamadı {}: {}", path, e))?;
        
        // Script'i Lua VM'e yükle ve çalıştır (fonksiyon tanımları register edilir)
        self.lua.load(&content).exec()
            .map_err(|e| format!("Lua hata {}: {}", path, e))?;
        
        self.loaded_scripts.insert(path.to_string(), content);
        println!("ScriptEngine: Yüklendi → {}", path);
        Ok(())
    }

    /// Her frame çağrılan `on_update(ctx)` Lua fonksiyonunu çalıştırır
    pub fn run_update(&self, ctx: &ScriptContext) -> Result<ScriptResult, String> {
        let globals = self.lua.globals();
        
        // on_update fonksiyonu tanımlı mı?
        let func: LuaFunction = match globals.get("on_update") {
            Ok(f) => f,
            Err(_) => return Ok(ScriptResult::default()), // Tanımlı değilse atla
        };

        // Context tablosunu oluştur
        let ctx_table = self.lua.create_table().map_err(|e| e.to_string())?;
        ctx_table.set("entity_id", ctx.entity_id).map_err(|e| e.to_string())?;
        ctx_table.set("dt", ctx.dt).map_err(|e| e.to_string())?;
        
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
        
        // Input tablosu
        let input = self.lua.create_table().map_err(|e| e.to_string())?;
        input.set("w", ctx.key_w).map_err(|e| e.to_string())?;
        input.set("a", ctx.key_a).map_err(|e| e.to_string())?;
        input.set("s", ctx.key_s).map_err(|e| e.to_string())?;
        input.set("d", ctx.key_d).map_err(|e| e.to_string())?;
        input.set("space", ctx.key_space).map_err(|e| e.to_string())?;
        ctx_table.set("input", input).map_err(|e| e.to_string())?;

        // Fonksiyonu çağır
        let result_table: LuaTable = func.call(ctx_table).map_err(|e| format!("Lua runtime: {}", e))?;
        
        let mut result = ScriptResult::default();
        
        // Sonuçlardan position ve velocity'yi oku (Lua nil ise None kalır)
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

    /// Script'in hot-reload edilip edilmeyeceğini kontrol eder
    pub fn reload_if_changed(&mut self, path: &str) -> Result<bool, String> {
        let current = std::fs::read_to_string(path)
            .map_err(|e| format!("Script okunamadı: {}", e))?;
        
        if let Some(cached) = self.loaded_scripts.get(path) {
            if *cached == current {
                return Ok(false); // Değişmemiş
            }
        }
        
        self.load_script(path)?;
        Ok(true)
    }
}
