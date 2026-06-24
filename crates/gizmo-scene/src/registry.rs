/// Type alias for the [component registry](gizmo_core::registry::ComponentRegistry)
/// used to (de)serialize scene components.
pub type SceneRegistry = gizmo_core::registry::ComponentRegistry;

/// Builds a [`SceneRegistry`] pre-populated with the engine's built-in
/// serializable/reflectable components (transforms, physics bodies, colliders,
/// hitboxes, scripts, ...). This is the registry the editor and runtime use for
/// scene save/load and snapshot capture/restore.
pub fn default_scene_registry() -> SceneRegistry {
    let mut reg = SceneRegistry::new();

    reg.register_reflect::<gizmo_physics_core::Transform>("Transform");
    reg.register_reflect::<gizmo_physics_rigid::components::Velocity>("Velocity");
    reg.register_reflect::<gizmo_physics_rigid::components::RigidBody>("RigidBody");
    // Collider has not been migrated to Reflect yet, use legacy serializable
    reg.register_serializable::<gizmo_physics_core::Collider>("Collider")
        .expect("built-in component 'Collider' registration must not conflict");
    reg.register_serializable::<gizmo_physics_core::components::Hitbox>("Hitbox")
        .expect("built-in component 'Hitbox' registration must not conflict");
    reg.register_serializable::<gizmo_physics_core::components::Hurtbox>("Hurtbox")
        .expect("built-in component 'Hurtbox' registration must not conflict");
    reg.register_serializable::<gizmo_physics_core::components::FighterController>("FighterController")
        .expect("built-in component 'FighterController' registration must not conflict");

    #[cfg(not(target_arch = "wasm32"))]
    reg.register_serializable::<gizmo_scripting::Script>("Script")
        .expect("built-in component 'Script' registration must not conflict");

    reg
}
