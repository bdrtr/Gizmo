//! Scene API — Lua'ya sunulan sahne yönetim fonksiyonları
//!
//! Sahne kaydetme/yükleme ve entity arama işlemleri için kullanılır.

use mlua::prelude::*;
use std::sync::Arc;
use crate::commands::{CommandQueue, ScriptCommand};
use gizmo_core::World;

/// Scene API fonksiyonlarını Lua'ya kaydeder
pub fn register_scene_api(lua: &Lua, command_queue: Arc<CommandQueue>) -> Result<(), LuaError> {
    let scene_table = lua.create_table()?;

    // === SAHNE KAYDET ===
    {
        let cq = command_queue.clone();
        scene_table.set("save", lua.create_function(move |_, path: String| {
            cq.push(ScriptCommand::SaveScene(path));
            Ok(())
        })?)?;
    }

    // === SAHNE YÜKLE ===
    {
        let cq = command_queue.clone();
        scene_table.set("load", lua.create_function(move |_, path: String| {
            cq.push(ScriptCommand::LoadScene(path));
            Ok(())
        })?)?;
    }

    lua.globals().set("scene", scene_table)?;
    Ok(())
}

/// Her frame sahne verisini Lua'ya günceller (entity listesi, isim arama)
pub fn update_scene_api(lua: &Lua, world: &World) -> Result<(), LuaError> {
    let scene_table: LuaTable = lua.globals().get("scene")?;
    
    // Entity listesini güncelle
    let entities_table = lua.create_table()?;
    let mut idx = 1;
    for entity in world.iter_alive_entities() {
        entities_table.set(idx, entity.id())?;
        idx += 1;
    }
    scene_table.set("_entities", entities_table)?;
    
    // İsim-ID eşleme tablosu
    let name_map = lua.create_table()?;
    if let Some(names) = world.borrow::<gizmo_core::EntityName>() {
        for &eid in &names.entity_dense {
            if let Some(n) = names.get(eid) {
                name_map.set(n.0.clone(), eid)?;
            }
        }
    }
    scene_table.set("_name_map", name_map)?;
    
    // Lua helper fonksiyonları
    lua.load(r#"
        function scene.get_all_entities()
            return scene._entities or {}
        end
        
        function scene.find_by_name(name)
            return scene._name_map[name]
        end
        
        function scene.entity_count()
            local count = 0
            for _ in pairs(scene._entities or {}) do
                count = count + 1
            end
            return count
        end
    "#).exec()?;
    
    Ok(())
}
