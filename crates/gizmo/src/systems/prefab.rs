//! Fulfilling a [`PrefabRequest`](gizmo_core::PrefabRequest) — the resolver `gizmo-core` says the
//! application has to supply, and nobody did.
//!
//! `gizmo-core` holds no prefab catalogue and says so in the component's own doc: the key is
//! opaque there, "no system in this workspace consumes the component", and whoever resolves it
//! "must also remove the component afterwards, since nothing clears it automatically". That is an
//! honest division of labour and it was never taken up. The result was a **silent** one, which is
//! why this is a bug and not just an unimplemented feature:
//!
//! Lua's `entity.spawn_prefab(name, prefab_type, x, y, z)` is a registered API call, and
//! `ScriptEngine::flush_commands` *matches* it — it spawns an entity with an `EntityName`, a
//! `Transform` and a `PrefabRequest`, and returns nothing to the caller. So the command counted as
//! applied, the script author saw their entity appear in the hierarchy with the right name in the
//! right place, and it had no mesh, no collider and nothing else of the prefab. The engine's other
//! unhandled script commands are handed *back* to the host and are therefore visible; this one was
//! swallowed.
//!
//! **The key is a path, because that is the only rule this tree can support.** The one resolver in
//! the workspace, [`SceneData::load_prefab`](gizmo_scene::scene::SceneData::load_prefab), takes a
//! path; the editor writes prefabs as `prefab_{entity_id}.prefab` into the asset root; and there is
//! no catalogue anywhere to turn a name into either of those. Inventing one here would be deciding
//! a project-layout question inside a system.
//!
//! The prefab is loaded **as a child** of the requesting entity rather than in place of it. That
//! keeps the name and the position the request already carries — a script's `spawn_prefab("crate",
//! …, 0, 5, 0)` produces an entity called `crate` at that point with the prefab hanging under it —
//! and it is the same shape the editor's drag-and-drop already uses, which passes a parent too.

use gizmo_core::PrefabRequest;

/// Resolves every outstanding [`PrefabRequest`], and clears it either way.
///
/// Clearing on failure as well as on success is the whole retry policy: the component is what
/// selects an entity, so a key that cannot be loaded would be re-opened at the frame rate for the
/// life of the process. One warning, once, is the honest answer — and it is one more than the
/// caller used to get.
pub fn prefab_request_system(world: &mut crate::core::World) {
    // Collect first: loading a prefab spawns entities, and structural changes cannot run while a
    // query borrow is live.
    let pending: Vec<(u32, String)> = {
        let requests = world.borrow::<PrefabRequest>();
        requests.iter().map(|(id, req)| (id, req.0.clone())).collect()
    };
    if pending.is_empty() {
        return;
    }

    let registry = crate::full_scene_registry();
    for (id, key) in pending {
        if key.is_empty() {
            tracing::warn!("[Prefab] a request with an empty key cannot name a file; dropped");
        } else {
            match crate::scene::scene::SceneData::load_prefab(&key, Some(id), world, &registry) {
                Ok(_) => tracing::info!(prefab = %key, entity = id, "[Prefab] instantiated"),
                Err(e) => tracing::warn!(
                    prefab = %key,
                    error = %e,
                    "[Prefab] could not be loaded; the entity stays empty and is not retried"
                ),
            }
        }
        if let Some(entity) = world.get_entity(id) {
            world.remove_component::<PrefabRequest>(entity);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::World;

    /// A request with no key names no file, and must not be retried for ever either.
    #[test]
    fn an_empty_key_is_dropped_rather_than_retried() {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, PrefabRequest::new(""));

        prefab_request_system(&mut world);

        assert!(
            world.borrow::<PrefabRequest>().get(e.id()).is_none(),
            "the request must be cleared, or it selects this entity again next frame"
        );
    }

    /// **The retry policy, which is the half that has to hold when the load fails.** A key that
    /// names nothing leaves the entity as it was — but without the request, because the component
    /// is what selects it and a missing file would otherwise be re-opened at the frame rate.
    #[test]
    fn a_key_that_names_nothing_is_tried_once() {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, PrefabRequest::new("does/not/exist.prefab"));

        prefab_request_system(&mut world);

        assert!(
            world.borrow::<PrefabRequest>().get(e.id()).is_none(),
            "a failed load must still clear the request"
        );
        assert!(world.get_entity(e.id()).is_some(), "and must not take the entity with it");
    }

    /// **The whole point, driven through a real file.** A prefab is written from one world and
    /// requested in another; what has to arrive is the prefab's *contents*, which is exactly what
    /// a script author was not getting.
    ///
    /// The requesting entity keeps its own name and position — those are what the request already
    /// carried — and the prefab lands underneath it, the same shape the editor's drag-and-drop
    /// uses.
    ///
    /// Gated on `physics` because the recognisable component it plants is a `Velocity`, and
    /// `gizmo-physics-rigid` is an optional dependency — a `--features scene` test build without
    /// physics would not compile otherwise. The system itself needs no physics.
    #[test]
    #[cfg(feature = "physics")]
    fn a_request_that_names_a_file_gets_the_prefabs_contents() {
        use crate::math::Vec3;
        use crate::physics::components::Transform;
        use gizmo_core::EntityName;

        // A prefab: one entity carrying a recognisable velocity, written to disk.
        let path = std::env::temp_dir()
            .join(format!("gizmo_prefab_req_{}.prefab", std::process::id()))
            .to_string_lossy()
            .to_string();
        {
            let mut source = World::new();
            let e = source.spawn();
            source.add_component(e, Transform::new(Vec3::new(7.0, 0.0, 0.0)));
            source.add_component(
                e,
                gizmo_physics_rigid::components::Velocity::new(Vec3::new(0.0, 0.0, 3.5)),
            );
            crate::scene::scene::SceneData::save_prefab(
                &source,
                e.id(),
                &path,
                &crate::full_scene_registry(),
            )
            .expect("the prefab is written");
        }

        // …and requested, exactly as `flush_commands` leaves it after `entity.spawn_prefab`.
        let mut world = World::new();
        let host = world.spawn();
        world.add_component(host, EntityName::new("crate"));
        world.add_component(host, Transform::new(Vec3::new(0.0, 5.0, 0.0)));
        world.add_component(host, PrefabRequest::new(&path));

        prefab_request_system(&mut world);
        let _ = std::fs::remove_file(&path);

        assert!(
            world.borrow::<PrefabRequest>().get(host.id()).is_none(),
            "a fulfilled request must be cleared — nothing else clears it"
        );

        let velocities = world.borrow::<gizmo_physics_rigid::components::Velocity>();
        let arrived: Vec<f32> = velocities.iter().map(|(_, v)| v.linear.z).collect();
        assert_eq!(
            arrived,
            vec![3.5],
            "the prefab's contents have to arrive — a named empty transform is what this was \
             producing before, and it is indistinguishable from success on screen"
        );
        drop(velocities);

        let names = world.borrow::<EntityName>();
        assert_eq!(
            names.get(host.id()).map(|n| n.0.as_str()),
            Some("crate"),
            "and the requesting entity keeps the name the script gave it"
        );
    }

    /// A world with no requests does no work — this runs on the fixed step of every game.
    #[test]
    fn nothing_pending_is_a_no_op() {
        let mut world = World::new();
        let e = world.spawn();
        prefab_request_system(&mut world);
        assert!(world.get_entity(e.id()).is_some());
    }
}
