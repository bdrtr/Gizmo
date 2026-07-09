//! Physics API — Lua'ya sunulan fizik sistemi fonksiyonları
//!
//! Kuvvet uygulama, raycast ve yerçekimi ayarı gibi işlemler için kullanılır.

use crate::commands::{CommandQueue, ScriptCommand};
use gizmo_math::Vec3;
use mlua::prelude::*;
use std::sync::Arc;

/// Bir collider boyutunun alt sınırı. Script'ten gelen negatif/NaN/sonsuz/sıfır değerler
/// (yazım hatası) bu değere kelepçelenir — dejenere AABB veya GJK'da NaN üretmesinler.
const MIN_COLLIDER_DIM: f32 = 1e-4;

/// Collider boyutunu güvene al: sonlu ve pozitif değilse küçük pozitif bir extent'e çek.
/// `NaN`/`-inf`/`inf`/negatif/sıfır hepsi tek dalda `MIN_COLLIDER_DIM`'e düşer.
fn sanitize_dim(v: f32) -> f32 {
    if v.is_finite() && v > MIN_COLLIDER_DIM {
        v
    } else {
        MIN_COLLIDER_DIM
    }
}

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
                // Contact friction/restitution live on the collider material, not
                // the body, so `add_rigidbody` no longer takes them.
                move |_, (id, mass, use_gravity): (u32, f32, bool)| {
                    cq.push(ScriptCommand::AddRigidBody {
                        id,
                        mass,
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
                cq.push(ScriptCommand::AddBoxCollider {
                    id,
                    hx: sanitize_dim(hx),
                    hy: sanitize_dim(hy),
                    hz: sanitize_dim(hz),
                });
                Ok(())
            })?,
        )?;
    }

    {
        let cq = command_queue.clone();
        physics_table.set(
            "add_sphere_collider",
            lua.create_function(move |_, (id, radius): (u32, f32)| {
                cq.push(ScriptCommand::AddSphereCollider {
                    id,
                    radius: sanitize_dim(radius),
                });
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
    
    if let Ok(physics_world) = world.try_get_resource::<gizmo_physics_rigid::world::PhysicsWorld>() {
        // Trigger (Tetikleyici) Olayları
        for (i, t_event) in physics_world.trigger_events().iter().enumerate() {
            let ev = lua.create_table()?;
            ev.set("trigger_id", t_event.trigger_entity.id())?;
            ev.set("other_id", t_event.other_entity.id())?;
            let status = match t_event.event_type {
                gizmo_physics_core::collision::CollisionEventType::Started => "enter",
                gizmo_physics_core::collision::CollisionEventType::Persisting => "stay",
                gizmo_physics_core::collision::CollisionEventType::Ended => "exit",
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
                gizmo_physics_core::collision::CollisionEventType::Started => "enter",
                gizmo_physics_core::collision::CollisionEventType::Persisting => "stay",
                gizmo_physics_core::collision::CollisionEventType::Ended => "exit",
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
    use gizmo_physics_rigid::world::PhysicsWorld;
    use gizmo_physics_core::collision::{TriggerEvent, CollisionEvent, CollisionEventType};
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
            trigger_entity: gizmo_physics_core::BodyHandle::from_id(ent1.id()),
            other_entity: gizmo_physics_core::BodyHandle::from_id(ent2.id()),
            event_type: CollisionEventType::Started,
        });

        physics_world.collision_events.push(CollisionEvent {
            entity_a: gizmo_physics_core::BodyHandle::from_id(ent1.id()),
            entity_b: gizmo_physics_core::BodyHandle::from_id(ent2.id()),
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

    #[test]
    fn sanitize_dim_rejects_bad_values() {
        assert_eq!(sanitize_dim(f32::NAN), MIN_COLLIDER_DIM);
        assert_eq!(sanitize_dim(f32::INFINITY), MIN_COLLIDER_DIM);
        assert_eq!(sanitize_dim(f32::NEG_INFINITY), MIN_COLLIDER_DIM);
        assert_eq!(sanitize_dim(-5.0), MIN_COLLIDER_DIM);
        assert_eq!(sanitize_dim(0.0), MIN_COLLIDER_DIM);
        assert_eq!(sanitize_dim(2.5), 2.5); // valid values pass through
    }

    #[test]
    fn box_collider_dims_are_sanitized_from_lua() {
        let lua = Lua::new();
        let cq = Arc::new(CommandQueue::new());
        register_physics_api(&lua, cq.clone()).unwrap();
        // hx negative, hy NaN (0/0), hz valid — script typo hardening.
        lua.load("physics.add_box_collider(1, -5.0, 0/0, 2.0)").exec().unwrap();

        let cmds = cq.drain();
        assert_eq!(cmds.len(), 1);
        match &cmds[0] {
            ScriptCommand::AddBoxCollider { hx, hy, hz, .. } => {
                assert!(hx.is_finite() && *hx > 0.0, "negative hx must be clamped, got {hx}");
                assert!(hy.is_finite() && *hy > 0.0, "NaN hy must be clamped, got {hy}");
                assert_eq!(*hz, 2.0, "valid hz must pass through untouched");
            }
            other => panic!("expected AddBoxCollider, got {other:?}"),
        }
    }

    #[test]
    fn sphere_collider_radius_is_sanitized_from_lua() {
        let lua = Lua::new();
        let cq = Arc::new(CommandQueue::new());
        register_physics_api(&lua, cq.clone()).unwrap();
        lua.load("physics.add_sphere_collider(7, -3.0)").exec().unwrap();

        let cmds = cq.drain();
        assert_eq!(cmds.len(), 1);
        match &cmds[0] {
            ScriptCommand::AddSphereCollider { radius, .. } => {
                assert!(radius.is_finite() && *radius > 0.0, "radius clamped, got {radius}");
            }
            other => panic!("expected AddSphereCollider, got {other:?}"),
        }
    }
}
