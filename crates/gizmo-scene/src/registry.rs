use gizmo_core::{Component, World};
use serde::{de::DeserializeOwned, Serialize};
use std::collections::HashMap;

pub type SerializeFn = Box<dyn Fn(&World, u32) -> Option<String> + Send + Sync>;
pub type DeserializeFn = Box<dyn Fn(&mut World, u32, &String) + Send + Sync>;

pub type SceneRegistry = gizmo_core::registry::ComponentRegistry;

pub fn default_scene_registry() -> SceneRegistry {
    let mut reg = SceneRegistry::new();

    reg.register_reflect::<gizmo_physics_core::Transform>("Transform");
    reg.register_reflect::<gizmo_physics_rigid::components::Velocity>("Velocity");
    reg.register_reflect::<gizmo_physics_rigid::components::RigidBody>("RigidBody");
    // Collider has not been migrated to Reflect yet, use legacy serializable
    reg.register_serializable::<gizmo_physics_core::Collider>("Collider");
    reg.register_serializable::<gizmo_physics_core::components::Hitbox>("Hitbox");
    reg.register_serializable::<gizmo_physics_core::components::Hurtbox>("Hurtbox");
    reg.register_serializable::<gizmo_physics_core::components::FighterController>("FighterController");

    reg.register_serializable::<gizmo_scripting::Script>("Script");

    reg
}
