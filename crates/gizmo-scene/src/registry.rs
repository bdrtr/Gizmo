/// Type alias for the [component registry](gizmo_core::registry::ComponentRegistry)
/// used to (de)serialize scene components.
pub type SceneRegistry = gizmo_core::registry::ComponentRegistry;

/// Builds a [`SceneRegistry`] pre-populated with the engine's built-in
/// serializable/reflectable components (transforms, physics bodies, colliders,
/// hitboxes, scripts, ...). This is the registry the editor and runtime use for
/// scene save/load and snapshot capture/restore.
pub fn default_scene_registry() -> SceneRegistry {
    let mut reg = SceneRegistry::new();

    // Transform/Velocity/RigidBody round-trip through `bevy_reflect` when the
    // `reflect` feature is on, and through plain `serde` otherwise. Both crates
    // derive `Serialize`/`Deserialize`, so the fallback is fully functional.
    #[cfg(feature = "reflect")]
    {
        reg.register_reflect::<gizmo_physics_core::Transform>("Transform");
        reg.register_reflect::<gizmo_physics_rigid::components::Velocity>("Velocity");
        reg.register_reflect::<gizmo_physics_rigid::components::RigidBody>("RigidBody");
    }
    #[cfg(not(feature = "reflect"))]
    {
        reg.register_serializable::<gizmo_physics_core::Transform>("Transform")
            .expect("built-in component 'Transform' registration must not conflict");
        reg.register_serializable::<gizmo_physics_rigid::components::Velocity>("Velocity")
            .expect("built-in component 'Velocity' registration must not conflict");
        reg.register_serializable::<gizmo_physics_rigid::components::RigidBody>("RigidBody")
            .expect("built-in component 'RigidBody' registration must not conflict");
    }
    // Collider has not been migrated to Reflect yet, use legacy serializable
    reg.register_serializable::<gizmo_physics_core::Collider>("Collider")
        .expect("built-in component 'Collider' registration must not conflict");
    reg.register_serializable::<gizmo_physics_core::components::Hitbox>("Hitbox")
        .expect("built-in component 'Hitbox' registration must not conflict");
    reg.register_serializable::<gizmo_physics_core::components::Hurtbox>("Hurtbox")
        .expect("built-in component 'Hurtbox' registration must not conflict");
    reg.register_serializable::<gizmo_physics_core::components::FighterController>("FighterController")
        .expect("built-in component 'FighterController' registration must not conflict");

    // NOTE: scripting's `Script` component is registered separately by the layer
    // that owns both scenes and scripting (app / editor / facade), via
    // `gizmo_scripting::register_script_components`. This keeps `gizmo-scene` free
    // of a dependency on `gizmo-scripting` (and thus on the renderer it pulls in),
    // so scene save/load works in a GPU-free / headless build.
    tracing::debug!(
        component_count = reg.len(),
        reflect = cfg!(feature = "reflect"),
        "[Scene] default scene registry oluşturuldu",
    );
    reg
}

#[cfg(test)]
mod tests {
    use super::*;

    // The default registry is the contract the editor/runtime rely on for scene save/load:
    // every built-in serializable component must be present under its canonical name. A
    // dropped registration = that component silently vanishes on save (no error, data loss).
    #[test]
    fn default_registry_registers_all_builtin_components() {
        let reg = default_scene_registry();
        for name in [
            "Transform",
            "Velocity",
            "RigidBody",
            "Collider",
            "Hitbox",
            "Hurtbox",
            "FighterController",
        ] {
            assert!(
                reg.contains_name(name),
                "built-in component '{name}' must be in the default scene registry"
            );
        }
        // Exactly these seven — a stray extra registration is as much a regression as a
        // missing one (identical under both `reflect` on/off: reflect swaps HOW three of
        // them (de)serialize, not how many names are registered).
        assert_eq!(reg.len(), 7, "default registry component count drifted");
    }

    // `gizmo-scene` deliberately does NOT depend on `gizmo-scripting` (that would drag the
    // renderer in and break headless/GPU-free builds). So the `Script` component must be
    // registered by a higher layer, never here — its presence would signal a bad dep edge.
    #[test]
    fn default_registry_excludes_script_component() {
        let reg = default_scene_registry();
        assert!(
            !reg.contains_name("Script"),
            "Script must be registered by the scripting layer, not gizmo-scene"
        );
    }

    // Name↔TypeId is a two-way map; a round-trip through it (name → TypeId → name) must be
    // the identity for every registered component, or lookups during load would desync.
    #[test]
    fn name_typeid_lookup_round_trips_for_every_component() {
        let reg = default_scene_registry();
        for name in reg.all_names() {
            let tid = reg
                .get_type_id(name)
                .unwrap_or_else(|| panic!("'{name}' listed by all_names but has no TypeId"));
            assert_eq!(
                reg.get_name_by_id(tid),
                Some(name),
                "TypeId for '{name}' must map back to the same name"
            );
        }
    }

    // The registry is backed by a BTreeMap precisely so iteration order is deterministic
    // (stable diffs, reproducible scene files). Two independent builds must agree, and the
    // order must be sorted.
    #[test]
    fn default_registry_name_order_is_sorted_and_deterministic() {
        let a = default_scene_registry();
        let b = default_scene_registry();
        assert_eq!(a.all_names(), b.all_names(), "registry order must be reproducible");

        let names = a.all_names();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted, "registry names must iterate in sorted order");
    }

    // Every registered component must actually carry a (de)serialization capability, or a
    // save/load through it is a guaranteed no-op/failure. This holds in BOTH feature configs:
    // with `reflect` off the serde `serialize_fn` is set; with `reflect` on the three
    // reflect-backed types instead expose a reflect accessor. Assert "one path exists".
    #[test]
    fn every_registered_component_has_a_serialization_path() {
        let reg = default_scene_registry();
        for name in reg.all_names() {
            let tid = reg.get_type_id(name).unwrap();
            let registration = reg
                .get_registration(tid)
                .unwrap_or_else(|| panic!("'{name}' has a TypeId but no TypeRegistration"));
            let has_serde = registration.serialize_fn.is_some();
            #[cfg(feature = "reflect")]
            let has_reflect = registration.get_reflect_ptr_fn.is_some();
            #[cfg(not(feature = "reflect"))]
            let has_reflect = false;
            assert!(
                has_serde || has_reflect,
                "'{name}' is registered but has no way to be serialized"
            );
        }
    }
}
