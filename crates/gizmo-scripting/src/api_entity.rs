//! The entity API — the entity-management functions exposed to Lua.
//!
//! Gives Lua scripts access to entity position, rotation, velocity and scale. Every change is
//! written to the command queue.

use gizmo_core::Entity;
use crate::commands::{CommandQueue, ScriptCommand};
use gizmo_core::World;
use gizmo_math::{Quat, Vec3};
use mlua::prelude::*;
use std::sync::Arc;

/// Registers the entity API functions with Lua.
pub fn register_entity_api(lua: &Lua, command_queue: Arc<CommandQueue>) -> Result<(), LuaError> {
    crate::api_table::register_protected(lua, "entity", |entity_table| {

    // === POSITION ===
    {
        let cq = command_queue.clone();
        entity_table.raw_set(
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
        entity_table.raw_set(
            "set_rotation",
            lua.create_function(move |_, (id, x, y, z, w): (u32, f32, f32, f32, f32)| {
                cq.push(ScriptCommand::SetRotation(id, Quat::from_xyzw(x, y, z, w)));
                Ok(())
            })?,
        )?;
    }

    // === MESH / COLOUR ===
    {
        let cq = command_queue.clone();
        entity_table.raw_set(
            "set_mesh",
            lua.create_function(move |_, (id, key): (u32, String)| {
                cq.push(ScriptCommand::SetMesh(id, key));
                Ok(())
            })?,
        )?;
    }
    {
        let cq = command_queue.clone();
        entity_table.raw_set(
            "set_color",
            lua.create_function(move |_, (id, r, g, b, a): (u32, f32, f32, f32, Option<f32>)| {
                cq.push(ScriptCommand::SetColor(id, gizmo_math::Vec4::new(r, g, b, a.unwrap_or(1.0))));
                Ok(())
            })?,
        )?;
    }

    // === SCALE ===
    {
        let cq = command_queue.clone();
        entity_table.raw_set(
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
        entity_table.raw_set(
            "set_velocity",
            lua.create_function(move |_, (id, x, y, z): (u32, f32, f32, f32)| {
                cq.push(ScriptCommand::SetVelocity(id, Vec3::new(x, y, z)));
                Ok(())
            })?,
        )?;
    }

    {
        let cq = command_queue.clone();
        entity_table.raw_set(
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
        entity_table.raw_set(
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
        entity_table.raw_set(
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
        entity_table.raw_set(
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
        entity_table.raw_set(
            "set_name",
            lua.create_function(move |_, (id, name): (u32, String)| {
                cq.push(ScriptCommand::SetEntityName(id, name));
                Ok(())
            })?,
        )?;
    }

        // Defined once, at registration: these are static helpers, and the per-frame update path
        // used to re-`load` them on every frame — which is both wasted work and, now that the API
        // table is read-only to Lua, a write the proxy rejects.
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
    })
}

/// Refreshes the entity read API from the world, every frame.
#[tracing::instrument(skip_all, name = "script_entity_read")]
pub fn update_entity_read_api(lua: &Lua, world: &World) -> Result<(), LuaError> {
    // The real table, not the global: the global is a read-only proxy so a script cannot
    // rewrite the API (see `api_table`), and the engine's per-frame writes go behind it.
    let entity_table = crate::api_table::raw(lua, "entity")?;

    // get_position: World'den doğrudan okunan veriye dayalı closure
    // Her frame snapshot alarak Lua table'ına yazıyoruz
    let positions = lua.create_table()?;
    let velocities = lua.create_table()?;
    let rotations = lua.create_table()?;
    let scales = lua.create_table()?;
    let names = lua.create_table()?;

    let transforms = world.borrow::<gizmo_physics_core::Transform>();
    for (entity, _) in world.query::<(Entity, &gizmo_physics_core::Transform)>().iter() {
        if let Some(t) = transforms.get(entity) {
            let pos = lua.create_table()?;
            pos.set("x", t.position.x)?;
            pos.set("y", t.position.y)?;
            pos.set("z", t.position.z)?;
            positions.set(entity.id(), pos)?;

            let rot = lua.create_table()?;
            rot.set("x", t.rotation.x)?;
            rot.set("y", t.rotation.y)?;
            rot.set("z", t.rotation.z)?;
            rot.set("w", t.rotation.w)?;
            rotations.set(entity.id(), rot)?;

            let scl = lua.create_table()?;
            scl.set("x", t.scale.x)?;
            scl.set("y", t.scale.y)?;
            scl.set("z", t.scale.z)?;
            scales.set(entity.id(), scl)?;
        }
    }

    let vels = world.borrow::<gizmo_physics_rigid::components::Velocity>();
    for (entity, _) in world.query::<(Entity, &gizmo_physics_rigid::components::Velocity)>().iter() {
        if let Some(v) = vels.get(entity) {
            let vel = lua.create_table()?;
            vel.set("x", v.linear.x)?;
            vel.set("y", v.linear.y)?;
            vel.set("z", v.linear.z)?;
            velocities.set(entity.id(), vel)?;
        }
    }

    let entity_names = world.borrow::<gizmo_core::EntityName>();
    for (entity, _) in world.query::<(Entity, &gizmo_core::EntityName)>().iter() {
        if let Some(n) = entity_names.get(entity) {
            names.set(entity.id(), n.0.clone())?;
        }
    }

    // Snapshot table'ları entity API'sine bağla
    entity_table.raw_set("_positions", positions)?;
    entity_table.raw_set("_velocities", velocities)?;
    entity_table.raw_set("_rotations", rotations)?;
    entity_table.raw_set("_scales", scales)?;
    entity_table.raw_set("_names", names)?;


    Ok(())
}

/// For the length of `run` — a frame's `on_update` calls — `entity.spawn(name, x, y, z)` and
/// `entity.spawn_prefab(name, prefab, x, y, z)` **return the new entity's id**.
///
/// They used to return nothing: the spawn was a queued command, so a script that made something
/// could not name it afterwards — not to move it, colour it or destroy it — short of finding it
/// by name on a later frame. Here the id is reserved from `world`'s allocator (which takes
/// `&World`) and travels in the command, so `local id = entity.spawn(...)` followed by
/// `entity.set_position(id, ...)` works in the same frame: the queue applies them in order.
///
/// Outside the window (a script's top level runs at load, with no world) the plain functions are
/// back and return `nil`.
pub(crate) fn with_spawn_ids<R>(
    lua: &Lua,
    world: &World,
    command_queue: &Arc<CommandQueue>,
    run: impl FnOnce() -> Result<R, LuaError>,
) -> Result<R, LuaError> {
    let table = crate::api_table::raw(lua, "entity")?;
    let plain_spawn: LuaValue = table.raw_get("spawn")?;
    let plain_prefab: LuaValue = table.raw_get("spawn_prefab")?;
    let out = lua.scope(|scope| {
        let cq = command_queue.clone();
        let spawn = scope.create_function(move |_, (name, x, y, z): (String, f32, f32, f32)| {
            reserve(world, &cq, name, Vec3::new(x, y, z), None)
        })?;
        let cq = command_queue.clone();
        let spawn_prefab = scope.create_function(
            move |_, (name, prefab, x, y, z): (String, String, f32, f32, f32)| {
                reserve(world, &cq, name, Vec3::new(x, y, z), Some(prefab))
            },
        )?;
        table.raw_set("spawn", spawn)?;
        table.raw_set("spawn_prefab", spawn_prefab)?;
        run()
    });
    table.raw_set("spawn", plain_spawn)?;
    table.raw_set("spawn_prefab", plain_prefab)?;
    out
}

/// Reserves an entity and queues its spawn; the id is what the Lua call returns.
fn reserve(
    world: &World,
    command_queue: &CommandQueue,
    name: String,
    position: Vec3,
    prefab: Option<String>,
) -> Result<u32, LuaError> {
    // Checked before reserving: the queue drops a non-finite command, and an id reserved for a
    // command that never arrives would count as alive with nothing behind it.
    if !position.is_finite() {
        return Err(LuaError::RuntimeError(format!("entity.spawn('{name}'): the position is not finite")));
    }
    let Some(entities) = world.get_resource::<gizmo_core::entity::allocator::Entities>() else {
        return Err(LuaError::RuntimeError("entity.spawn: the world has no entity allocator".into()));
    };
    let entity = entities.reserve_entity();
    drop(entities);
    command_queue.push(ScriptCommand::SpawnReserved { entity, name, position, prefab });
    Ok(entity.id())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_physics_core::Transform;
    use mlua::Lua;

    /// The write side: set_position/scale/velocity/angular_velocity and set_rotation must
    /// convert their arguments into the right ScriptCommands (Vec3/Quat) and queue them.
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

    /// Lifecycle: spawn / spawn_prefab / destroy / set_name must turn into the right
    /// commands.
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

    /// The read side: update_entity_read_api snapshots the World, and get_position must return
    /// the real value for a known entity.
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

    /// For an unknown entity the getters must return safe defaults: zero position and velocity,
    /// identity rotation (w=1), unit scale (1,1,1) and an empty name.
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
