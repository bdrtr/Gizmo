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
            BoneAttachment, Camera, DirectionalLight, MaterialDesc, MeshRenderer, ParticleEmitter,
            PointLight, PostProcess, RectLight, SpotLight, Terrain,
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
        reg_or_warn!(RectLight, "RectLight");
        reg_or_warn!(gizmo_renderer::irradiance::IrradianceVolume, "IrradianceVolume");
        // These three were addable in the editor and unsaveable: a user could put a particle
        // emitter, a terrain or a bone attachment on an entity, save, reopen, and find it gone —
        // silently, because a component the registry does not know is simply not written. All
        // three already derived `Serialize`/`Deserialize`; nothing structural kept them out.
        reg_or_warn!(ParticleEmitter, "ParticleEmitter");
        reg_or_warn!(Terrain, "Terrain");
        reg_or_warn!(BoneAttachment, "BoneAttachment");
        // `MeshRenderer` reached this list by a different route than those three, and it is why
        // the test guarding the list had to be widened. It is **not** in the ➕ menu —
        // `asset_loading` puts one beside every mesh it loads — so a check driven by the menu
        // could not see it at all. What a user edits in its inspector section is a LOD bias and a
        // shadow mode ("this hero prop stays sharp", "that blocker casts but is not drawn"), and
        // both went back to their defaults on every save-and-reopen with nothing to say so.
        reg_or_warn!(MeshRenderer, "MeshRenderer");
        // The authored look. It had no file to live in at all: the editor's "World & Environment"
        // panel wrote `EditorState::post_process`, which only the editor's own viewport read, and
        // the engine's frame read the renderer's unrelated fields — so a tuned look was gone on
        // reopen and absent from every exported build, while the panel said it was the scene's.
        reg_or_warn!(PostProcess, "PostProcess");
        // The material a file CAN hold: `Material` owns a live wgpu bind group, `MaterialDesc` is
        // everything else it is. `gizmo_renderer::material_sync` keeps the pair in step — one pass
        // writes a description for every material so a save has something to write, the other
        // rebuilds the material from the description on load.
        reg_or_warn!(MaterialDesc, "MaterialDesc");
        // A material built from nodes. The surface id it is drawn with is the renderer's and means
        // nothing in another run; the graph is what travels, and `resolve_surface_graphs` builds the
        // surface from it after a load.
        reg_or_warn!(gizmo_renderer::shader_graph::SurfaceGraph, "SurfaceGraph");
        // A glTF model: the file on its root, and its player's state (the clips are the file's, and
        // come back from it — `gizmo_renderer::gltf_model`). Without them an animated model reopened
        // in its bind pose and never moved.
        reg_or_warn!(gizmo_renderer::gltf_model::GltfModel, "GltfModel");
        reg_or_warn!(gizmo_renderer::components::AnimationPlayer, "AnimationPlayer");
        // A model's clips as the editor's timeline left them, played instead of the file's.
        reg_or_warn!(gizmo_renderer::gltf_model::GltfClipEdits, "GltfClipEdits");
        // Geometry made from points in code, vertices and all (decided 2026-11-14): saved, and
        // carried through the editor's Play/Stop snapshot, which re-creates what play destroyed.
        reg_or_warn!(gizmo_renderer::components::MeshData, "MeshData");
        // What is seen, saved (scene fidelity, 2026-11-14): `gizmo-studio/tests/scene_fidelity.rs`
        // holds every component to this list or to a reason it is not on it.
        reg_or_warn!(gizmo_core::component::IsHidden, "IsHidden");
        reg_or_warn!(gizmo_renderer::components::ReflectionProbe, "ReflectionProbe");
        reg_or_warn!(gizmo_renderer::components::PlanarReflection, "PlanarReflection");
        reg_or_warn!(gizmo_renderer::components::Occluder, "Occluder");
        reg_or_warn!(gizmo_renderer::components::Wireframe, "Wireframe");
        reg_or_warn!(gizmo_renderer::components::WireframeColor, "WireframeColor");
        reg_or_warn!(gizmo_renderer::components::NoWireframe, "NoWireframe");
        reg_or_warn!(gizmo_renderer::components::CameraShake, "CameraShake");
        reg_or_warn!(gizmo_renderer::components::ColorTween, "ColorTween");
        // A transform eased to a target, where it had got to; and a state machine's graph, state,
        // playhead and blend — its clips are the model's, given back by `resolve_gltf_models`.
        reg_or_warn!(gizmo_renderer::tween::Tween, "Tween");
        reg_or_warn!(gizmo_renderer::components::AnimationStateMachine, "AnimationStateMachine");
        // The limbs the engine bends: each chain's tip bone by name, its target and pole as world
        // points, its weight (`gizmo_renderer::ik`).
        reg_or_warn!(gizmo_renderer::ik::IkChains, "IkChains");
        // A label by its key in the string table: relabelled in the language of the day.
        reg_or_warn!(crate::locale::LocalizedText, "LocalizedText");
        // The scene's fluid (its dials, its box, how much of it) and what stands in it — both
        // handed to the GPU solver by the frame preparation (`gizmo::systems::render::fluid`).
        #[cfg(feature = "physics")]
        reg_or_warn!(gizmo_physics_core::components::FluidSimulation, "FluidSimulation");
        reg_or_warn!(gizmo_renderer::components::FluidInteractor, "FluidInteractor");
        // And the water itself, read from the GPU as it is saved.
        reg_or_warn!(gizmo_renderer::gpu_fluid::FluidState, "FluidState");
        reg_or_warn!(gizmo_renderer::components::SpriteAnimation, "SpriteAnimation");
        // Saved in its faces by name (`text_sync`): "default", a path, or a name a game loads it under.
        reg_or_warn!(gizmo_renderer::components::Text, "Text");
        // Saved in their images by name and sampling (`sprite_sync`).
        reg_or_warn!(gizmo_renderer::components::Sprite, "Sprite");
        reg_or_warn!(gizmo_renderer::components::Tilemap, "Tilemap");
        // A decal owns a GPU bind group; what travels is its description (`sync_decals`).
        reg_or_warn!(gizmo_renderer::components::DecalDesc, "DecalDesc");
        // A LOD group owns GPU meshes; what travels is its description (`sync_lod_groups`).
        reg_or_warn!(gizmo_renderer::components::LodGroupDesc, "LodGroupDesc");
        // A camera drawing into a texture: its size, the texture made again (`sync_camera_targets`).
        reg_or_warn!(gizmo_renderer::components::CameraTargetDesc, "CameraTargetDesc");
    }

    // A game timer, wherever it is ticking.
    if let Err(e) = reg.register_serializable::<gizmo_core::Timer>("Timer") {
        tracing::warn!(error = ?e, "[Scene] Timer registration failed");
    }
    // Vehicles and drowning, the dynamics layer's.
    #[cfg(feature = "physics")]
    {
        if let Err(e) = reg.register_serializable::<gizmo_physics_dynamics::vehicle::VehicleController>("VehicleController") {
            tracing::warn!(error = ?e, "[Scene] VehicleController registration failed");
        }
        if let Err(e) = reg.register_serializable::<gizmo_physics_dynamics::oxygen::Oxygen>("Oxygen") {
            tracing::warn!(error = ?e, "[Scene] Oxygen registration failed");
        }
    }

    #[cfg(feature = "audio")]
    if let Err(e) = reg.register_serializable::<gizmo_audio::AudioSource>("AudioSource") {
        tracing::warn!(error = ?e, "[Scene] AudioSource registration failed");
    }

    // The editor has drawn a full `NavAgent` section — max speed, steering force, arrival radius,
    // live state — for as long as the component has existed, and a scene could not keep one: not
    // registered here, so an agent placed in the editor was gone on save, silently. (It also could
    // not be *added* from the ➕ menu; both lists are fixed together, and
    // `every_addable_component_survives_a_save` holds them that way.) What travels is the tuning
    // and the destination; the route and the replan schedule are runtime state — see `NavAgent`.
    #[cfg(feature = "ai")]
    if let Err(e) = reg.register_serializable::<gizmo_ai::components::NavAgent>("NavAgent") {
        tracing::warn!(error = ?e, "[Scene] NavAgent registration failed");
    }
    // An agent's behaviour tree, mid-run state included: its leaves are names the game's
    // `BtLibrary` resolves, so the tree is data (decided 2026-11-14).
    #[cfg(feature = "ai")]
    if let Err(e) = reg.register_serializable::<gizmo_ai::BehaviorTree>("BehaviorTree") {
        tracing::warn!(error = ?e, "[Scene] BehaviorTree registration failed");
    }

    tracing::debug!(
        component_count = reg.len(),
        "[Scene] full scene registry assembled"
    );
    reg
}

/// [`full_scene_registry`] plus whatever the game registered into the world's
/// `ComponentRegistry` resource — the registry an **editor** save or load must use.
///
/// The engine's registry decides how built-ins are written; the world's can only add types to it
/// ([`ComponentRegistry::extend_from`](gizmo_core::ComponentRegistry::extend_from) never
/// overrides). Before this existed every editor save built `full_scene_registry()` from nothing,
/// so a game component the inspector showed and edited was silently left out of the file.
pub fn scene_registry_for_world(world: &gizmo_core::World) -> SceneRegistry {
    let mut reg = full_scene_registry();
    if let Some(game) = world.get_resource::<gizmo_core::ComponentRegistry>() {
        // Serializable registrations only: an editor's registry also lists types for its ➕
        // menu alone (studio registers `Material` that way), and each of those would make every
        // save warn once per entity and count the component as dropped.
        let added = reg.extend_serializable_from(&game);
        tracing::debug!(added, "[Scene] game components merged into the scene registry");
    }
    reg
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A component the game registered into the world reaches the file and comes back — the
    /// editor-save path end to end, minus the editor.
    #[test]
    fn a_game_component_registered_in_the_world_survives_a_save_and_load() {
        #[derive(Clone, Debug, PartialEq, Default, serde::Serialize, serde::Deserialize)]
        struct Health {
            hp: f32,
            lives: u32,
        }
        gizmo_core::impl_component!(Health);

        let dir = std::env::temp_dir().join(format!("gizmo_game_comp_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("scene.ron");
        let path = path.to_str().unwrap();

        let mut world = gizmo_core::World::new();
        let mut game = gizmo_core::ComponentRegistry::new();
        game.register_editable::<Health>("Health").unwrap();
        world.insert_resource(game);
        let e = world.spawn();
        world.add_component(e, gizmo_core::EntityName::new("hero"));
        world.add_component(e, Health { hp: 7.5, lives: 3 });

        // The old path — the engine's registry alone — does not write it. That is the bug.
        gizmo_scene::scene::SceneData::save(&world, path, &full_scene_registry()).unwrap();
        assert!(
            !std::fs::read_to_string(path).unwrap().contains("Health"),
            "the engine-only registry wrote a component it was never told about?"
        );

        gizmo_scene::scene::SceneData::save(&world, path, &scene_registry_for_world(&world)).unwrap();
        let mut loaded = gizmo_core::World::new();
        let mut game = gizmo_core::ComponentRegistry::new();
        game.register_editable::<Health>("Health").unwrap();
        loaded.insert_resource(game);
        let registry = scene_registry_for_world(&loaded);
        gizmo_scene::scene::SceneData::load_into(path, &mut loaded, &registry).unwrap();
        let healths: Vec<Health> = loaded.query::<&Health>().iter().cloned().collect();
        assert_eq!(healths, vec![Health { hp: 7.5, lives: 3 }]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A material built from nodes travels in the scene file: the graph, every node, link and
    /// value of it, comes back as it was saved.
    #[cfg(feature = "render")]
    #[test]
    fn a_surface_graph_survives_a_save_and_load() {
        use gizmo_renderer::shader_graph::{NodeKind, ShaderGraph, SurfaceGraph};
        let dir = std::env::temp_dir().join(format!("gizmo_surface_graph_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("scene.ron");
        let path = path.to_str().unwrap();

        let mut graph = ShaderGraph::new();
        let noise = graph.add(NodeKind::Noise, [10.0, 20.0]);
        graph.node_mut(noise).unwrap().values[1] = [4.0; 4];
        let color = graph.add(NodeKind::Color([0.9, 0.05, 0.05, 1.0]), [10.0, 120.0]);
        let multiply = graph.add(NodeKind::Multiply, [200.0, 60.0]);
        graph.connect(noise, 0, multiply, 0).unwrap();
        graph.connect(color, 0, multiply, 1).unwrap();
        graph.connect(multiply, 0, graph.output().unwrap(), 0).unwrap();

        let mut world = gizmo_core::World::new();
        let e = world.spawn();
        world.add_component(e, gizmo_core::EntityName::new("statue"));
        world.add_component(e, SurfaceGraph::new(graph.clone()));
        gizmo_scene::scene::SceneData::save(&world, path, &full_scene_registry()).unwrap();

        let mut loaded = gizmo_core::World::new();
        gizmo_scene::scene::SceneData::load_into(path, &mut loaded, &full_scene_registry()).unwrap();
        let graphs: Vec<ShaderGraph> = loaded.query::<&SurfaceGraph>().iter().map(|g| g.graph.clone()).collect();
        assert_eq!(graphs, vec![graph]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The regression this module exists for: a scene is not just physics.
    ///
    /// Before this, `default_scene_registry` was what the save path used, so lights and
    /// cameras — the things a scene visibly *is* — round-tripped to nothing.
    #[test]
    #[cfg(feature = "render")]
    fn lights_and_cameras_are_registered() {
        let reg = full_scene_registry();
        for name in ["Camera", "PointLight", "DirectionalLight", "SpotLight", "RectLight"] {
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
