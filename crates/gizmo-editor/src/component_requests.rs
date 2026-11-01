//! Carrying out the inspector's component requests for types the editor only knows by **name** —
//! a game's own components, registered with `ComponentRegistry::register_editable`.
//!
//! The inspector cannot mutate the world while egui draws it, so every edit is recorded on
//! [`EditorState`] and applied by the host afterwards. `gizmo-studio` used to be the only host
//! that applied the JSON edits at all — the in-engine editor (`gizmo-app`'s `editor` feature)
//! drew the generic section, accepted the drag, queued the update, and dropped it — and neither
//! host could add or remove a component whose name was not spelled out in a `match`. These
//! functions are the part both hosts share, so that stays one implementation.

use crate::editor_state::EditorState;
use gizmo_core::entity::Entity;
use gizmo_core::World;

/// Applies every queued inspector JSON edit, logging (not dropping silently) any the component
/// refused — a value its `Deserialize` impl does not accept.
pub fn apply_json_updates(world: &mut World, state: &mut EditorState) {
    for (entity, set_json, value) in std::mem::take(&mut state.pending_json_updates) {
        if let Err(e) = set_json(world, entity, value) {
            state.log_error(&format!("Bileşen güncellenemedi: {e}"));
        }
    }
}

/// Adds a default-valued `name` to `entity` if the world's `ComponentRegistry` knows how —
/// i.e. it was registered with `register_editable`. Returns whether it did.
pub fn add_registered_component(world: &mut World, entity: Entity, name: &str) -> bool {
    let insert = world
        .get_resource::<gizmo_core::ComponentRegistry>()
        .and_then(|r| r.insert_default_fn(name));
    match insert {
        Some(insert) if world.is_alive(entity) => {
            insert(world, entity);
            true
        }
        _ => false,
    }
}

/// Removes `name` from `entity` if the world's `ComponentRegistry` knows how. Returns whether it
/// did.
pub fn remove_registered_component(world: &mut World, entity: Entity, name: &str) -> bool {
    let remove = world
        .get_resource::<gizmo_core::ComponentRegistry>()
        .and_then(|r| r.remove_fn(name));
    match remove {
        Some(remove) if world.is_alive(entity) => {
            remove(world, entity);
            true
        }
        _ => false,
    }
}

/// Everything above, for a host that has no components of its own to match first — the
/// in-engine editor. Add and remove requests for names the registry cannot handle are logged and
/// dropped: leaving them queued would block the next request behind one that can never succeed.
pub fn apply_registry_requests(world: &mut World, state: &mut EditorState) {
    apply_json_updates(world, state);
    if let Some((entity, name)) = state.add_component_request.take() {
        if !add_registered_component(world, entity, &name) {
            state.log_warning(&format!("Eklenemedi: {name} (register_editable ile kaydedilmemiş)"));
        }
    }
    if let Some((entity, name)) = state.remove_component_request.take() {
        if !remove_registered_component(world, entity, &name) {
            state.log_warning(&format!("Silinemedi: {name} (register_editable ile kaydedilmemiş)"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq, Default, serde::Serialize, serde::Deserialize)]
    struct Health {
        hp: f32,
        lives: u32,
    }
    gizmo_core::impl_component!(Health);

    fn world_with_health_registered() -> World {
        let mut world = World::new();
        let mut reg = gizmo_core::ComponentRegistry::new();
        reg.register_editable::<Health>("Health").unwrap();
        world.insert_resource(reg);
        world
    }

    fn state() -> EditorState {
        let mut s = EditorState::default();
        s.pending_json_updates.clear();
        s
    }

    #[test]
    fn add_then_edit_then_remove_a_game_component_by_name() {
        let mut world = world_with_health_registered();
        let e = world.spawn();
        let mut st = state();

        st.add_component_request = Some((e, "Health".into()));
        apply_registry_requests(&mut world, &mut st);
        assert_eq!(world.borrow::<Health>().get(e.id()).cloned(), Some(Health::default()));

        let set = world
            .get_resource::<gizmo_core::ComponentRegistry>()
            .unwrap()
            .get_registration(std::any::TypeId::of::<Health>())
            .unwrap()
            .set_json_fn
            .unwrap();
        st.pending_json_updates.push((e, set, serde_json::json!({ "hp": 2.5, "lives": 4 })));
        apply_registry_requests(&mut world, &mut st);
        assert_eq!(world.borrow::<Health>().get(e.id()).cloned(), Some(Health { hp: 2.5, lives: 4 }));

        st.remove_component_request = Some((e, "Health".into()));
        apply_registry_requests(&mut world, &mut st);
        assert!(world.borrow::<Health>().get(e.id()).is_none());
        assert!(st.add_component_request.is_none() && st.remove_component_request.is_none());
    }

    /// An edit the type refuses is reported, and the component keeps its old value.
    #[test]
    fn a_refused_edit_is_logged_and_changes_nothing() {
        let mut world = world_with_health_registered();
        let e = world.spawn();
        world.add_component(e, Health { hp: 1.0, lives: 2 });
        let mut st = state();
        let set = world
            .get_resource::<gizmo_core::ComponentRegistry>()
            .unwrap()
            .get_registration(std::any::TypeId::of::<Health>())
            .unwrap()
            .set_json_fn
            .unwrap();
        // `lives` is a u32: a float there is exactly what the old inspector wrote back.
        st.pending_json_updates.push((e, set, serde_json::json!({ "hp": 1.0, "lives": 3.0 })));
        st.last_error = None;
        apply_registry_requests(&mut world, &mut st);
        assert_eq!(world.borrow::<Health>().get(e.id()).map(|h| h.lives), Some(2));
        assert!(st.last_error.is_some(), "the refusal was swallowed");
    }

    #[test]
    fn an_unknown_name_is_dropped_not_left_queued() {
        let mut world = world_with_health_registered();
        let e = world.spawn();
        let mut st = state();
        st.add_component_request = Some((e, "Nope".into()));
        apply_registry_requests(&mut world, &mut st);
        assert!(st.add_component_request.is_none());
    }
}
