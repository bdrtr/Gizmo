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
        "it owns a live wgpu bind group — a handle to GPU state, not data — so the component \
         itself is not what travels. `MaterialDesc` is: every other field, including the texture \
         path, registered under its own name, with `gizmo_renderer::material_sync` converting at \
         each end. So a material DOES survive a save; what cannot be written is the handle.",
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

/// **A material survives the file** — as a description, which is the only form a file can hold.
///
/// The component a user edits owns a live wgpu bind group; what gets written is `MaterialDesc`,
/// and `gizmo_renderer::material_sync` converts at each end. This drives the half that needs no
/// GPU: a description written to a scene comes back with every value, including the texture path
/// the resolve step will rebuild the bind group from.
#[test]
fn a_material_description_survives_the_file() {
    use gizmo::prelude::*;
    use gizmo::renderer::components::{MaterialDesc, MaterialType};

    let mut world = World::new();
    let e = world.spawn();
    world.add_component(e, Transform::new(Vec3::ZERO));
    world.add_component(
        e,
        MaterialDesc {
            albedo: Vec4::new(0.2, 0.4, 0.6, 1.0),
            roughness: 0.3,
            metallic: 0.8,
            anisotropy: 0.0,
            clear_coat: 0.0,
            subsurface: 0.0,
            ambient: Vec3::ZERO,
            emissive: Vec3::new(0.0, 1.0, 0.0),
            texture_source: Some("textures/rust.png".to_string()),
            material_type: MaterialType::Pbr,
            is_transparent: false,
            is_double_sided: true,
        },
    );

    let path = std::env::temp_dir().join(format!("gizmo_material_{}.scene", std::process::id()));
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
    let descs = loaded.borrow::<MaterialDesc>();
    let desc = descs.get(id).expect("the material description survived");

    assert_eq!(desc.albedo, Vec4::new(0.2, 0.4, 0.6, 1.0), "the colour the user picked");
    assert_eq!((desc.roughness, desc.metallic), (0.3, 0.8), "and the PBR scalars");
    assert_eq!(desc.emissive, Vec3::new(0.0, 1.0, 0.0), "and the glow");
    assert_eq!(
        desc.texture_source.as_deref(),
        Some("textures/rust.png"),
        "and the texture path — which is what lets the load rebuild the bind group at all"
    );
    assert!(desc.is_double_sided, "and the flags");
}

/// **A navigation agent survives the file** — the whole point of registering it.
///
/// The editor has drawn an "AI NavAgent" section for as long as the component has existed, and
/// there was no way to put one on an entity from the editor at all (no ➕ menu entry, no add
/// handler) and no way for a scene to keep one. What travels is the tuning and the destination;
/// the route and the replan schedule are runtime state and are deliberately dropped.
#[test]
fn a_nav_agent_survives_the_file_with_its_tuning_and_not_its_route() {
    use gizmo::ai::components::{NavAgent, NavAgentState};
    use gizmo::prelude::*;

    let mut world = World::new();
    let e = world.spawn();
    world.add_component(e, Transform::new(Vec3::new(4.0, 0.0, -2.0)));

    let mut agent = NavAgent::new(2.5, 8.0, 1.25);
    agent.set_target(Vec3::new(30.0, 0.0, 12.0));
    agent.set_path(vec![Vec3::ZERO, Vec3::new(1.0, 0.0, 1.0)]);
    agent.state = NavAgentState::Moving;
    world.add_component(e, agent);

    let path = std::env::temp_dir().join(format!("gizmo_navagent_{}.scene", std::process::id()));
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

    let agents = loaded.borrow::<NavAgent>();
    let agent = agents.get(id).expect("the agent survived the round trip");
    assert_eq!(
        (agent.max_speed, agent.steering_force, agent.arrival_radius),
        (2.5, 8.0, 1.25),
        "the tuning is what an author sets, so it is what a file keeps"
    );
    assert_eq!(agent.target, Some(Vec3::new(30.0, 0.0, 12.0)));
    assert_eq!(agent.path_len(), 0, "a route through the old level is not data");
    assert_eq!(agent.state, NavAgentState::Idle);
}

/// Every `draw_*_section` the inspector calls, in the order the panel draws them.
///
/// Read from `inspector/mod.rs` rather than restated, for the same reason the ➕ list is read from
/// `setup.rs`: a section added tomorrow is classified the same day or the test fails.
fn inspector_section_fns(root: &Path) -> Vec<String> {
    let src = std::fs::read_to_string(root.join("crates/gizmo-editor/src/inspector/mod.rs"))
        .expect("gizmo-editor inspector/mod.rs");
    let mut names = Vec::new();
    for line in src.lines() {
        let line = line.trim();
        if line.starts_with("//") {
            continue;
        }
        let mut rest = line;
        while let Some(at) = rest.find("draw_") {
            rest = &rest[at..];
            let end = rest
                .find('(')
                .filter(|e| rest[..*e].chars().all(|c| c.is_alphanumeric() || c == '_'));
            match end {
                Some(e) if rest[..e].ends_with("_section") => {
                    names.push(rest[..e].to_string());
                    rest = &rest[e..];
                }
                _ => rest = &rest["draw_".len()..],
            }
        }
    }
    names.sort();
    names.dedup();
    names
}

/// **What the inspector lets you EDIT, the scene file has to keep** — the other half of the ➕ menu.
///
/// `every_addable_component_survives_a_save` drives its list from the ➕ menu, and that is exactly
/// the hole `MeshRenderer` came through: nobody adds one, `asset_loading` puts one beside every
/// mesh it loads, so it is not in the menu and the menu-driven check could not see it. Meanwhile
/// the inspector drew a full "Mesh Renderer" section — LOD bias, and `Cast shadow: On/Off/Only` —
/// and every value an artist set there went back to its default on the next save-and-reopen, with
/// nothing to say so. The editing surface is the user-visible surface; this is the list that
/// matches it.
///
/// A section must be in exactly one of the two tables below. That is the point: a new section
/// cannot be added without someone answering "and does a file keep this?".
#[test]
fn every_component_the_inspector_edits_survives_a_save() {
    /// (section fn, the component name the scene registry has to know).
    const EDITS: &[(&str, &str)] = &[
        ("draw_bone_attachment_section", "BoneAttachment"),
        ("draw_camera_section", "Camera"),
        ("draw_collider_section", "Collider"),
        ("draw_directional_light_section", "DirectionalLight"),
        ("draw_fighter_controller_section", "FighterController"),
        ("draw_hitbox_section", "Hitbox"),
        ("draw_hurtbox_section", "Hurtbox"),
        ("draw_mesh_renderer_section", "MeshRenderer"),
        ("draw_particle_emitter_section", "ParticleEmitter"),
        ("draw_point_light_section", "PointLight"),
        ("draw_rigidbody_section", "RigidBody"),
        ("draw_script_section", "Script"),
        ("draw_terrain_section", "Terrain"),
        ("draw_transform_section", "Transform"),
        ("draw_velocity_section", "Velocity"),
        ("draw_ai_section", "NavAgent"),
    ];

    /// (section fn, why the registry is not where its answer lives).
    const NOT_THE_REGISTRYS_JOB: &[(&str, &str)] = &[
        (
            "draw_name_section",
            "`EntityName` is written as a field of the entity record rather than as a registered \
             component — `SceneData` reads it back through `EntityName::new` on restore \
             (gizmo-scene/src/snapshot.rs). Registering it would write the name twice.",
        ),
        (
            "draw_material_section",
            "`Material` owns a live wgpu bind group. `MaterialDesc` is what travels, under its \
             own name, with `gizmo_renderer::material_sync` converting at each end.",
        ),
        (
            "draw_animation_player_section",
            "`AnimationPlayer` holds an `Arc<AnimationClip>` — a loaded asset, like `Mesh` — and \
             a `HashMap<String, Entity>` cache of resolved targets, whose ids do not survive a \
             reload. What could travel is a description (clip name, speed, looping), which does \
             not exist yet; the `Material`/`MaterialDesc` pair is the shape it would take.",
        ),
        (
            "draw_joint_section",
            "not a component at all: the section reads `PhysicsWorld::joints`, a resource. Joints \
             round-trip with the physics world, not with an entity's component set.",
        ),
        (
            "draw_reflection_section",
            "not one component: it is the generic fallback that draws every registered component \
             the sections above do not claim, through `ComponentRegistry`'s JSON hooks.",
        ),
        (
            "draw_fluid_section",
            "`FluidSimulation` is not registered ON PURPOSE while nothing consumes it. Measured \
             2026-08-19: the inspector is the only code in the workspace that reads its fields \
             (the component's own doc says so), and `Renderer::fluid_enabled` — the flag that \
             decides whether a single particle is simulated — is set by two demo binaries and by \
             nothing in the engine or the editor. Registering it would make a knob that steers \
             nothing persist, which reads as a closed gap and is not one. See docs/ENGINE.md.",
        ),
    ];

    let root = workspace_root();
    let sections = inspector_section_fns(&root);
    assert!(
        sections.len() > 15,
        "only {} inspector sections scanned — the parser has stopped matching the call shape, \
         which would make this test pass by seeing nothing: {sections:?}",
        sections.len()
    );

    let registry = gizmo::full_scene_registry();
    let saved: std::collections::BTreeSet<String> =
        registry.all_names().into_iter().map(str::to_string).collect();

    let mut unclassified = Vec::new();
    let mut lost = Vec::new();
    for section in &sections {
        let edits = EDITS.iter().find(|(s, _)| s == section);
        let exempt = NOT_THE_REGISTRYS_JOB.iter().any(|(s, _)| s == section);
        match (edits, exempt) {
            (Some((_, component)), false) => {
                if !saved.contains(*component) {
                    lost.push(format!("{section} edits {component}"));
                }
            }
            (None, true) => {}
            _ => unclassified.push(section.clone()),
        }
    }

    assert!(
        unclassified.is_empty(),
        "{unclassified:?} is drawn by the inspector and appears in neither table (or in both). \
         Say which component it edits, or why the scene registry is not where its answer lives — \
         an unanswered section is how `MeshRenderer` stayed lost."
    );
    assert!(
        lost.is_empty(),
        "the inspector edits {lost:?} and a save drops the values on the floor — silently, \
         because a component the registry does not know is simply not written. Register them in \
         `gizmo-app/src/scene_registry.rs`, or move them to `NOT_THE_REGISTRYS_JOB` with the \
         reason."
    );

    // An exemption that has stopped being true must go, or the list rots into decoration.
    let stale: Vec<&str> = NOT_THE_REGISTRYS_JOB
        .iter()
        .map(|(s, _)| *s)
        .filter(|s| {
            // `draw_x_section` → the component it would name, only for the ones that name one.
            matches!(*s, "draw_fluid_section") && saved.contains("FluidSimulation")
        })
        .collect();
    assert!(
        stale.is_empty(),
        "{stale:?} is listed as un-registered on purpose, but the registry knows its component \
         now — delete the entry and move the section to EDITS"
    );
}

/// **A mesh renderer's settings survive the file** — the loss this widening was written for.
///
/// Driven through a real save and load rather than a name comparison, because a registry entry is
/// not proof: the values have to come back. Both fields are set away from their defaults, so a
/// component that silently reverted would fail here rather than pass by coincidence.
#[test]
fn a_mesh_renderers_lod_bias_and_shadow_mode_survive_the_file() {
    use gizmo::prelude::*;
    use gizmo::renderer::components::{MeshRenderer, ShadowCasting};

    let mut world = World::new();
    let e = world.spawn();
    world.add_component(e, Transform::new(Vec3::new(4.0, 0.0, 0.0)));
    world.add_component(
        e,
        MeshRenderer::new()
            .with_lod_bias(3.0)
            .with_shadows(ShadowCasting::Only),
    );

    let path = std::env::temp_dir().join(format!("gizmo_meshrenderer_{}.scene", std::process::id()));
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

    let renderers = loaded.borrow::<MeshRenderer>();
    let r = renderers.get(id).expect("the mesh renderer survived");
    assert_eq!(
        (r.lod_bias, r.shadows),
        (3.0, ShadowCasting::Only),
        "the LOD bias and the shadow mode are what an artist sets, so they are what a file keeps"
    );
}
