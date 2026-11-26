//! Moving an asset from the browser: the file and its identity (`AssetManager::move_asset`), and
//! then every reference the open world and the editor hold to it.
//!
//! A scene file on disk needs nothing: it names its assets by path and identity, and the registry
//! now has the identity at its new path (`SceneData::repair_asset_paths`). The open world is
//! different — its components hold the old path, and saving it would write the old path back, with
//! the identity beside it pointing elsewhere. So the move rewrites them: every component that names
//! a file, in each of the forms a name takes (an `obj:` mesh, a glTF sub-mesh key, an image inside
//! a model, a bare path).

use gizmo::editor::EditorState;
use gizmo::prelude::*;
use gizmo::renderer::asset::AssetManager;
use std::collections::HashMap;
use std::path::PathBuf;

/// Carries out the browser's move or rename, if it asked for one. Refused during ▶: the open world
/// then is the play world, and the edited one would keep the old paths.
pub fn handle_asset_move(world: &mut World, editor_state: &mut EditorState, playing: bool) {
    let Some((from, to)) = editor_state.assets.move_request.take() else {
        return;
    };
    if playing {
        let message = "Play sırasında asset taşınmaz — önce ⏹ ile durdurun.";
        editor_state.log_warning(message);
        editor_state.status_message = message.to_string();
        return;
    }
    let result = match world.get_resource_mut::<AssetManager>() {
        Some(mut assets) => assets.move_asset(&from, &to),
        // No registry (a host without the studio's setup): the file still moves with its sidecar.
        None => AssetManager::new().move_asset(&from, &to),
    };
    match result {
        Ok(moved) => {
            let moves = Moves::new(&moved);
            let rewritten = rewrite_world(world, &moves);
            rewrite_editor(editor_state, &moves, &from, &to);
            let message = format!("Taşındı: {} → {} ({rewritten} referans güncellendi)", from.display(), to.display());
            editor_state.log_info(&message);
            editor_state.status_message = message;
        }
        Err(e) => {
            let message = format!("Taşınamadı: {e}");
            editor_state.log_warning(&message);
            editor_state.status_message = message;
        }
    }
}

/// The files a move moved, by their normalised old path.
pub(crate) struct Moves(HashMap<String, String>);

impl Moves {
    pub(crate) fn new(moved: &[(String, String)]) -> Self {
        Self(moved.iter().cloned().collect())
    }

    /// Where the file `path` went, or `None` if it did not move.
    fn path(&self, path: &str) -> Option<String> {
        self.0.get(&AssetManager::normalize_path(path.trim())).cloned()
    }

    /// A mesh key with the file it names moved: an `obj:` path, a glTF sub-mesh key rebuilt around
    /// the new path (its node and primitive kept), or a bare path. A built-in mesh names no file.
    fn mesh(&self, key: &str) -> Option<String> {
        if AssetManager::is_primitive_mesh_key(key) {
            return None;
        }
        if let Some((path, _)) = gizmo::core::component::MeshSource::split_gltf_key(key) {
            return gizmo::core::component::MeshSource::gltf_key_with_path(key, &self.path(path)?);
        }
        if let Some(path) = key.strip_prefix("obj:") {
            return Some(format!("obj:{}", self.path(path)?));
        }
        self.path(key)
    }

    /// A texture name with the file it names moved: an image inside a model (`gltf:<file>#<n>`)
    /// follows its model, a path follows itself; generated and imported names name no file.
    fn texture(&self, key: &str) -> Option<String> {
        if let Some((file, index)) = gizmo::renderer::asset::loaders::split_gltf_image_key(key) {
            return Some(format!("gltf:{}#{index}", self.path(file)?));
        }
        self.path(key)
    }

    fn texture_opt(&self, key: &Option<String>) -> Option<Option<String>> {
        key.as_deref().and_then(|k| self.texture(k)).map(Some)
    }
}

/// Rewrites every component in `world` that names a moved file, and reports how many changed.
pub(crate) fn rewrite_world(world: &mut World, moves: &Moves) -> usize {
    use gizmo::core::component::{MaterialSource, MeshSource, PrefabRequest};
    use gizmo::renderer::components::{
        DecalDesc, LodGroupDesc, Material, MaterialDesc, MaterialMaps, Mesh, ParticleEmitter, SpotLight, Sprite, Terrain, Text,
        Tilemap,
    };
    let mut n = 0;
    n += each(world, |c: &MeshSource| moves.mesh(&c.0).map(MeshSource));
    n += each(world, |c: &Mesh| moves.mesh(&c.source).map(|source| {
        let mut out = c.clone();
        out.source = source;
        out
    }));
    n += each(world, |c: &PrefabRequest| moves.path(&c.0).map(PrefabRequest));
    n += each(world, |c: &MaterialSource| {
        moves.texture_opt(&c.texture_source).map(|texture_source| MaterialSource { texture_source, ..c.clone() })
    });

    let maps = |m: &MaterialMaps| -> Option<MaterialMaps> {
        let fields = [&m.normal, &m.metallic_roughness, &m.emissive, &m.occlusion, &m.lightmap];
        let new: Vec<Option<Option<String>>> = fields.iter().map(|f| moves.texture_opt(f)).collect();
        if new.iter().all(Option::is_none) {
            return None;
        }
        let pick = |i: usize, old: &Option<String>| new[i].clone().unwrap_or_else(|| old.clone());
        let mut out = m.clone();
        out.normal = pick(0, &m.normal);
        out.metallic_roughness = pick(1, &m.metallic_roughness);
        out.emissive = pick(2, &m.emissive);
        out.occlusion = pick(3, &m.occlusion);
        out.lightmap = pick(4, &m.lightmap);
        Some(out)
    };
    n += each(world, |c: &MaterialDesc| {
        let (texture, new_maps) = (moves.texture_opt(&c.texture_source), maps(&c.maps));
        (texture.is_some() || new_maps.is_some()).then(|| {
            let mut out = c.clone();
            out.texture_source = texture.unwrap_or_else(|| c.texture_source.clone());
            out.maps = new_maps.unwrap_or_else(|| c.maps.clone());
            out
        })
    });
    n += each(world, |c: &Material| {
        let (texture, new_maps) = (moves.texture_opt(&c.texture_source), maps(&c.maps));
        (texture.is_some() || new_maps.is_some()).then(|| {
            let mut out = c.clone();
            out.texture_source = texture.unwrap_or_else(|| c.texture_source.clone());
            out.maps = new_maps.unwrap_or_else(|| c.maps.clone());
            out
        })
    });

    n += each(world, |c: &Terrain| moves.path(&c.heightmap_path).map(|heightmap_path| Terrain { heightmap_path, ..c.clone() }));
    n += each(world, |c: &ParticleEmitter| {
        moves.texture_opt(&c.texture_source).map(|texture_source| {
            let mut out = c.clone();
            out.texture_source = texture_source;
            out
        })
    });
    n += each(world, |c: &DecalDesc| moves.texture_opt(&c.texture_source).map(|texture_source| DecalDesc { texture_source, ..c.clone() }));
    n += each(world, |c: &SpotLight| moves.texture_opt(&c.cookie).map(|cookie| SpotLight { cookie, ..c.clone() }));
    n += each(world, |c: &LodGroupDesc| {
        let new: Vec<Option<String>> = c.levels.iter().map(|l| moves.mesh(&l.mesh)).collect();
        new.iter().any(Option::is_some).then(|| {
            let mut out = c.clone();
            for (level, mesh) in out.levels.iter_mut().zip(new) {
                if let Some(mesh) = mesh {
                    level.mesh = mesh;
                }
            }
            out
        })
    });
    n += each(world, |c: &Text| {
        let font = c.font_name.as_deref().and_then(|f| moves.path(f));
        let spans: Vec<Option<String>> = c.spans.iter().map(|s| s.font_name.as_deref().and_then(|f| moves.path(f))).collect();
        (font.is_some() || spans.iter().any(Option::is_some)).then(|| {
            let mut out = c.clone();
            if let Some(font) = font {
                out.font_name = Some(font);
            }
            for (span, font) in out.spans.iter_mut().zip(spans) {
                if let Some(font) = font {
                    span.font_name = Some(font);
                }
            }
            out
        })
    });
    let image = |i: &Option<gizmo::renderer::sprite::ImageName>| -> Option<Option<gizmo::renderer::sprite::ImageName>> {
        let i = i.as_ref()?;
        let (path, normal) = (moves.texture(&i.path), moves.texture_opt(&i.normal_map));
        (path.is_some() || normal.is_some()).then(|| {
            let mut out = i.clone();
            out.path = path.unwrap_or_else(|| i.path.clone());
            out.normal_map = normal.unwrap_or_else(|| i.normal_map.clone());
            Some(out)
        })
    };
    n += each(world, |c: &Sprite| image(&c.image_name).map(|image_name| {
        let mut out = c.clone();
        out.image_name = image_name;
        out
    }));
    n += each(world, |c: &Tilemap| image(&c.tileset_name).map(|tileset_name| {
        let mut out = c.clone();
        out.tileset_name = tileset_name;
        out
    }));
    n += each(world, |c: &gizmo::renderer::gltf_model::GltfModel| {
        moves.path(&c.path).map(|path| {
            let mut out = c.clone();
            out.path = path;
            out
        })
    });
    n += each(world, |c: &gizmo::scripting::Script| {
        moves.path(&c.file_path).map(|file_path| gizmo::scripting::Script { file_path, ..c.clone() })
    });
    // A sound is named by what it was loaded as; a name that is a moved file's path follows it.
    n += each(world, |c: &AudioSource| moves.path(&c.sound_name).map(|sound_name| {
        let mut out = c.clone();
        out.sound_name = sound_name;
        out
    }));
    n
}

/// Replaces each `T` that `edit` gives a new value for; reports how many. A component left as it was
/// is not written, so nothing is marked changed that did not change.
fn each<T: gizmo::core::Component + Clone>(world: &mut World, edit: impl Fn(&T) -> Option<T>) -> usize {
    let mut n = 0;
    let mut query = world.query_mut::<&mut T>();
    for mut c in query.iter_mut() {
        if let Some(new) = edit(&c) {
            *c = new;
            n += 1;
        }
    }
    n
}

/// The editor's own paths that named what moved: the open scene, the script being edited, the
/// browser's selection and the folder it shows; and the browser's listings, read again.
pub(crate) fn rewrite_editor(editor_state: &mut EditorState, moves: &Moves, from: &std::path::Path, to: &std::path::Path) {
    let follow = |p: &str| -> Option<String> {
        moves.path(p).or_else(|| {
            // A folder that moved, or a path inside one.
            let (p, from) = (AssetManager::normalize_path(p), AssetManager::normalize_path(&from.to_string_lossy()));
            let to = AssetManager::normalize_path(&to.to_string_lossy());
            if p == from {
                Some(to)
            } else {
                p.strip_prefix(&format!("{from}/")).map(|rest| format!("{to}/{rest}"))
            }
        })
    };
    if let Some(new) = follow(&editor_state.scene_path) {
        editor_state.scene_path = new;
    }
    if let Some(new) = editor_state.script.active_path.as_deref().and_then(follow) {
        editor_state.script.active_path = Some(new);
    }
    if let Some(new) = editor_state.assets.selected.as_ref().and_then(|p| follow(&p.to_string_lossy())) {
        editor_state.assets.selected = Some(PathBuf::from(new));
    }
    if let Some(new) = follow(&editor_state.assets.root) {
        editor_state.assets.root = new;
    }
    editor_state.assets.cached_dir = None;
    editor_state.assets.tree_cache.clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo::core::component::{MaterialSource, MeshSource};
    use gizmo::renderer::components::{MaterialDesc, MaterialMaps, Terrain};

    fn moves() -> Moves {
        Moves::new(&[
            ("assets/car.glb".into(), "assets/vehicles/car.glb".into()),
            ("assets/rock.obj".into(), "assets/stone.obj".into()),
            ("assets/bark.png".into(), "assets/wood/bark.png".into()),
            ("scripts/door.lua".into(), "scripts/gate.lua".into()),
        ])
    }

    /// Each form a reference takes follows its file: a glTF sub-mesh key keeps its node and
    /// primitive around the new path, an `obj:` mesh its prefix, a material's maps and an image
    /// inside a moved model their own; a built-in mesh and a file that did not move are left alone,
    /// and so is a component that names nothing that moved.
    #[test]
    fn every_form_of_a_reference_follows_its_file() {
        let mut world = World::new();
        let spawn = |world: &mut World, c: MeshSource| {
            let e = world.spawn();
            world.add_component(e, c);
            e
        };
        let sub = spawn(&mut world, MeshSource("gltf_mesh_assets/car.glb_wheel_front_left_p0".into()));
        let obj = spawn(&mut world, MeshSource("obj:./assets/rock.obj".into()));
        let cube = spawn(&mut world, MeshSource("standard_cube".into()));
        let other = spawn(&mut world, MeshSource("assets/tree.obj".into()));
        let desc = MaterialDesc {
            texture_source: Some("gltf:assets/car.glb#2".into()),
            maps: MaterialMaps::default().with_normal("assets/bark.png", 1.0),
            ..Default::default()
        };
        world.add_component(cube, desc);
        let mat = MaterialSource { albedo: [1.0; 4], roughness: 0.5, metallic: 0.0, unlit: 0.0, texture_source: Some("assets/bark.png".into()) };
        world.add_component(other, mat);
        world.add_component(obj, gizmo::scripting::Script::new("scripts/door.lua"));
        world.add_component(sub, Terrain::new("assets/height.png".into(), 1.0, 1.0, 1.0));

        let n = rewrite_world(&mut world, &moves());
        let mesh = |e| world.query::<&MeshSource>().get(e).unwrap().0.clone();
        assert_eq!(mesh(sub), "gltf_mesh_assets/vehicles/car.glb_wheel_front_left_p0");
        assert_eq!(mesh(obj), "obj:assets/stone.obj");
        assert_eq!(mesh(cube), "standard_cube");
        assert_eq!(mesh(other), "assets/tree.obj");
        let desc = world.query::<&MaterialDesc>().get(cube).unwrap().clone();
        assert_eq!(desc.texture_source.as_deref(), Some("gltf:assets/vehicles/car.glb#2"));
        assert_eq!(desc.maps.normal.as_deref(), Some("assets/wood/bark.png"));
        assert_eq!(world.query::<&MaterialSource>().get(other).unwrap().texture_source.as_deref(), Some("assets/wood/bark.png"));
        assert_eq!(world.query::<&gizmo::scripting::Script>().get(obj).unwrap().file_path, "scripts/gate.lua");
        assert_eq!(world.query::<&Terrain>().get(sub).unwrap().heightmap_path, "assets/height.png");
        assert_eq!(n, 5, "two meshes, a material description, a material source, a script");
    }

    /// The browser's move, end to end on a real directory: the file goes with its sidecar, keeps its
    /// identity, and what the open world and the editor named by the old path names the new one.
    #[test]
    fn a_moved_asset_is_followed_by_the_world_and_the_editor() {
        let dir = std::env::temp_dir().join("gizmo_studio_asset_move");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("props")).unwrap();
        let (old, new) = (dir.join("crate.obj"), dir.join("props/box.obj"));
        std::fs::write(&old, b"o crate").unwrap();

        let mut world = World::new();
        let mut registry = AssetManager::new();
        registry.import_assets_directory(&dir);
        let uuid = registry.get_uuid(&old.to_string_lossy()).unwrap();
        world.insert_resource(registry);
        let e = world.spawn();
        world.add_component(e, MeshSource(old.to_string_lossy().into_owned()));

        let mut editor = EditorState::default();
        editor.assets.selected = Some(old.clone());
        editor.assets.move_request = Some((old.clone(), new.clone()));
        handle_asset_move(&mut world, &mut editor, false);

        assert!(new.exists() && !old.exists(), "{}", editor.status_message);
        let registry = world.get_resource::<AssetManager>().unwrap();
        assert_eq!(registry.get_uuid(&new.to_string_lossy()), Some(uuid));
        let normalized = AssetManager::normalize_path(&new.to_string_lossy());
        assert_eq!(world.query::<&MeshSource>().get(e).unwrap().0, normalized);
        assert_eq!(editor.assets.selected, Some(PathBuf::from(&normalized)));
        assert!(editor.assets.move_request.is_none() && editor.assets.cached_dir.is_none());
        drop(registry);

        // During ▶ nothing moves: the world in front of the editor is the play world.
        editor.assets.move_request = Some((new.clone(), old.clone()));
        handle_asset_move(&mut world, &mut editor, true);
        assert!(new.exists() && !old.exists(), "moved during play");
        assert!(editor.status_message.contains("Play"), "{}", editor.status_message);
    }
}
