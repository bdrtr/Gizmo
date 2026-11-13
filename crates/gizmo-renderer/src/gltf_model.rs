//! A glTF model as a scene keeps it.
//!
//! What a spawned model is made of splits in two. Its entities, transforms, meshes (by
//! `MeshSource`) and materials (by `MaterialDesc`, its textures by `gltf:<file>#<image>`) are data
//! and travel in a scene file as any entity's do. Its skeletons and its animation clips are not:
//! a [`Skeleton`] is a GPU buffer and a clip table is the file's own data, which a scene has no
//! reason to copy. [`GltfModel`] on the model's root names the file, and
//! [`resolve_gltf_models`] — part of every frame the engine prepares — gives a model that has come
//! back from a file its skeletons and its clips again, read from that file. The player's state (the
//! clip chosen, where it is in it, its speed) is the player's own and is saved with it.

use std::collections::HashMap;
use std::sync::Arc;

use gizmo_core::component::{Children, MeshSource};
use gizmo_core::{Entity, World};

use crate::components::{AnimationPlayer, Skeleton};

/// The file a glTF model was spawned from, on its root entity. Saved with a scene.
#[derive(Clone, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct GltfModel {
    /// The file, as the spawner was given it.
    pub path: String,
}

impl GltfModel {
    /// A model of the file at `path`.
    #[must_use]
    pub fn new(path: impl Into<String>) -> Self {
        Self { path: path.into() }
    }
}

/// The model has its skeletons and clips: the spawner gave them, or [`resolve_gltf_models`] has
/// looked. Not saved, so a model read from a file is looked at once.
#[derive(Clone, Copy, Debug, Default)]
pub struct GltfModelResolved;

gizmo_core::impl_component!(GltfModel, GltfModelResolved);

/// Gives every [`GltfModel`] that has come back from a scene file what the file holds and the scene
/// does not: a [`Skeleton`] on the root and on each skinned mesh (found by its `MeshSource`, among
/// the root's descendants), and the clips of its [`AnimationPlayer`] — the saved player's, whose
/// state is kept, or a new one when none was saved. Each model once; the file is imported once per
/// path ([`AssetManager::gltf_scene`](crate::asset::AssetManager::gltf_scene)). A file that will not
/// load is warned about, and the model stays as it was.
pub fn resolve_gltf_models(world: &mut World, renderer: &crate::Renderer) {
    let pending: Vec<(Entity, String)> = {
        let resolved = world.query::<&GltfModelResolved>();
        world
            .query::<(Entity, &GltfModel)>()
            .iter()
            .filter(|(e, _)| resolved.get(*e).is_none())
            .map(|(e, m)| (e, m.path.clone()))
            .collect()
    };
    for (root, path) in pending {
        world.add_component(root, GltfModelResolved);
        let scene = {
            let mut assets = renderer.asset_manager.write().unwrap_or_else(|p| p.into_inner());
            assets.gltf_scene(&renderer.device, &renderer.queue, &renderer.scene.texture_bind_group_layout, &path)
        };
        let scene = match scene {
            Ok(scene) => scene,
            Err(error) => {
                tracing::warn!(path, %error, "[GltfModel] the model's file would not load; no skeleton or clips for it");
                continue;
            }
        };

        if !scene.skeletons.is_empty() {
            let skeletons: Vec<Skeleton> =
                scene.skeletons.iter().map(|h| renderer.create_skeleton(Arc::new(h.clone()))).collect();
            // Which skin each of the model's meshes is bound to, by the mesh's cache key.
            let mut skin_of: HashMap<String, usize> = HashMap::new();
            let mut stack: Vec<&crate::asset::GltfNodeData> = scene.roots.iter().collect();
            while let Some(node) = stack.pop() {
                if let Some(skin) = node.skin_index {
                    for (mesh, _) in &node.primitives {
                        skin_of.insert(mesh.source.clone(), skin);
                    }
                }
                stack.extend(node.children.iter());
            }
            let mut gives: Vec<(Entity, Skeleton)> = Vec::new();
            {
                let have = world.query::<&Skeleton>();
                if have.get(root).is_none() {
                    gives.push((root, skeletons[0].clone()));
                }
                let children = world.query::<&Children>();
                let sources = world.query::<&MeshSource>();
                let mut walk = vec![root];
                let mut seen = std::collections::HashSet::new();
                while let Some(e) = walk.pop() {
                    if !seen.insert(e) {
                        continue;
                    }
                    if let Some(skin) = sources.get(e).and_then(|s| skin_of.get(&s.0)) {
                        if have.get(e).is_none() {
                            if let Some(skeleton) = skeletons.get(*skin) {
                                gives.push((e, skeleton.clone()));
                            }
                        }
                    }
                    if let Some(c) = children.get(e) {
                        walk.extend(c.0.iter().copied().filter(|c| world.is_alive(*c)));
                    }
                }
            }
            for (entity, skeleton) in gives {
                world.add_component(entity, skeleton);
            }
        }

        if !scene.animations.is_empty() {
            let clips: Arc<[gizmo_animation::skeletal::AnimationClip]> = Arc::from(scene.animations.clone().into_boxed_slice());
            let saved = world.query_mut::<&mut AnimationPlayer>().get_mut(root).map(|mut p| {
                if p.animations.is_empty() {
                    p.animations = clips.clone();
                }
            });
            if saved.is_none() {
                world.add_component(root, AnimationPlayer { animations: clips, ..Default::default() });
            }
        }
        tracing::debug!(path, entity = root.id(), "[GltfModel] skeletons and clips given back");
    }
}
