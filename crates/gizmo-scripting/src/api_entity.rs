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
pub fn update_entity_read_api(lua: &Lua, world: &World) -> Result<(), LuaError> {
    let entity_table: LuaTable = lua.globals().get("entity")?;

    // get_position: World'den doğrudan okunan veriye dayalı closure
    // Her frame snapshot alarak Lua table'ına yazıyoruz
    let positions = lua.create_table()?;
    let velocities = lua.create_table()?;
    let rotations = lua.create_table()?;
    let scales = lua.create_table()?;
    let names = lua.create_table()?;

    if let Some(transforms) = world.borrow::<gizmo_physics::components::Transform>() {
        for eid in transforms.dense.iter().map(|e| e.entity) {
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
    }

    if let Some(vels) = world.borrow::<gizmo_physics::components::Velocity>() {
        for eid in vels.dense.iter().map(|e| e.entity) {
            if let Some(v) = vels.get(eid) {
                let vel = lua.create_table()?;
                vel.set("x", v.linear.x)?;
                vel.set("y", v.linear.y)?;
                vel.set("z", v.linear.z)?;
                velocities.set(eid, vel)?;
            }
        }
    }

    if let Some(entity_names) = world.borrow::<gizmo_core::EntityName>() {
        for eid in entity_names.dense.iter().map(|e| e.entity) {
            if let Some(n) = entity_names.get(eid) {
                names.set(eid, n.0.clone())?;
            }
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
