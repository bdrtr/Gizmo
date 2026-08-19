//! The registry scene save/load actually uses.
//!
//! `gizmo-scene` depends on neither the renderer nor the scripting layer, deliberately, so
//! that scene save/load works in a GPU-free headless build. The cost is that
//! [`gizmo_scene::registry::default_scene_registry`] can only register what physics owns —
//! transforms, bodies, colliders, the fighter components. Everything a scene visibly
//! consists of lived outside its reach.
//!
//! That was not a theoretical gap. The three places that save or load a scene each built
//! their own registry from `default_scene_registry()` and hand-added `Script`, so a scene
//! saved from the editor came back with its physics intact and its lights, cameras and
//! audio emitters gone — no error, no warning, just an unlit world. This module is the one
//! place that assembles the whole thing.

use gizmo_scene::registry::SceneRegistry;

/// Every component the enabled feature set can round-trip.
///
/// **The list this has to agree with is the editor's ➕ menu**: a component a user can add and
/// then loses on save is worse than one they cannot add at all, because nothing says so — an
/// unregistered component is simply not written. `gizmo-studio`'s
/// `every_addable_component_survives_a_save` holds the two lists together.
///
/// `Material` itself is absent and always will be: it owns a live wgpu bind group, which is a
/// handle to GPU state rather than data. What travels in its place is `MaterialDesc` — every other
/// field, including the texture path — and `gizmo_renderer::material_sync` turns one into the
/// other at each end. Before that pair existed a scene round-trip lost every material a user had
/// authored, silently.
pub fn full_scene_registry() -> SceneRegistry {
    let mut reg = gizmo_scene::registry::default_scene_registry();

    #[cfg(all(feature = "scene", not(target_arch = "wasm32")))]
    gizmo_scripting::register_script_components(&mut reg);

    #[cfg(feature = "render")]
    {
        use gizmo_renderer::components::{
            BoneAttachment, Camera, DirectionalLight, MaterialDesc, ParticleEmitter, PointLight,
            SpotLight, Terrain,
        };
        // Registration is best-effort: a name collision is a bug in this file, not a reason
        // to take the application down mid-save. Warn loudly and keep the rest.
        macro_rules! reg_or_warn {
            ($ty:ty, $name:literal) => {
                if let Err(e) = reg.register_serializable::<$ty>($name) {
                    tracing::warn!(component = $name, error = ?e, "[Scene] component registration failed");
                }
            };
        }
        reg_or_warn!(Camera, "Camera");
        reg_or_warn!(PointLight, "PointLight");
        reg_or_warn!(DirectionalLight, "DirectionalLight");
        reg_or_warn!(SpotLight, "SpotLight");
        // These three were addable in the editor and unsaveable: a user could put a particle
        // emitter, a terrain or a bone attachment on an entity, save, reopen, and find it gone —
        // silently, because a component the registry does not know is simply not written. All
        // three already derived `Serialize`/`Deserialize`; nothing structural kept them out.
        reg_or_warn!(ParticleEmitter, "ParticleEmitter");
        reg_or_warn!(Terrain, "Terrain");
        reg_or_warn!(BoneAttachment, "BoneAttachment");
        // The material a file CAN hold: `Material` owns a live wgpu bind group, `MaterialDesc` is
        // everything else it is. `gizmo_renderer::material_sync` keeps the pair in step — one pass
        // writes a description for every material so a save has something to write, the other
        // rebuilds the material from the description on load.
        reg_or_warn!(MaterialDesc, "MaterialDesc");
    }

    #[cfg(feature = "audio")]
    if let Err(e) = reg.register_serializable::<gizmo_audio::AudioSource>("AudioSource") {
        tracing::warn!(error = ?e, "[Scene] AudioSource registration failed");
    }

    tracing::debug!(
        component_count = reg.len(),
        "[Scene] full scene registry assembled"
    );
    reg
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The regression this module exists for: a scene is not just physics.
    ///
    /// Before this, `default_scene_registry` was what the save path used, so lights and
    /// cameras — the things a scene visibly *is* — round-tripped to nothing.
    #[test]
    #[cfg(feature = "render")]
    fn lights_and_cameras_are_registered() {
        let reg = full_scene_registry();
        for name in ["Camera", "PointLight", "DirectionalLight", "SpotLight"] {
            assert!(
                reg.contains_name(name),
                "{name} must round-trip; a scene that loses its lights is not a scene"
            );
        }
    }

    /// Whatever the feature set, the physics core must still be there — this registry is a
    /// superset of the default one, never a replacement for it.
    #[test]
    fn the_physics_built_ins_are_still_present() {
        let full = full_scene_registry();
        let default = gizmo_scene::registry::default_scene_registry();
        for name in default.all_names() {
            assert!(
                full.contains_name(name),
                "{name} is in the default registry but missing from the full one"
            );
        }
        assert!(full.len() >= default.len());
    }

    /// User components are the other half of the complaint: the registry has to be open, or
    /// a game's own state is unsaveable no matter what the engine registers.
    #[test]
    fn user_components_can_be_added() {
        #[derive(Clone, serde::Serialize, serde::Deserialize)]
        struct Health(f32);
        gizmo_core::impl_component!(Health);

        let mut reg = full_scene_registry();
        reg.register_serializable::<Health>("Health")
            .expect("a fresh name must register");
        assert!(reg.contains_name("Health"));
    }
}
