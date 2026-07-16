//! Entity API — Lua'ya sunulan entity yönetim fonksiyonları
//!
//! Lua scriptlerinden entity pozisyon, rotasyon, hız ve ölçek bilgilerine
//! erişim sağlar. Tüm değişiklikler komut kuyruğuna yazılır.

use crate::commands::{CommandQueue, ScriptCommand};
use gizmo_core::World;
use gizmo_math::{Quat, Vec3};
use mlua::prelude::*;
use std::sync::Arc;

/// Entity API fonksiyonlarını Lua'ya kaydeder
pub fn register_entity_api(lua: &Lua, command_queue: Arc<CommandQueue>) -> Result<(), LuaError> {
    let entity_table = lua.create_table()?;

    // === POSITION ===
    {
        let cq = command_queue.clone();
        entity_table.set(
            "set_position",
            lua.create_function(move |_, (id, x, y, z): (u32, f32, f32, f32)| {
                cq.push(ScriptCommand::SetPosition(id, Vec3::new(x, y, z)));
                Ok(())
            })?,
        )?;
    }

    // === ROTATION ===
    {
        let cq = command_queue.clone();
        entity_table.set(
            "set_rotation",
            lua.create_function(move |_, (id, x, y, z, w): (u32, f32, f32, f32, f32)| {
                cq.push(ScriptCommand::SetRotation(id, Quat::from_xyzw(x, y, z, w)));
                Ok(())
            })?,
        )?;
    }

    // === SCALE ===
    {
        let cq = command_queue.clone();
        entity_table.set(
            "set_scale",
            lua.create_function(move |_, (id, x, y, z): (u32, f32, f32, f32)| {
                cq.push(ScriptCommand::SetScale(id, Vec3::new(x, y, z)));
                Ok(())
            })?,
        )?;
    }

    // === VELOCITY ===
    {
        let cq = command_queue.clone();
        entity_table.set(
            "set_velocity",
            lua.create_function(move |_, (id, x, y, z): (u32, f32, f32, f32)| {
                cq.push(ScriptCommand::SetVelocity(id, Vec3::new(x, y, z)));
                Ok(())
            })?,
        )?;
    }

    {
        let cq = command_queue.clone();
        entity_table.set(
            "set_angular_velocity",
            lua.create_function(move |_, (id, x, y, z): (u32, f32, f32, f32)| {
                cq.push(ScriptCommand::SetAngularVelocity(id, Vec3::new(x, y, z)));
                Ok(())
            })?,
        )?;
    }

    // === SPAWN ===
    {
        let cq = command_queue.clone();
        entity_table.set(
            "spawn",
            lua.create_function(move |_, (name, x, y, z): (String, f32, f32, f32)| {
                cq.push(ScriptCommand::SpawnEntity {
                    name,
                    position: Vec3::new(x, y, z),
                });
                Ok(())
            })?,
        )?;
    }

    // === SPAWN PREFAB ===
    {
        let cq = command_queue.clone();
        entity_table.set(
            "spawn_prefab",
            lua.create_function(
                move |_, (name, prefab_type, x, y, z): (String, String, f32, f32, f32)| {
                    cq.push(ScriptCommand::SpawnPrefab {
                        name,
                        prefab_type,
                        position: Vec3::new(x, y, z),
                    });
                    Ok(())
                },
            )?,
        )?;
    }

    // === DESTROY ===
    {
        let cq = command_queue.clone();
        entity_table.set(
            "destroy",
            lua.create_function(move |_, id: u32| {
                cq.push(ScriptCommand::DestroyEntity(id));
                Ok(())
            })?,
        )?;
    }

    // === SET NAME ===
    {
        let cq = command_queue.clone();
        entity_table.set(
            "set_name",
            lua.create_function(move |_, (id, name): (u32, String)| {
                cq.push(ScriptCommand::SetEntityName(id, name));
                Ok(())
            })?,
        )?;
    }

    lua.globals().set("entity", entity_table)?;
    Ok(())
}

/// World'den okunan verilerle entity read API'sini günceller (her frame)
#[tracing::instrument(skip_all, name = "script_entity_read")]
pub fn update_entity_read_api(lua: &Lua, world: &World) -> Result<(), LuaError> {
    let entity_table: LuaTable = lua.globals().get("entity")?;

    // get_position: World'den doğrudan okunan veriye dayalı closure
    // Her frame snapshot alarak Lua table'ına yazıyoruz
    let positions = lua.create_table()?;
    let velocities = lua.create_table()?;
    let rotations = lua.create_table()?;
    let scales = lua.create_table()?;
    let names = lua.create_table()?;

    let transforms = world.borrow::<gizmo_physics_core::Transform>();
    for (eid, _) in transforms.iter() {
        if let Some(t) = transforms.get(eid) {
            let pos = lua.create_table()?;
            pos.set("x", t.position.x)?;
            pos.set("y", t.position.y)?;
            pos.set("z", t.position.z)?;
            positions.set(eid, pos)?;

            let rot = lua.create_table()?;
            rot.set("x", t.rotation.x)?;
            rot.set("y", t.rotation.y)?;
            rot.set("z", t.rotation.z)?;
            rot.set("w", t.rotation.w)?;
            rotations.set(eid, rot)?;

            let scl = lua.create_table()?;
            scl.set("x", t.scale.x)?;
            scl.set("y", t.scale.y)?;
            scl.set("z", t.scale.z)?;
            scales.set(eid, scl)?;
        }
    }

    let vels = world.borrow::<gizmo_physics_rigid::components::Velocity>();
    for (eid, _) in vels.iter() {
        if let Some(v) = vels.get(eid) {
            let vel = lua.create_table()?;
            vel.set("x", v.linear.x)?;
            vel.set("y", v.linear.y)?;
            vel.set("z", v.linear.z)?;
            velocities.set(eid, vel)?;
        }
    }

    let entity_names = world.borrow::<gizmo_core::EntityName>();
    for (eid, _) in entity_names.iter() {
        if let Some(n) = entity_names.get(eid) {
            names.set(eid, n.0.clone())?;
        }
    }

    // Snapshot table'ları entity API'sine bağla
    entity_table.set("_positions", positions)?;
    entity_table.set("_velocities", velocities)?;
    entity_table.set("_rotations", rotations)?;
    entity_table.set("_scales", scales)?;
    entity_table.set("_names", names)?;

    // Lua tarafı get_position(id) gibi helper fonksiyonları kullanır
    lua.load(
        r#"
        function entity.get_position(id)
            return entity._positions[id] or {x=0, y=0, z=0}
        end
        function entity.get_velocity(id)
            return entity._velocities[id] or {x=0, y=0, z=0}
        end
        function entity.get_rotation(id)
            return entity._rotations[id] or {x=0, y=0, z=0, w=1}
        end
        function entity.get_scale(id)
            return entity._scales[id] or {x=1, y=1, z=1}
        end
        function entity.get_name(id)
            return entity._names[id] or ""
        end
    "#,
    )
    .exec()?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_physics_core::Transform;
    use mlua::Lua;

    /// Yazma tarafı: set_position/scale/velocity/angular_velocity ve set_rotation
    /// argümanları doğru ScriptCommand'lara (Vec3/Quat) dönüştürüp kuyruğa yazmalı.
    #[test]
    fn write_calls_push_expected_commands() {
        let lua = Lua::new();
        let cq = Arc::new(CommandQueue::new());
        register_entity_api(&lua, cq.clone()).unwrap();

        lua.load(
            r#"
            entity.set_position(1, 1.0, 2.0, 3.0)
            entity.set_scale(1, 2.0, 2.0, 2.0)
            entity.set_velocity(1, -5.0, 0.0, 0.0)
            entity.set_angular_velocity(1, 0.0, 1.5, 0.0)
            entity.set_rotation(1, 0.0, 0.0, 0.0, 1.0)
            "#,
        )
        .exec()
        .unwrap();

        let cmds = cq.drain();
        assert_eq!(cmds.len(), 5);
        assert!(matches!(cmds[0], ScriptCommand::SetPosition(1, p) if p == Vec3::new(1.0, 2.0, 3.0)));
        assert!(matches!(cmds[1], ScriptCommand::SetScale(1, s) if s == Vec3::new(2.0, 2.0, 2.0)));
        assert!(matches!(cmds[2], ScriptCommand::SetVelocity(1, v) if v == Vec3::new(-5.0, 0.0, 0.0)));
        assert!(matches!(cmds[3], ScriptCommand::SetAngularVelocity(1, v) if v == Vec3::new(0.0, 1.5, 0.0)));
        match cmds[4] {
            ScriptCommand::SetRotation(id, q) => {
                assert_eq!(id, 1);
                // Kimlik kuaterniyonu: (0,0,0,1)
                assert!((q.w - 1.0).abs() < 1e-6 && q.x.abs() < 1e-6);
            }
            ref other => panic!("beklenen SetRotation, gelen {other:?}"),
        }
    }

    /// Yaşam döngüsü: spawn / spawn_prefab / destroy / set_name doğru komutlara dönüşmeli.
    #[test]
    fn lifecycle_calls_push_expected_commands() {
        let lua = Lua::new();
        let cq = Arc::new(CommandQueue::new());
        register_entity_api(&lua, cq.clone()).unwrap();

        lua.load(
            r#"
            entity.spawn("hero", 4.0, 5.0, 6.0)
            entity.spawn_prefab("crate", "wooden_box", 0.0, 0.0, 0.0)
            entity.set_name(9, "renamed")
            entity.destroy(9)
            "#,
        )
        .exec()
        .unwrap();

        let cmds = cq.drain();
        assert_eq!(cmds.len(), 4);
        match &cmds[0] {
            ScriptCommand::SpawnEntity { name, position } => {
                assert_eq!(name, "hero");
                assert_eq!(*position, Vec3::new(4.0, 5.0, 6.0));
            }
            other => panic!("beklenen SpawnEntity, gelen {other:?}"),
        }
        match &cmds[1] {
            ScriptCommand::SpawnPrefab { name, prefab_type, position } => {
                assert_eq!(name, "crate");
                assert_eq!(prefab_type, "wooden_box");
                assert_eq!(*position, Vec3::ZERO);
            }
            other => panic!("beklenen SpawnPrefab, gelen {other:?}"),
        }
        match &cmds[2] {
            ScriptCommand::SetEntityName(id, name) => {
                assert_eq!(*id, 9);
                assert_eq!(name, "renamed");
            }
            other => panic!("beklenen SetEntityName, gelen {other:?}"),
        }
        assert!(matches!(cmds[3], ScriptCommand::DestroyEntity(9)));
    }

    /// Okuma tarafı: update_entity_read_api World'den snapshot alır; get_position bilinen
    /// entity için gerçek değeri döndürmeli.
    #[test]
    fn read_api_reflects_world_transform() {
        let lua = Lua::new();
        let cq = Arc::new(CommandQueue::new());
        register_entity_api(&lua, cq).unwrap();

        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Transform::new(Vec3::new(7.0, 8.0, 9.0)));
        let id = e.id();

        update_entity_read_api(&lua, &world).unwrap();

        lua.load(format!(
            r#"
            local p = entity.get_position({id})
            assert(math.abs(p.x - 7.0) < 1e-5, "x")
            assert(math.abs(p.y - 8.0) < 1e-5, "y")
            assert(math.abs(p.z - 9.0) < 1e-5, "z")
            "#
        ))
        .exec()
        .unwrap();
    }

    /// Bilinmeyen entity için getter'lar güvenli varsayılanlar döndürmeli:
    /// pozisyon/hız sıfır, rotasyon kimlik (w=1), ölçek birim (1,1,1), isim boş.
    #[test]
    fn read_api_returns_safe_defaults_for_unknown_id() {
        let lua = Lua::new();
        let cq = Arc::new(CommandQueue::new());
        register_entity_api(&lua, cq).unwrap();

        // Boş dünya → snapshot tabloları boş.
        let world = World::new();
        update_entity_read_api(&lua, &world).unwrap();

        lua.load(
            r#"
            local p = entity.get_position(999)
            assert(p.x == 0 and p.y == 0 and p.z == 0, "pozisyon varsayılanı sıfır")
            local v = entity.get_velocity(999)
            assert(v.x == 0 and v.y == 0 and v.z == 0, "hız varsayılanı sıfır")
            local r = entity.get_rotation(999)
            assert(r.x == 0 and r.y == 0 and r.z == 0 and r.w == 1, "rotasyon varsayılanı kimlik")
            local s = entity.get_scale(999)
            assert(s.x == 1 and s.y == 1 and s.z == 1, "ölçek varsayılanı birim")
            assert(entity.get_name(999) == "", "isim varsayılanı boş string")
            "#,
        )
        .exec()
        .unwrap();
    }
}
