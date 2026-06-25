//! Component (de)serialization bridge.
//!
//! Centralizes the optional `bevy_reflect`-based serialization path behind the
//! `reflect` feature, with a plain `serde` fallback that is always available.
//! Scene save/load ([`scene`](crate::scene)) and the in-memory snapshot
//! ([`snapshot`](crate::snapshot)) both route component (de)serialization
//! through these two helpers, so the `reflect` cfg lives in exactly one place
//! instead of being duplicated across every call site.

use gizmo_core::entity::Entity;
use gizmo_core::registry::{ComponentRegistry, TypeRegistration};
use gizmo_core::World;
use std::any::TypeId;

/// Serialize a single component to a RON string.
///
/// `ptr` is the component's raw pointer (from
/// [`World::get_component_ptr`](gizmo_core::World)). When the `reflect` feature
/// is on and the type is reflect-registered, the typed reflect serializer is
/// used; otherwise the registration's legacy `serde` function pointer is used.
/// Returns `None` when the component has no serializer at all.
pub(crate) fn serialize_component(
    registry: &ComponentRegistry,
    reg: &TypeRegistration,
    type_id: TypeId,
    ptr: *const u8,
) -> Option<String> {
    #[cfg(feature = "reflect")]
    if let Some(get_reflect_ptr) = reg.get_reflect_ptr_fn {
        if registry.reflect_registry.get(type_id).is_some() {
            // SAFETY: `ptr` is a live component pointer obtained from the world
            // for `type_id`; `get_reflect_ptr` is the matching reflect accessor
            // registered for that exact type.
            let reflect_val = unsafe { &*get_reflect_ptr(ptr) };
            let serializer = bevy_reflect::serde::TypedReflectSerializer::new(
                reflect_val,
                &registry.reflect_registry,
            );
            if let Ok(string_repr) = ron::ser::to_string(&serializer) {
                return Some(string_repr);
            }
        }
    }
    #[cfg(not(feature = "reflect"))]
    let _ = (registry, type_id);

    reg.serialize_fn.and_then(|ser_fn| ser_fn(ptr).ok())
}

/// Deserialize a single component from its RON string `comp_val` and insert it
/// onto `entity`.
///
/// When the `reflect` feature is on and the type is reflect-registered, the
/// typed reflect deserializer is used; otherwise the registration's legacy
/// `serde` function pointer is used. Returns `Err` with a human-readable reason
/// when no path succeeds.
pub(crate) fn deserialize_component(
    world: &mut World,
    entity: Entity,
    registry: &ComponentRegistry,
    reg: &TypeRegistration,
    type_id: TypeId,
    comp_val: &str,
) -> Result<(), String> {
    #[cfg(feature = "reflect")]
    if let Some(type_reg) = registry.reflect_registry.get(type_id) {
        let deserializer = bevy_reflect::serde::TypedReflectDeserializer::new(
            type_reg,
            &registry.reflect_registry,
        );
        let mut de = ron::de::Deserializer::from_str(comp_val).map_err(|e| e.to_string())?;
        let reflect_val = serde::de::DeserializeSeed::deserialize(deserializer, &mut de)
            .map_err(|e| e.to_string())?;
        let insert_fn = reg
            .insert_reflect_fn
            .ok_or("reflect-registered component is missing its insert fn")?;
        return insert_fn(world, entity, &*reflect_val);
    }
    #[cfg(not(feature = "reflect"))]
    let _ = (registry, type_id);

    let deserialize_fn = reg
        .deserialize_fn
        .ok_or("component has no deserializer registered")?;
    deserialize_fn(world, entity, comp_val)
}
