//! Scene API — Lua'ya sunulan sahne ve oyun yönetim fonksiyonları
//!
//! Kapsam: sahne kaydet/yükle, diyalog, ara sahne, yarış sistemi, kamera.

use crate::commands::{CommandQueue, ScriptCommand};
use gizmo_core::World;
use mlua::prelude::*;
use std::sync::Arc;

/// Scene + Game API fonksiyonlarını Lua'ya kaydeder
pub fn register_scene_api(lua: &Lua, command_queue: Arc<CommandQueue>) -> Result<(), LuaError> {
    let scene_table = lua.create_table()?;

    // --- SAHNE KAYDET / YÜKLE ---
    {
        let cq = command_queue.clone();
        scene_table.set(
            "save",
            lua.create_function(move |_, path: String| {
                cq.push(ScriptCommand::SaveScene(path));
                Ok(())
            })?,
        )?;
    }
    {
        let cq = command_queue.clone();
        scene_table.set(
            "load",
            lua.create_function(move |_, path: String| {
                cq.push(ScriptCommand::LoadScene(path));
                Ok(())
            })?,
        )?;
    }

    lua.globals().set("scene", scene_table)?;

    // --- DİYALOG ---
    let dialogue_table = lua.create_table()?;
    {
        let cq = command_queue.clone();
        dialogue_table.set(
            "show",
            lua.create_function(
                move |_, (speaker, text, duration): (String, String, Option<f32>)| {
                    cq.push(ScriptCommand::ShowDialogue {
                        speaker,
                        text,
                        duration: duration.unwrap_or(3.0),
                    });
                    Ok(())
                },
            )?,
        )?;
    }
    {
        let cq = command_queue.clone();
        dialogue_table.set(
            "hide",
            lua.create_function(move |_, ()| {
                cq.push(ScriptCommand::HideDialogue);
                Ok(())
            })?,
        )?;
    }
    lua.globals().set("dialogue", dialogue_table)?;

    // --- ARA SAHNE (CUTSCENE) ---
    let cutscene_table = lua.create_table()?;
    {
        let cq = command_queue.clone();
        cutscene_table.set(
            "play",
            lua.create_function(move |_, name: String| {
                cq.push(ScriptCommand::TriggerCutscene(name));
                Ok(())
            })?,
        )?;
    }
    {
        let cq = command_queue.clone();
        cutscene_table.set(
            "stop",
            lua.create_function(move |_, ()| {
                cq.push(ScriptCommand::EndCutscene);
                Ok(())
            })?,
        )?;
    }
    lua.globals().set("cutscene", cutscene_table)?;

    // --- YARIŞ SİSTEMİ ---
    let race_table = lua.create_table()?;
    {
        let cq = command_queue.clone();
        race_table.set(
            "add_checkpoint",
            lua.create_function(
                move |_, (id, x, y, z, radius): (u32, f32, f32, f32, Option<f32>)| {
                    cq.push(ScriptCommand::AddCheckpoint {
                        id,
                        position: gizmo_math::Vec3::new(x, y, z),
                        radius: radius.unwrap_or(5.0),
                    });
                    Ok(())
                },
            )?,
        )?;
    }
    {
        let cq = command_queue.clone();
        race_table.set(
            "activate_checkpoint",
            lua.create_function(move |_, id: u32| {
                cq.push(ScriptCommand::ActivateCheckpoint(id));
                Ok(())
            })?,
        )?;
    }
    {
        let cq = command_queue.clone();
        race_table.set(
            "finish",
            lua.create_function(move |_, winner: String| {
                cq.push(ScriptCommand::FinishRace {
                    winner_name: winner,
                });
                Ok(())
            })?,
        )?;
    }
    {
        let cq = command_queue.clone();
        race_table.set(
            "reset",
            lua.create_function(move |_, ()| {
                cq.push(ScriptCommand::ResetRace);
                Ok(())
            })?,
        )?;
    }
    {
        let cq = command_queue.clone();
        race_table.set(
            "start",
            lua.create_function(move |_, ()| {
                cq.push(ScriptCommand::StartRace);
                Ok(())
            })?,
        )?;
    }
    lua.globals().set("race", race_table)?;

    // --- KAMERA ---
    let camera_table = lua.create_table()?;
    {
        let cq = command_queue.clone();
        camera_table.set(
            "follow",
            lua.create_function(move |_, entity_id: u32| {
                cq.push(ScriptCommand::SetCameraTarget(entity_id));
                Ok(())
            })?,
        )?;
    }
    {
        let cq = command_queue.clone();
        camera_table.set(
            "set_fov",
            lua.create_function(move |_, fov: f32| {
                cq.push(ScriptCommand::SetCameraFov(fov));
                Ok(())
            })?,
        )?;
    }
    lua.globals().set("camera", camera_table)?;

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

    // İsim → ID eşleme tablosu
    let name_map = lua.create_table()?;
    let names = world.borrow::<gizmo_core::EntityName>().expect("ECS Aliasing Error");
    for (eid, _) in names.iter() {
        if let Some(n) = names.get(eid) {
            name_map.set(n.0.clone(), eid)?;
        }
    }
    scene_table.set("_name_map", name_map)?;

    // Lua helper'ları (sadece bir kere yüklenmeli ama idempotent)
    lua.load(
        r#"
        function scene.get_all_entities()
            return scene._entities or {}
        end

        function scene.find_by_name(name)
            return scene._name_map[name]
        end

        function scene.entity_count()
            local count = 0
            for _ in pairs(scene._entities or {}) do count = count + 1 end
            return count
        end
    "#,
    )
    .exec()?;

    Ok(())
}
