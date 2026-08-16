//! A scene from before components became strings must fail with a sentence, not a column number.
//!
//! The fixture is not synthetic: `legacy_reflection.scene` is the engine's own `perfect_car`
//! scene, which lived in `demo/assets/` and had stopped loading. Nothing noticed, because the
//! only thing that ever opened a `.scene` was the editor and the failure looked like a parser
//! hiccup — `10:28: Expected string`. It sits here now, where the failure it produces is the
//! point.

use gizmo_scene::scene::SceneData;
use gizmo_scene::SceneError;

fn fixture(name: &str) -> String {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
        .to_string_lossy()
        .to_string()
}

#[test]
fn a_reflection_era_scene_is_named_rather_than_pointed_at() {
    let mut world = gizmo_core::World::new();
    let registry = gizmo_scene::registry::default_scene_registry();

    let err = SceneData::load_into(&fixture("legacy_reflection.scene"), &mut world, &registry)
        .expect_err("this file cannot be read by this build");

    assert!(
        matches!(err, SceneError::LegacyComponentEncoding { .. }),
        "an old scene reported as {err:?}; a person holding one needs to be told what is wrong \
         with it, not where the parser stopped"
    );

    let message = err.to_string();
    assert!(
        message.contains("old reflection format"),
        "the message must say what the file is: {message}"
    );
    assert!(
        message.contains("Re-save it"),
        "and what to do about it: {message}"
    );
}

/// The detection must not fire on a file that is merely broken, or every parse error in the
/// engine turns into "your scene is from 2025".
#[test]
fn ordinary_parse_failures_are_still_parse_failures() {
    let path = std::env::temp_dir().join(format!("gizmo_broken_{}.scene", std::process::id()));
    // Valid RON, wrong shape for a scene: `entities` is not even a sequence.
    std::fs::write(&path, "(version: 1, entities: \"nope\", joints: [])").expect("fixture");

    let mut world = gizmo_core::World::new();
    let registry = gizmo_scene::registry::default_scene_registry();
    let err = SceneData::load_into(&path.to_string_lossy(), &mut world, &registry)
        .expect_err("a scene whose entities are a string cannot load");
    let _ = std::fs::remove_file(&path);

    assert!(
        matches!(err, SceneError::Parse(_)),
        "a plain malformed scene must stay a parse error, not be blamed on its age: {err:?}"
    );
}

/// A current-format scene still loads — the detection sits on the failure path and must not cost
/// the working one anything.
#[test]
fn a_current_scene_still_loads() {
    let path = std::env::temp_dir().join(format!("gizmo_current_{}.scene", std::process::id()));
    std::fs::write(
        &path,
        r#"(version: 1, entities: [(
            original_id: 0,
            name: Some("Kutu"),
            mesh_source: None,
            material_source: None,
            parent_id: None,
            components: {},
        )], joints: [])"#,
    )
    .expect("fixture");

    let mut world = gizmo_core::World::new();
    let registry = gizmo_scene::registry::default_scene_registry();
    SceneData::load_into(&path.to_string_lossy(), &mut world, &registry).expect("bugünkü biçim");
    let _ = std::fs::remove_file(&path);
}
