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
//!
//! # Clips edited in the editor
//!
//! The editor's timeline edits an entity's own copy of its clips, never the file. Those edits are
//! the scene's: [`sync_clip_edits`] keeps a [`GltfClipEdits`] beside the player while its clips are
//! not the file's — written when an edit lands, taken away when an undo brings the file's clips
//! back — and the scene saves it. A reopened model plays those clips instead of the file's. The
//! file itself is never written: it may be shared by other scenes, or be someone else's asset.

use std::collections::HashMap;
use std::sync::Arc;

use gizmo_core::component::{Children, MeshSource};
use gizmo_core::{Entity, World};

use crate::components::{AnimationPlayer, AnimationStateMachine, Skeleton};
use gizmo_animation::skeletal::AnimationClip;

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

/// The clips of a glTF model as this scene has edited them, on the model's root: what its player
/// plays instead of the file's. Saved with the scene; kept by [`sync_clip_edits`], not by hand.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GltfClipEdits {
    /// Every clip of the player, edited or not, in the player's order.
    pub clips: Vec<AnimationClip>,
}

/// The clip table the model's file gave its player, by identity: a player whose clips are this
/// `Arc` plays the file's, unedited.
#[derive(Clone)]
struct FileClips(Arc<[AnimationClip]>);

/// The clip table [`sync_clip_edits`] last wrote a [`GltfClipEdits`] from, so an unchanged edit is
/// not copied every frame.
#[derive(Clone)]
struct SyncedClips(Arc<[AnimationClip]>);

gizmo_core::impl_component!(GltfModel, GltfModelResolved, GltfClipEdits, FileClips, SyncedClips);

/// Marks `root` as a model just spawned from `path`, its skeletons and clips given: what the
/// spawner calls once the root has everything. Records the file and the clip table its
/// [`AnimationPlayer`] holds, if it has one, as the file's.
pub fn mark_spawned(world: &mut World, root: Entity, path: &str) {
    world.add_component(root, GltfModel::new(path));
    world.add_component(root, GltfModelResolved);
    let clips = world.query::<&AnimationPlayer>().get(root).map(|p| p.animations.clone());
    if let Some(clips) = clips {
        world.add_component(root, FileClips(clips));
    }
}

/// Keeps a [`GltfClipEdits`] beside every glTF model whose player's clips are not its file's — see
/// the [module docs](self). Needs no GPU; part of every frame the engine prepares.
pub fn sync_clip_edits(world: &mut World) {
    enum Change {
        Write(Arc<[AnimationClip]>),
        Remove,
    }
    let changes: Vec<(Entity, Change)> = {
        let file = world.query::<&FileClips>();
        let synced = world.query::<&SyncedClips>();
        let edits = world.query::<&GltfClipEdits>();
        world
            .query::<(Entity, &AnimationPlayer)>()
            .iter()
            .filter_map(|(e, player)| {
                let file = file.get(e)?;
                if Arc::ptr_eq(&player.animations, &file.0) {
                    edits.get(e).is_some().then_some((e, Change::Remove))
                } else if synced.get(e).is_some_and(|s| Arc::ptr_eq(&s.0, &player.animations)) {
                    None
                } else {
                    Some((e, Change::Write(player.animations.clone())))
                }
            })
            .collect()
    };
    for (entity, change) in changes {
        match change {
            Change::Write(clips) => {
                world.add_component(entity, GltfClipEdits { clips: clips.to_vec() });
                world.add_component(entity, SyncedClips(clips));
            }
            Change::Remove => {
                world.remove_component::<GltfClipEdits>(entity);
                world.remove_component::<SyncedClips>(entity);
            }
        }
    }
}

/// Gives every [`GltfModel`] that has come back from a scene file what the file holds and the scene
/// does not: a [`Skeleton`] on the root and on each skinned mesh (found by its `MeshSource`, among
/// the root's descendants), and its clips: to the saved [`AnimationPlayer`], whose state is kept, or
/// to the saved [`AnimationStateMachine`], whose state is kept too, or to a new player when neither
/// was saved. Each model once; the file is imported once per
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
            let file: Arc<[AnimationClip]> = Arc::from(scene.animations.clone().into_boxed_slice());
            // The scene's edits, when it saved some, are what the player plays; the file's table is
            // still remembered, so an undo back to it is seen as one.
            let edited: Option<Arc<[AnimationClip]>> =
                world.query::<&GltfClipEdits>().get(root).map(|e| Arc::from(e.clips.clone().into_boxed_slice()));
            let plays = edited.clone().unwrap_or_else(|| file.clone());
            let saved = world.query_mut::<&mut AnimationPlayer>().get_mut(root).map(|mut p| {
                if p.animations.is_empty() {
                    p.animations = plays.clone();
                }
            });
            // A model driven by a state machine plays the same table through it — and gets no
            // player of its own, which would fight the machine for the skeleton.
            let machine = world.query_mut::<&mut AnimationStateMachine>().get_mut(root).map(|mut m| {
                if m.clips.is_empty() {
                    m.clips = plays.clone();
                }
            });
            if saved.is_none() && machine.is_none() {
                world.add_component(root, AnimationPlayer { animations: plays.clone(), ..Default::default() });
            }
            world.add_component(root, FileClips(file));
            if edited.is_some() {
                world.add_component(root, SyncedClips(plays));
            }
        }
        tracing::debug!(path, entity = root.id(), "[GltfModel] skeletons and clips given back");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_animation::skeletal::{InterpolationMode, Keyframe, Track};
    use gizmo_math::Quat;

    fn clip(turn: f32) -> AnimationClip {
        AnimationClip {
            name: "Bend".to_string(),
            duration: 1.0,
            translations: Vec::new(),
            rotations: vec![Track {
                target_node: 2,
                target_node_name: Some("Tip".to_string()),
                interpolation: InterpolationMode::Linear,
                keyframes: vec![Keyframe::new(0.0, Quat::IDENTITY), Keyframe::new(1.0, Quat::from_rotation_z(turn))],
            }],
            scales: Vec::new(),
        }
    }

    /// **An edit to a model's clips is kept for the save, and an undo back to the file's clips takes
    /// it away.** A spawned model plays its file's table: no edits. Its player given another table
    /// (what the timeline does): the edits are that table. Nothing changed since: the same edits, not
    /// copied again. The file's table back (what undo does): no edits.
    #[test]
    fn clip_edits_follow_the_player_and_an_undo_takes_them_away() {
        let mut world = World::new();
        let root = world.spawn();
        let file: Arc<[AnimationClip]> = Arc::from(vec![clip(1.0)].into_boxed_slice());
        world.add_component(root, AnimationPlayer { animations: file.clone(), ..Default::default() });
        mark_spawned(&mut world, root, "models/arm.glb");
        sync_clip_edits(&mut world);
        assert!(world.query::<&GltfClipEdits>().get(root).is_none(), "the file's own clips counted as an edit");

        let edited: Arc<[AnimationClip]> = Arc::from(vec![clip(-1.0)].into_boxed_slice());
        world.query_mut::<&mut AnimationPlayer>().get_mut(root).unwrap().animations = edited.clone();
        sync_clip_edits(&mut world);
        assert_eq!(world.query::<&GltfClipEdits>().get(root).map(|e| e.clips.clone()), Some(edited.to_vec()));

        // Unchanged: the component is the one already written (a copy would be a new allocation).
        let before = world.query::<&GltfClipEdits>().get(root).unwrap().clips.as_ptr();
        sync_clip_edits(&mut world);
        assert_eq!(world.query::<&GltfClipEdits>().get(root).unwrap().clips.as_ptr(), before, "an unchanged edit was copied again");

        world.query_mut::<&mut AnimationPlayer>().get_mut(root).unwrap().animations = file;
        sync_clip_edits(&mut world);
        assert!(world.query::<&GltfClipEdits>().get(root).is_none(), "undone back to the file, the edit stayed");
    }

    /// A clip survives the scene format — RON — exactly, tangents and all.
    #[test]
    fn edited_clips_round_trip_through_ron() {
        let mut c = clip(0.7);
        c.rotations[0].keyframes[1] = Keyframe::with_tangents(1.0, Quat::from_rotation_z(0.7), Quat::IDENTITY, Quat::from_rotation_x(0.2));
        c.rotations[0].interpolation = InterpolationMode::CubicSpline;
        let edits = GltfClipEdits { clips: vec![c] };
        let back: GltfClipEdits = ron::from_str(&ron::to_string(&edits).unwrap()).unwrap();
        assert_eq!(back, edits);
    }
}
