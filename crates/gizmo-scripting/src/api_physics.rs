//! Physics API — Lua'ya sunulan fizik sistemi fonksiyonları
//!
//! Kuvvet uygulama, raycast ve yerçekimi ayarı gibi işlemler için kullanılır.

use crate::commands::{CommandQueue, ScriptCommand};
use gizmo_math::Vec3;
use mlua::prelude::*;
use std::sync::Arc;

/// Physics API fonksiyonlarını Lua'ya kaydeder
pub fn register_physics_api(lua: &Lua, command_queue: Arc<CommandQueue>) -> Result<(), LuaError> {
    let physics_table = lua.create_table()?;

    // === KUVVET UYGULA ===
    {
        let cq = command_queue.clone();
        physics_table.set(
            "apply_force",
            lua.create_function(move |_, (id, fx, fy, fz): (u32, f32, f32, f32)| {
                cq.push(ScriptCommand::ApplyForce(id, Vec3::new(fx, fy, fz)));
                Ok(())
            })?,
        )?;
    }

    // === İMPULS UYGULA ===
    {
        let cq = command_queue.clone();
        physics_table.set(
            "apply_impulse",
            lua.create_function(move |_, (id, ix, iy, iz): (u32, f32, f32, f32)| {
                cq.push(ScriptCommand::ApplyImpulse(id, Vec3::new(ix, iy, iz)));
                Ok(())
            })?,
        )?;
    }

    // === RIGIDBODY EKLE ===
    {
        let cq = command_queue.clone();
        physics_table.set(
            "add_rigidbody",
            lua.create_function(
                move |_,
                      (id, mass, restitution, friction, use_gravity): (
                    u32,
                    f32,
                    f32,
                    f32,
                    bool,
                )| {
                    cq.push(ScriptCommand::AddRigidBody {
                        id,
                        mass,
                        restitution,
                        friction,
                        use_gravity,
                    });
                    Ok(())
                },
            )?,
        )?;
    }

    // === COLLIDER EKLE ===
    {
        let cq = command_queue.clone();
        physics_table.set(
            "add_box_collider",
            lua.create_function(move |_, (id, hx, hy, hz): (u32, f32, f32, f32)| {
                cq.push(ScriptCommand::AddBoxCollider { id, hx, hy, hz });
                Ok(())
            })?,
        )?;
    }

    {
        let cq = command_queue.clone();
        physics_table.set(
            "add_sphere_collider",
            lua.create_function(move |_, (id, radius): (u32, f32)| {
                cq.push(ScriptCommand::AddSphereCollider { id, radius });
                Ok(())
            })?,
        )?;
    }

    lua.globals().set("physics", physics_table)?;

    Ok(())
}

/// Her frame güncel fizik olaylarını (Tetikleyiciler, Çarpışmalar) Lua'ya aktarır
pub fn update_physics_api(
    lua: &Lua,
    world: &gizmo_core::World,
) -> Result<(), LuaError> {
    let physics_table: LuaTable = lua.globals().get("physics")?;
    
    let triggers = lua.create_table()?;
    let collisions = lua.create_table()?;
    
    if let Ok(physics_world) = world.try_get_resource::<gizmo_physics::world::PhysicsWorld>() {
        // Trigger (Tetikleyici) Olayları
        for (i, t_event) in physics_world.trigger_events().iter().enumerate() {
            let ev = lua.create_table()?;
            ev.set("trigger_id", t_event.trigger_entity.id())?;
            ev.set("other_id", t_event.other_entity.id())?;
            let status = match t_event.event_type {
                gizmo_physics::collision::CollisionEventType::Started => "enter",
                gizmo_physics::collision::CollisionEventType::Persisting => "stay",
                gizmo_physics::collision::CollisionEventType::Ended => "exit",
            };
            ev.set("status", status)?;
            triggers.set(i + 1, ev)?;
        }
        
        // Fiziksel Çarpışma Olayları
        for (i, c_event) in physics_world.collision_events().iter().enumerate() {
            let ev = lua.create_table()?;
            ev.set("entity_a", c_event.entity_a.id())?;
            ev.set("entity_b", c_event.entity_b.id())?;
            let status = match c_event.event_type {
                gizmo_physics::collision::CollisionEventType::Started => "enter",
                gizmo_physics::collision::CollisionEventType::Persisting => "stay",
                gizmo_physics::collision::CollisionEventType::Ended => "exit",
            };
            ev.set("status", status)?;
            collisions.set(i + 1, ev)?;
        }
    }
    
    // Her frame listeleri güncelle
    physics_table.set("triggers", triggers)?;
    physics_table.set("collisions", collisions)?;
    
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_core::World;
    use gizmo_physics::world::PhysicsWorld;
    use gizmo_physics::collision::{TriggerEvent, CollisionEvent, CollisionEventType};
    use mlua::Lua;

    #[test]
    fn test_update_physics_api() {
        let lua = Lua::new();
        let globals = lua.globals();
        
        // build_physics_api initializes the `physics` global table
        let physics_table = lua.create_table().unwrap();
        globals.set("physics", physics_table).unwrap();

        let mut world = World::new();
        let ent1 = world.spawn();
        let ent2 = world.spawn();

        let mut physics_world = PhysicsWorld::new();
        physics_world.trigger_events.push(TriggerEvent {
            trigger_entity: ent1,
            other_entity: ent2,
            event_type: CollisionEventType::Started,
        });

        physics_world.collision_events.push(CollisionEvent {
            entity_a: ent1,
            entity_b: ent2,
            event_type: CollisionEventType::Started,
            contact_points: Default::default(),
        });

        world.insert_resource(physics_world);

        update_physics_api(&lua, &world).unwrap();

        let script = r#"
            local triggers = physics.triggers
            local collisions = physics.collisions
            assert(#triggers == 1)
            assert(triggers[1].status == "enter")
            assert(#collisions == 1)
            assert(collisions[1].status == "enter")
        "#;

        lua.load(script).exec().unwrap();
    }
}
