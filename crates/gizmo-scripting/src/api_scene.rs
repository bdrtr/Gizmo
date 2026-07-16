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
#[tracing::instrument(skip_all, name = "script_scene_read")]
pub fn update_scene_api(lua: &Lua, world: &World) -> Result<(), LuaError> {
    let scene_table: LuaTable = lua.globals().get("scene")?;

    // Entity listesini güncelle
    let entities_table = lua.create_table()?;
    for (idx, entity) in world.iter_alive_entities().into_iter().enumerate() {
        entities_table.set(idx + 1, entity.id())?;
    }
    scene_table.set("_entities", entities_table)?;

    // İsim → ID eşleme tablosu
    let name_map = lua.create_table()?;
    let names = world.borrow::<gizmo_core::EntityName>();
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

#[cfg(test)]
mod tests {
    use super::*;
    use mlua::Lua;

    fn setup() -> (Lua, Arc<CommandQueue>) {
        let lua = Lua::new();
        let cq = Arc::new(CommandQueue::new());
        register_scene_api(&lua, cq.clone()).unwrap();
        (lua, cq)
    }

    /// dialogue.show süre argümanı opsiyonel: verilmezse 3.0'a düşmeli, verilirse korunmalı.
    #[test]
    fn dialogue_show_duration_defaults_and_overrides() {
        let (lua, cq) = setup();
        lua.load(r#"dialogue.show("Ada", "merhaba")"#).exec().unwrap();
        lua.load(r#"dialogue.show("Ada", "hoşça kal", 1.5)"#).exec().unwrap();

        let cmds = cq.drain();
        assert_eq!(cmds.len(), 2);
        match &cmds[0] {
            ScriptCommand::ShowDialogue { speaker, text, duration } => {
                assert_eq!(speaker, "Ada");
                assert_eq!(text, "merhaba");
                assert!((duration - 3.0).abs() < 1e-6, "varsayılan süre 3.0 olmalı");
            }
            other => panic!("beklenen ShowDialogue, gelen {other:?}"),
        }
        match &cmds[1] {
            ScriptCommand::ShowDialogue { duration, .. } => {
                assert!((duration - 1.5).abs() < 1e-6, "verilen süre korunmalı");
            }
            other => panic!("beklenen ShowDialogue, gelen {other:?}"),
        }
    }

    /// race.add_checkpoint yarıçapı opsiyonel: verilmezse 5.0'a düşmeli, verilirse korunmalı.
    #[test]
    fn checkpoint_radius_defaults_and_overrides() {
        let (lua, cq) = setup();
        lua.load("race.add_checkpoint(1, 0.0, 0.0, 0.0)").exec().unwrap();
        lua.load("race.add_checkpoint(2, 1.0, 2.0, 3.0, 12.0)").exec().unwrap();

        let cmds = cq.drain();
        assert_eq!(cmds.len(), 2);
        match &cmds[0] {
            ScriptCommand::AddCheckpoint { id, position, radius } => {
                assert_eq!(*id, 1);
                assert_eq!(*position, gizmo_math::Vec3::ZERO);
                assert!((radius - 5.0).abs() < 1e-6, "varsayılan yarıçap 5.0 olmalı");
            }
            other => panic!("beklenen AddCheckpoint, gelen {other:?}"),
        }
        match &cmds[1] {
            ScriptCommand::AddCheckpoint { position, radius, .. } => {
                assert_eq!(*position, gizmo_math::Vec3::new(1.0, 2.0, 3.0));
                assert!((radius - 12.0).abs() < 1e-6, "verilen yarıçap korunmalı");
            }
            other => panic!("beklenen AddCheckpoint, gelen {other:?}"),
        }
    }

    /// Sahne/kamera/ara-sahne binding'leri: her çağrı beklenen komutu üretmeli.
    #[test]
    fn scene_camera_cutscene_calls_push_expected_commands() {
        let (lua, cq) = setup();
        lua.load(
            r#"
            scene.save("slot1.scene")
            scene.load("level2.scene")
            camera.follow(7)
            camera.set_fov(75.0)
            cutscene.play("intro")
            cutscene.stop()
            race.start()
            race.activate_checkpoint(3)
            race.finish("Ada")
            race.reset()
            "#,
        )
        .exec()
        .unwrap();

        let cmds = cq.drain();
        assert!(matches!(&cmds[0], ScriptCommand::SaveScene(p) if p == "slot1.scene"));
        assert!(matches!(&cmds[1], ScriptCommand::LoadScene(p) if p == "level2.scene"));
        assert!(matches!(cmds[2], ScriptCommand::SetCameraTarget(7)));
        assert!(matches!(cmds[3], ScriptCommand::SetCameraFov(f) if (f - 75.0).abs() < 1e-6));
        assert!(matches!(&cmds[4], ScriptCommand::TriggerCutscene(n) if n == "intro"));
        assert!(matches!(cmds[5], ScriptCommand::EndCutscene));
        assert!(matches!(cmds[6], ScriptCommand::StartRace));
        assert!(matches!(cmds[7], ScriptCommand::ActivateCheckpoint(3)));
        assert!(matches!(&cmds[8], ScriptCommand::FinishRace { winner_name } if winner_name == "Ada"));
        assert!(matches!(cmds[9], ScriptCommand::ResetRace));
    }

    /// update_scene_api isim→id eşlemesi kurmalı; bilinmeyen isim nil, entity_count doğru.
    #[test]
    fn scene_name_lookup_and_count() {
        let (lua, cq) = setup();
        let _ = cq;
        let mut world = World::new();
        let a = world.spawn();
        world.add_component(a, gizmo_core::EntityName::new("player"));
        let b = world.spawn();
        world.add_component(b, gizmo_core::EntityName::new("enemy"));

        update_scene_api(&lua, &world).unwrap();

        let a_id = a.id();
        let b_id = b.id();
        lua.load(format!(
            r#"
            assert(scene.find_by_name("player") == {a_id}, "player id eşleşmeli")
            assert(scene.find_by_name("enemy") == {b_id}, "enemy id eşleşmeli")
            assert(scene.find_by_name("ghost") == nil, "bilinmeyen isim nil dönmeli")
            assert(scene.entity_count() == 2, "iki canlı entity sayılmalı")
            "#
        ))
        .exec()
        .unwrap();
    }
}
