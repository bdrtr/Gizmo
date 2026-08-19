//! What the editor lets you add, the scene file has to keep.
//!
//! Three lists have to agree and only two of them ever did:
//!
//! 1. the `ComponentRegistry` the studio fills at setup — what the ➕ menu offers,
//! 2. `scene_ops`'s match — what "add" actually does (guarded by
//!    `every_delete_button_in_the_inspector_removes_its_component` and its neighbours),
//! 3. the `SceneRegistry` — what a save writes and a load reads back.
//!
//! On 2026-08-19, list 3 was missing `ParticleEmitter`, `Terrain` and `BoneAttachment`. All three
//! were offered by the menu, all three were added by the request handler, all three derived
//! `Serialize`/`Deserialize` — and all three vanished on save, **silently**, because a component
//! the scene registry does not know is not written and nothing reports it. That is the worst shape
//! this class of defect takes: the editor tells you it worked.
//!
//! The list is read from `setup.rs` rather than restated here, so a component registered tomorrow
//! is checked the same day.

use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf()
}

/// Every component name the studio registers for the ➕ menu, read from its setup.
fn menu_component_names(root: &Path) -> Vec<String> {
    let src = std::fs::read_to_string(root.join("crates/gizmo-studio/src/setup.rs"))
        .expect("gizmo-studio setup.rs");
    let mut names = Vec::new();
    for line in src.lines() {
        let Some(rest) = line.split("component_registry.register::<").nth(1) else {
            continue;
        };
        // …::<Type>("Name");  — the quoted name is what every other list keys on.
        if let Some(quoted) = rest.split('"').nth(1) {
            names.push(quoted.to_string());
        }
    }
    names.sort();
    names.dedup();
    names
}

/// **Everything the ➕ menu offers must survive a save**, or be named here with the reason.
#[test]
fn every_addable_component_survives_a_save() {
    /// (component, why a save cannot keep it).
    const CANNOT_ROUND_TRIP: &[(&str, &str)] = &[(
        "Material",
        "it owns a live wgpu bind group — a handle to GPU state, not data — so there is nothing \
         to write. This is why a scene round-trip still loses PBR maps, and it is a real gap \
         rather than an oversight: fixing it means serialising the material's *description* and \
         rebuilding the bind group on load.",
    )];

    let root = workspace_root();
    let menu = menu_component_names(&root);
    assert!(
        menu.len() > 10,
        "only {} components scanned from setup.rs — the parser has stopped matching the \
         registration shape, which would make this test pass by seeing nothing: {menu:?}",
        menu.len()
    );

    let registry = gizmo::full_scene_registry();
    let saved: std::collections::BTreeSet<String> =
        registry.all_names().into_iter().map(str::to_string).collect();

    let mut lost = Vec::new();
    for name in &menu {
        if saved.contains(name) {
            continue;
        }
        if CANNOT_ROUND_TRIP.iter().any(|(n, _)| n == name) {
            continue;
        }
        lost.push(name.clone());
    }

    assert!(
        lost.is_empty(),
        "the editor offers {lost:?} and a save drops them on the floor — silently, because an \
         unregistered component is simply not written. Register them in \
         `gizmo-app/src/scene_registry.rs`, or say here why the file cannot keep them."
    );

    // …and an exception that has stopped being true must go, or the list rots into decoration.
    let stale: Vec<&str> = CANNOT_ROUND_TRIP
        .iter()
        .map(|(n, _)| *n)
        .filter(|n| saved.contains(*n))
        .collect();
    assert!(
        stale.is_empty(),
        "{stale:?} is listed as un-saveable but the registry knows it now — delete the entry"
    );
}

/// The three that were lost, driven through a real save and load rather than a name comparison.
///
/// A registry entry is not proof: the component still has to serialise, deserialise, and come back
/// with its values. This writes a scene to a temp file, loads it into a fresh world and reads the
/// fields back out.
#[test]
fn a_particle_emitter_a_terrain_and_a_bone_attachment_survive_the_file() {
    use gizmo::prelude::*;
    use gizmo::renderer::components::{BoneAttachment, ParticleEmitter, Terrain};

    let mut world = World::new();
    let e = world.spawn();
    world.add_component(e, Transform::new(Vec3::new(1.0, 2.0, 3.0)));

    let mut emitter = ParticleEmitter::default();
    emitter.spawn_rate = 42.0;
    world.add_component(e, emitter);

    world.add_component(e, Terrain::new("heightmaps/valley.png".to_string(), 77.0, 55.0, 12.0));

    world.add_component(
        e,
        BoneAttachment {
            bone_index: 7,
            ..Default::default()
        },
    );

    let path = std::env::temp_dir().join(format!("gizmo_addable_{}.scene", std::process::id()));
    let path = path.to_string_lossy().to_string();
    let registry = gizmo::full_scene_registry();
    gizmo::scene::SceneData::save(&world, &path, &registry).expect("save");

    let mut loaded = World::new();
    gizmo::scene::SceneData::load_into(&path, &mut loaded, &registry).expect("load");
    let _ = std::fs::remove_file(&path);

    let id = loaded
        .borrow::<Transform>()
        .iter()
        .map(|(id, _)| id)
        .next()
        .expect("the entity survived");

    assert_eq!(
        loaded
            .borrow::<ParticleEmitter>()
            .get(id)
            .map(|p| p.spawn_rate),
        Some(42.0),
        "the emitter was written and read back with its rate"
    );
    let terrain = loaded.borrow::<Terrain>();
    let terrain = terrain.get(id).expect("the terrain survived");
    assert_eq!(
        (terrain.heightmap_path.as_str(), terrain.width, terrain.max_height),
        ("heightmaps/valley.png", 77.0, 12.0),
        "and the terrain with the recipe it was authored from"
    );
    assert_eq!(
        loaded
            .borrow::<BoneAttachment>()
            .get(id)
            .map(|b| b.bone_index),
        Some(7),
        "and the attachment with the joint it names"
    );
}
