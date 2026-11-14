//! **What a scene saves is what the world holds.** Every component the engine defines is either
//! registered for scene files — and so for the editor's ▶/⏹ snapshot, which is a scene — or named
//! here with the reason it is not.
//!
//! Decided 2026-11-14: "saved = seen" is a guarantee, closed class by class. Until then a component
//! nobody registered was simply not written and nothing said so: a decal, a sprite, a cloth, a
//! whole UI, were gone after a save — and after ⏹, for an entity play had destroyed.
//!
//! Two lists, and the test holds them both to the source:
//!
//! - [`NOT_SAVED`]: components a scene must not hold, each with why — carried another way (the
//!   hierarchy, the name), rebuilt from what is saved (GPU state, caches), or state that only exists
//!   while a game runs.
//! - [`OPEN`]: components that should be saved and are not yet. **This list only shrinks**: an entry
//!   that becomes registered fails the test until it is removed, and a new component that is in
//!   neither list and not registered fails it at once.

use std::path::{Path, PathBuf};

/// Not saved, on purpose. Each reason is the argument; a wrong one is a bug in this list.
const NOT_SAVED: &[(&str, &str)] = &[
    // ── Carried by the scene format itself ─────────────────────────────────────────────────────
    ("EntityName", "the scene's `EntityData::name`"),
    ("Parent", "the scene's `EntityData::parent_id`"),
    ("Children", "rebuilt on load from the `Parent` edges, so the two directions cannot disagree"),
    ("MeshSource", "the scene's `EntityData::mesh_source`"),
    ("MaterialSource", "the scene's `EntityData::material_source`"),
    // ── Rebuilt from what is saved ──────────────────────────────────────────────────────────────
    ("GlobalTransform", "computed from `Transform` every frame"),
    ("Mesh", "a GPU buffer; saved as its `MeshSource` key or its `MeshData` (`mesh_sync`)"),
    ("Material", "a GPU bind group; saved as its `MaterialDesc` (`material_sync`)"),
    ("Skeleton", "a GPU buffer; rebuilt from the model's file (`gltf_model`)"),
    ("Decal", "a GPU bind group; saved as its `DecalDesc` (`sync_decals`)"),
    ("LodGroup", "GPU meshes; saved as its `LodGroupDesc`, each level's mesh by its key (`sync_lod_groups`)"),
    ("CameraTarget", "a GPU texture; saved as its `CameraTargetDesc` (its size), the texture made again (`sync_camera_targets`)"),
    ("GltfModelResolved", "the mark that a model has its skeletons and clips; left off a file so a reopened one is resolved"),
    ("Animated", "the transform animator's mark of what it is driving, set by the system each frame"),
    ("LodLevel", "one level inside a `LodGroup`, saved with it"),
    ("VisibilityIndex", "the renderer's culling cache"),
    ("FluidHandle", "the GPU fluid simulation's handle, made when the simulation is"),
    ("FluidParticle", "marks an entity as one slot of the GPU fluid buffer; the particle itself (where it is, how fast) lives in that buffer and is never read back, so a saved marker would bring back nothing"),
    ("FluidPhase", "the phase of that slot, the same"),
    ("GpuPhysicsLink", "an entity's slot in the GPU rigid-body table, given when the body is uploaded"),
    // ── Only while a game runs ──────────────────────────────────────────────────────────────────
    ("IsDeleted", "an editor soft delete: the entity is not written at all"),
    ("Pooled", "an object pool's spare, waiting to be reused"),
    ("PrefabRequest", "a request the prefab system consumes in the frame it is made"),
    ("Explosion", "a blast applied once and despawned with its entity in the same step"),
    ("Debris", "a piece a fracture makes during play; ⏹ removes what play made"),
    ("Clicked", "this frame's click on a UI node"),
    ("Interaction", "this frame's pointer state over a UI node"),
    ("RenderStats", "the last frame's draw counts, written by the renderer"),
    // ── Generic over a game's own type ──────────────────────────────────────────────────────────
    ("Handle", "`Handle<T>` is an id into a game's `Assets<T>` in memory; a game that saves one registers its own `T`'s"),
    ("DespawnOnEnter", "`DespawnOnEnter<S>` is over a game's state type; the game registers `DespawnOnEnter<ItsState>`"),
    ("DespawnOnExit", "`DespawnOnExit<S>`, the same"),
    // ── The editor's own ────────────────────────────────────────────────────────────────────────
    ("EditorOnly", "editor scaffolding, never part of a scene"),
    ("EditorRenderTarget", "the editor viewport's texture"),
    ("GameRenderTarget", "the game view's texture in the editor"),
];

/// Should be saved, and is not yet — closed class by class (decided 2026-11-14). Only shrinks.
const OPEN: &[&str] = &[
    // Physics and play
    "BehaviorTree",
    // The GPU fluid, which nothing in the engine drives from the ECS yet: `FluidSimulation` is read
    // by the inspector alone and `FluidInteractor` by the fluid demos alone. Saving them would keep
    // knobs that steer nothing (`addable_components_round_trip` refuses the first for that reason);
    // they close with the fluid being driven by them.
    "FluidSimulation", "FluidInteractor",
    // Animation
    "AnimationStateMachine", "Tween", "TwoBoneIkChain",
    // UI
    "UiRoot", "Node", "Style", "BackgroundColor", "ZIndex", "ClipContent", "Scroll", "Drag", "DropTarget", "Dropdown",
    "ProgressBar", "Slider", "Toggle", "Tooltip", "TextInput", "UiTransform", "UiTween", "VirtualButton", "VirtualStick",
    "LocalizedText",
];

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().and_then(|p| p.parent()).expect("workspace root").to_path_buf()
}

fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// The names of the modules `src` declares as `#[cfg(test)]` files (`#[cfg(test)] mod name;`).
fn test_module_files(src: &str) -> Vec<String> {
    src.match_indices("#[cfg(test)]")
        .filter_map(|(i, attr)| {
            let rest = src[i + attr.len()..].trim_start();
            let rest = rest.strip_prefix("pub(crate) ").or_else(|| rest.strip_prefix("pub ")).unwrap_or(rest);
            let name = rest.strip_prefix("mod ")?.split(';').next()?.trim();
            (!name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_')).then(|| name.to_string())
        })
        .collect()
}

/// `src` without its inline test modules: every `#[cfg(test)] mod name { … }`, braces matched. A
/// `#[cfg(test)] mod name;` declaration is a file, left out by [`engine_sources`]; cutting a file
/// at the first `#[cfg(test)]` instead dropped everything after such a declaration.
fn without_test_modules(src: &str) -> String {
    let mut out = String::new();
    let mut rest = src;
    while let Some(i) = rest.find("#[cfg(test)]") {
        let after = &rest[i + "#[cfg(test)]".len()..];
        let item = after.trim_start();
        let item = item.strip_prefix("pub(crate) ").or_else(|| item.strip_prefix("pub ")).unwrap_or(item);
        let inline = item.strip_prefix("mod ").and_then(|m| {
            let head = m.split(['{', ';']).next()?;
            m[head.len()..].starts_with('{').then_some(())
        });
        if inline.is_none() {
            out.push_str(&rest[..i + "#[cfg(test)]".len()]);
            rest = after;
            continue;
        }
        out.push_str(&rest[..i]);
        let open = i + rest[i..].find('{').expect("an inline module has a body");
        let mut depth = 0usize;
        let mut end = rest.len();
        for (k, c) in rest[open..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = open + k + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// Every engine source file's non-test code: inline test modules taken out, and test-module files
/// left out. The studio is the editor, not the engine, and is left out too.
fn engine_sources() -> Vec<String> {
    let mut files = Vec::new();
    collect_rs(&workspace().join("crates"), &mut files);
    let files: Vec<(PathBuf, String)> = files
        .into_iter()
        .filter(|f| f.components().any(|c| c.as_os_str() == "src"))
        .filter(|f| !f.to_string_lossy().contains("gizmo-studio"))
        .map(|f| {
            let src = std::fs::read_to_string(&f).unwrap_or_default().replace("\r\n", "\n");
            (f, src)
        })
        .collect();
    let tests: std::collections::HashSet<String> = files.iter().flat_map(|(_, s)| test_module_files(s)).collect();
    files
        .into_iter()
        .filter(|(f, _)| f.file_stem().and_then(|s| s.to_str()).is_none_or(|stem| !tests.contains(stem)))
        .map(|(_, s)| without_test_modules(&s))
        .collect()
}

/// Every public type of the engine that is a component: declared `pub struct` / `pub enum`, and
/// named in an `impl_component!` or an `impl … Component for`.
fn engine_components() -> Vec<String> {
    let sources = engine_sources();
    let ident = |s: &str| -> String { s.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect() };
    let mut public = std::collections::HashSet::new();
    let mut components = std::collections::BTreeSet::new();
    for src in &sources {
        for line in src.lines() {
            let line = line.trim_start();
            for kw in ["pub struct ", "pub enum "] {
                if let Some(rest) = line.strip_prefix(kw) {
                    public.insert(ident(rest));
                }
            }
            if let Some((_, rest)) = line.split_once("Component for ") {
                if line.contains("impl") {
                    components.insert(ident(rest.trim_start()));
                }
            }
        }
        for (i, _) in src.match_indices("impl_component!(") {
            let body = &src[i + "impl_component!(".len()..];
            let body = body.split(')').next().unwrap_or("").split(';').next().unwrap_or("");
            for name in body.split(',') {
                let name = ident(name.trim());
                if name.starts_with(|c: char| c.is_ascii_uppercase()) {
                    components.insert(name);
                }
            }
        }
    }
    // A macro's own definition names `$ty`, which reads as nothing: not a type.
    components.into_iter().filter(|c| !c.is_empty() && public.contains(c)).collect()
}

#[test]
fn every_component_is_saved_or_says_why_not() {
    let registry = gizmo::full_scene_registry();
    let components = engine_components();
    assert!(components.len() > 80, "the scan found {} components: it is not reading the engine", components.len());
    let listed = |name: &str| NOT_SAVED.iter().any(|(n, _)| *n == name) || OPEN.contains(&name);

    let unaccounted: Vec<&String> =
        components.iter().filter(|c| !registry.contains_name(c) && !listed(c)).collect();
    assert!(
        unaccounted.is_empty(),
        "{unaccounted:?} are components a scene does not save, and nothing here says why. Register \
         each (`gizmo_app::scene_registry`), or add it to NOT_SAVED with the reason a scene must not hold it."
    );

    let closed: Vec<&str> = OPEN.iter().copied().filter(|n| registry.contains_name(n)).collect();
    assert!(closed.is_empty(), "{closed:?} are saved now: take them out of OPEN, which only shrinks");
    let saved_anyway: Vec<&str> = NOT_SAVED.iter().map(|(n, _)| *n).filter(|n| registry.contains_name(n)).collect();
    assert!(saved_anyway.is_empty(), "{saved_anyway:?} are listed as not saved and are registered: which is it?");

    let stale: Vec<&str> = NOT_SAVED
        .iter()
        .map(|(n, _)| *n)
        .chain(OPEN.iter().copied())
        .filter(|n| !components.iter().any(|c| c == n))
        .collect();
    assert!(stale.is_empty(), "{stale:?} are listed and are no engine component: renamed or gone");
    eprintln!("scene fidelity: {} components, {} open", components.len(), OPEN.len());
}
