//! The first automated cross-check between the engine's render path and the editor's.
//!
//! # What this is for
//!
//! The engine has two renderers — the game's deferred path (`gizmo::systems::render`) and the
//! editor's forward one (`gizmo_studio::render_pipeline`) — and until this file the only thing
//! comparing them was a person looking at two windows. Everything that drifted between them was
//! found by reading: `BakedLit` routed by one path and defaulted by the other, a light array read
//! from the raw `Transform` here and `GlobalTransform` there, spot cone angles passed as radians
//! to a shader expecting cosines, the editor's depth-of-field linearising depth against a
//! hardcoded range. Each was a fix in two places, applied to one.
//!
//! The pass recording genuinely differs and is not the target. What must not differ is the
//! *setup*: what the world says is in it. That now lives in one function,
//! [`collect_scene_setup`](gizmo::systems::render::collect_scene_setup), and the two paths reach
//! it with different arguments — so this file holds the two argument sets side by side and pins
//! the difference to exactly what each path declares.
//!
//! These tests need no GPU: setup is a pure function of the world and the camera.

use gizmo::core::World;
use gizmo::math::Vec3;
use gizmo::renderer::components::{DirectionalLight, LightRole, PointLight};
use gizmo::renderer::{CameraFrame, EnvironmentFrame, SceneUniforms};
use gizmo::systems::render::{collect_scene_setup, SceneSetup, SceneSetupInputs, ShadowCaster};
use gizmo::prelude::GlobalTransform;

fn a_camera() -> CameraFrame {
    CameraFrame {
        view_proj: gizmo::math::Mat4::perspective_rh(0.8, 1.6, 0.3, 900.0),
        position: Vec3::new(4.0, 6.0, 12.0),
        forward: Vec3::new(0.0, 0.0, -1.0),
        near: 0.3,
        far: 900.0,
        exposure: 1.25,
    }
}

/// The arguments the **game** path passes: cascades from the sun only, environment presets and
/// debug shading from the renderer, point-shadow cube rendered.
fn game_inputs() -> SceneSetupInputs {
    SceneSetupInputs {
        camera: a_camera(),
        aspect: 1.6,
        cam_fov: 0.8,
        shadow_caster: ShadowCaster::SunOnly,
        // Live renderer state in the real call; the values themselves are not the subject here.
        environment: EnvironmentFrame { preset: 2, preset_2: 3, blend_t: 0.5, shading_mode: 0 },
        point_shadows_enabled: true,
        elapsed_time: 7.5,
    }
}

/// The arguments the **editor** path passes: a shadow-casting fallback so a sunless scene still
/// shows shadows, no environment blend, debug shading from the viewport dropdown, and no
/// point-shadow cube because it records no such pass.
fn editor_inputs() -> SceneSetupInputs {
    SceneSetupInputs {
        camera: a_camera(),
        aspect: 1.6,
        cam_fov: 0.8,
        shadow_caster: ShadowCaster::SunOrFirstLight,
        environment: EnvironmentFrame { shading_mode: 4, ..Default::default() },
        point_shadows_enabled: false,
        elapsed_time: 7.5,
    }
}

fn sunlit_scene() -> World {
    let mut world = World::new();
    let sun = world.spawn();
    world.add_component(sun, GlobalTransform::default());
    world.add_component(
        sun,
        DirectionalLight { color: Vec3::new(1.0, 0.95, 0.9), intensity: 3.0, role: LightRole::Sun },
    );
    let lamp = world.spawn();
    world.add_component(lamp, GlobalTransform::default());
    world.add_component(lamp, PointLight::new(Vec3::new(0.2, 0.4, 1.0), 800.0, 25.0));
    world
}

fn both_ways(world: &World) -> (SceneSetup, SceneSetup) {
    (collect_scene_setup(world, &game_inputs()), collect_scene_setup(world, &editor_inputs()))
}

/// The heart of it: for one world and one camera, the two paths must agree about **everything the
/// world decides** — which lights exist, where they are, whether there is a sun, how the cascades
/// are split, what time it is — and may differ only where each path has said it differs.
///
/// The comparison is on the uploaded block rather than on the Rust struct, because the block is
/// what the shaders read; a divergence that survives into bytes is one the picture can show.
#[test]
fn the_two_paths_agree_on_everything_the_world_decides() {
    let world = sunlit_scene();
    let (game, editor) = both_ways(&world);
    let g = SceneUniforms::new(&game.frame);
    let e = SceneUniforms::new(&editor.frame);

    // ── The world's answer. Any disagreement here is a bug in one of the two paths. ──
    assert_eq!(g.lights, e.lights, "the two paths collected different lights");
    assert_eq!(g.num_lights, e.num_lights);
    assert_eq!(g.sun_direction, e.sun_direction, "sun direction, and the sun-present flag with it");
    assert_eq!(g.sun_color, e.sun_color);
    assert_eq!(g.cascade_splits, e.cascade_splits, "cascade splits come from the camera, not the path");
    assert_eq!(
        g.light_view_proj, e.light_view_proj,
        "this scene has a sun, so both paths fit the cascades to the same direction"
    );
    assert_eq!(g.cascade_params[0], e.cascade_params[0], "camera z-near");
    assert_eq!(g.cascade_params[1], e.cascade_params[1], "shadow texel size");
    assert_eq!(g.cascade_params[2], e.cascade_params[2], "elapsed time");
    assert_eq!(
        g.cascade_params[3], e.cascade_params[3],
        "the point-shadow caster index is the same light; only `point_shadows_enabled` decides \
         whether the shader looks at it"
    );
    assert_eq!(g.view_proj, e.view_proj);
    assert_eq!(g.camera_pos, e.camera_pos);
    assert_eq!(g.camera_forward, e.camera_forward);
    assert_eq!(g.inv_view_proj, e.inv_view_proj);
    assert_eq!(g.exposure, e.exposure);

    // ── The declared differences, asserted as differences so that removing one is also a test
    //    failure — a silent convergence is as much a surprise as a silent divergence. ──
    assert_eq!(g.point_shadows_enabled, 1, "the game renders the cube");
    assert_eq!(e.point_shadows_enabled, 0, "the editor records no cube pass");
    assert_ne!(g.shading_mode, e.shading_mode, "debug shading is the viewport's, not the scene's");
    assert_eq!(
        (e.environment_preset, e.environment_preset_2, e.environment_blend_t),
        (0, 0, 0.0),
        "the editor renders the scene as authored, with no environment preset blend"
    );

    // And nothing else. Every field of the block is either compared above or named here.
    assert_eq!(g._pre_align_pad, e._pre_align_pad);
    assert_eq!(g._align_pad, e._align_pad);
    assert_eq!(
        std::mem::size_of::<SceneUniforms>(),
        560 + gizmo::renderer::MAX_LIGHTS * 64,
        "a field was added to the block — decide here whether the two paths should agree on it \
         (the light array is the block's only variable part, so it is written against MAX_LIGHTS)"
    );
}

/// The editor's one real policy difference, and the reason it exists: a scene someone is still
/// lighting has no sun yet, and a viewport with no shadows at all reads as broken.
#[test]
fn the_editor_casts_from_a_light_when_the_scene_has_no_sun() {
    let mut world = World::new();
    let lamp = world.spawn();
    world.add_component(lamp, GlobalTransform::default());
    world.add_component(lamp, PointLight::new(Vec3::ONE, 900.0, 30.0));

    let (game, editor) = both_ways(&world);
    assert!(!game.lights.has_sun && !editor.lights.has_sun, "neither may claim a sun");
    assert_eq!(
        SceneUniforms::new(&game.frame).sun_direction[3],
        0.0,
        "no sun means the shader skips the sun branch in both paths"
    );
    assert_ne!(
        game.cascade_view_projs, editor.cascade_view_projs,
        "the editor fits its cascades to the lamp; the game leaves them on the default \
         down-vector because nothing will sample them"
    );
}

/// With nothing to cast from, the editor leaves the matrices at identity rather than shipping a
/// fit to a placeholder direction — the splits stay real, because the shadow-distance fade reads
/// them whether or not anything casts.
#[test]
fn an_unlit_scene_leaves_the_editor_cascades_at_identity() {
    let world = World::new();
    let (game, editor) = both_ways(&world);

    assert_eq!(editor.cascade_view_projs, [gizmo::math::Mat4::IDENTITY; 4]);
    assert_ne!(game.cascade_view_projs, [gizmo::math::Mat4::IDENTITY; 4], "the game always fits");
    let e = SceneUniforms::new(&editor.frame);
    assert!(e.cascade_splits.iter().all(|s| *s > 0.0), "splits stay real for the distance fade");
    assert_eq!(e.sun_direction[3], 0.0);
    assert_eq!(e.num_lights, 0);
}

/// The parity above is only worth anything while both paths actually go through the shared
/// helper. This is the ratchet: a render path that collects its own lights or fits its own
/// cascades has left the comparison, and no assertion in this file would notice.
#[test]
fn neither_render_path_builds_its_own_scene_setup() {
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("crates/gizmo-studio sits two levels below the workspace root")
        .to_path_buf();

    // The two render paths, and only them: `collect_scene_lights` is public API that a game with
    // its own renderer is meant to call, so this is not a workspace-wide ban.
    let paths = [
        workspace.join("crates/gizmo/src/systems/render"),
        workspace.join("crates/gizmo-studio/src/render_pipeline"),
    ];

    let mut offenders = Vec::new();
    let mut scanned = 0;
    for dir in &paths {
        let mut files = Vec::new();
        collect_rs(dir, &mut files);
        assert!(!files.is_empty(), "no sources under {}", dir.display());
        for file in files {
            // `shared.rs` is where the helper lives; it is supposed to call them.
            if file.file_name().is_some_and(|n| n == "shared.rs") {
                continue;
            }
            scanned += 1;
            let text = std::fs::read_to_string(&file).unwrap_or_default();
            for (i, line) in text.lines().enumerate() {
                if line.trim_start().starts_with("//") {
                    continue;
                }
                for call in ["collect_scene_lights(", "compute_directional_cascades("] {
                    if line.contains(call) {
                        offenders.push(format!(
                            "{}:{} — calls `{call}` directly instead of `collect_scene_setup`",
                            file.strip_prefix(&workspace).unwrap_or(&file).display(),
                            i + 1
                        ));
                    }
                }
            }
        }
    }

    assert!(scanned >= 5, "only {scanned} render-path sources scanned");
    assert!(
        offenders.is_empty(),
        "a render path stepped outside the shared setup, so the parity tests in this file no \
         longer cover it:\n  {}",
        offenders.join("\n  ")
    );

    // The argument sets above are this file's claim about what each path passes. Pin the one
    // argument that is a policy rather than a live value, so the claim cannot quietly stop being
    // true — a test comparing two argument sets nobody uses would pass forever.
    let sources = |dir: &std::path::Path| {
        let mut files = Vec::new();
        collect_rs(dir, &mut files);
        files
            .iter()
            // `shared.rs` happens to sit inside the game path's directory and is where the enum
            // is *defined*, so it names both variants; only call sites are the subject here.
            .filter(|f| f.file_name().is_some_and(|n| n != "shared.rs"))
            .map(|f| std::fs::read_to_string(f).unwrap_or_default())
            .collect::<String>()
    };
    let game = sources(&paths[0]);
    let editor = sources(&paths[1]);
    assert!(
        game.contains("ShadowCaster::SunOnly") && !game.contains("ShadowCaster::SunOrFirstLight"),
        "the game path no longer casts from the sun only — `game_inputs()` above is now fiction"
    );
    assert!(
        editor.contains("ShadowCaster::SunOrFirstLight") && !editor.contains("ShadowCaster::SunOnly"),
        "the editor path no longer falls back to a light — `editor_inputs()` above is now fiction"
    );
}

/// The two per-entity decisions the draw loops used to answer for themselves.
///
/// Both are settled now — `LodGroup::pick` decides which mesh an entity draws, `InstanceRaw::new`
/// packs the anisotropy/clear-coat/subsurface triple — and both were re-derived inline before
/// that. The packing is the one that had actually come apart: the engine packs two decimal digits
/// per field, studio still packed three, and three is the layout the engine abandoned because nine
/// digits exceed `f32`'s exact-integer range. `gbuffer.wgsl` decodes two digits, so studio's
/// instances would have decoded as a different material — inert only because the editor's forward
/// pipeline never reaches that shader.
///
/// A path that reaches past these answers is out of the comparison again, so it fails here.
#[test]
fn neither_draw_loop_answers_the_shared_per_entity_questions_itself() {
    let (game, editor) = draw_path_sources();
    let mut offenders = Vec::new();
    for (path, text) in [("game", &game), ("editor", &editor)] {
        // In CODE, not in a comment — a path that deleted the call and kept the note explaining it
        // would otherwise pass, which is the failure mode a comment-tolerant scan invites.
        let runs_instance_model = text
            .lines()
            .any(|l| !l.trim_start().starts_with("//") && l.contains("instance_model("));
        for (i, line) in text.lines().enumerate() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            // `select_mesh` is public API a game may call; inside a draw loop it means the
            // three-case answer (override / fall back / cull) is being rewritten.
            if line.contains(".select_mesh(") {
                offenders.push(format!(
                    "{path} path line {}: calls `select_mesh` directly — use `LodGroup::pick`, \
                     which also decides the no-group and past-the-last-level cases",
                    i + 1
                ));
            }
            if line.contains("InstanceRaw::new(") && !runs_instance_model {
                offenders.push(format!(
                    "{path} path line {}: builds instances without running the authored matrix \
                     through `backdrop::instance_model` — a PLACED backdrop then rides the camera, \
                     because the backdrop shader adds the camera position and nothing took it out",
                    i + 1
                ));
            }
            if line.contains("packed_pbr_params") {
                offenders.push(format!(
                    "{path} path line {}: assembles the packed PBR slot — `InstanceRaw::new` \
                     packs it, and it must agree with gbuffer.wgsl's decoder digit for digit",
                    i + 1
                ));
            }
        }
    }
    assert!(offenders.is_empty(), "draw loops answering shared questions again:\n  {}", offenders.join("\n  "));
}

/// Both paths size the instance buffer for the frame they are about to draw.
///
/// `Renderer::ensure_instance_capacity` existed, was unit-tested, and had exactly one caller —
/// the editor. The engine's path clamped its upload to whatever the buffer already held and
/// reported the truncated count, so past 8 192 instances a game dropped geometry while the editor
/// showed the same scene whole. The engine side has a real test now (9 000 cubes, counted at the
/// GPU); this is the cheap half that covers the editor too, and fails if either path stops asking.
#[test]
fn both_paths_size_the_instance_buffer_for_the_frame() {
    let (game, editor) = draw_path_sources();
    for (path, text) in [("game", &game), ("editor", &editor)] {
        // In CODE, not in a comment — the same rule its sibling above already applies. Without
        // it this test was vacuous: both draw paths carry long comments *about*
        // `ensure_instance_capacity`, so deleting the call and keeping the explanation left this
        // green, and the defect it names (geometry silently dropped past the buffer's capacity)
        // was reachable underneath it. Measured 2026-08-18.
        assert!(
            code_only(text).contains("ensure_instance_capacity("),
            "the {path} path uploads instances without asking for room — past the buffer's \
             capacity it will drop geometry rather than grow"
        );
    }
}

/// The capability inventory: a render component the engine exports must be known to **both** draw
/// paths, or be named below with the reason it is not.
///
/// This is the other half of the root these tests come from. The uniform block is shared and the
/// setup is shared, but what each loop *draws* is still two implementations, and the default state
/// of a new capability is "lives in exactly one path" — `LodGroup` and `ParticleEmitter` each
/// spent a while editor-only, `animation_state_machine_update_system` is engine-only, and the
/// sweep that went looking for unwired capabilities could not see any of them, because
/// `gizmo-studio` is a workspace member and a capability wired only into its pipeline still has an
/// in-tree consumer.
///
/// The subjects are **scanned** from the component modules, so a component added tomorrow is in
/// the inventory the same day. Only the exceptions are written by hand, and a stale one fails too:
/// an entry that has stopped being true must be deleted, or the list becomes the same rotting
/// hand-count as the tests this file replaced.
///
/// Subjects come at two granularities, because the first miss was at the second. Component
/// **types** are matched as words; the public **fields of `Material` and `Mesh`** are matched as
/// field *accesses* (`.name`), which is what distinguishes `mat.is_double_sided` from a local
/// called `radius`. Those two structs and not every component: a per-entity capability lives on
/// the material or the mesh, while `Camera::primary` or `PointLight::color` are read through
/// shared collectors and would be flagged only because two different structs share a field name.
/// Widening further would mean exception entries that record a scanner limitation as though it
/// were a design decision, which is the worst kind of entry to have in this list.
///
/// A name mentioned only in a comment counts as known for a type, and does not for a field — a
/// path that says in writing why it does not handle something has considered it, and for a field
/// the access is the only unambiguous evidence either way.
/// Every component type the renderer exports: defined in `gizmo-renderer/src/components`, or
/// re-exported there from another crate.
///
/// The re-export half was added on 2026-08-19 and is not a detail: the whole skeletal-animation
/// family reaches consumers that way, so none of those eight types had ever been a subject — and
/// one of them was a driver the editor never ran.
fn renderer_component_types(workspace: &std::path::Path) -> Vec<String> {
    let comp_dir = workspace.join("crates/gizmo-renderer/src/components");
    let mut comp_files = Vec::new();
    collect_rs(&comp_dir, &mut comp_files);
    let mut components = Vec::new();
    for file in &comp_files {
        let src = std::fs::read_to_string(file).unwrap_or_default();
        for line in src.lines() {
            for kw in ["pub struct ", "pub enum "] {
                if let Some(rest) = line.strip_prefix(kw) {
                    let name: String =
                        rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                    if !name.is_empty() {
                        components.push(name);
                    }
                }
            }
        }
        if let Some((_, rest)) = src.split_once("pub use gizmo_animation::skeletal::{") {
            if let Some((list, _)) = rest.split_once('}') {
                for name in list.split(',') {
                    let name = name.trim();
                    if name.starts_with(|c: char| c.is_ascii_uppercase()) {
                        components.push(name.to_string());
                    }
                }
            }
        }
    }
    components.sort();
    components.dedup();
    components
}

/// One draw path's source, with `shared.rs` left out and each file cut at its `#[cfg(test)]`.
///
/// `shared.rs` is excluded because what lives there is *neither* path in particular; the cut is
/// because a test naming a component is not the path handling it (a GPU guard spawning a
/// `PointLight` reported it as game-only, in this repo, on 2026-08-17).
fn draw_path_source(dir: std::path::PathBuf) -> String {
    let mut files = Vec::new();
    collect_rs(&dir, &mut files);
    files
        .iter()
        .filter(|f| f.file_name().is_some_and(|n| n != "shared.rs"))
        .map(|f| {
            let src = std::fs::read_to_string(f).unwrap_or_default();
            match src.find("#[cfg(test)]") {
                Some(i) => src[..i].to_string(),
                None => src,
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Whole-word: `Mesh` must not match `MeshRenderer`.
fn names_type(haystack: &str, needle: &str) -> bool {
    haystack.match_indices(needle).any(|(i, _)| {
        let before = haystack[..i].chars().next_back();
        let after = haystack[i + needle.len()..].chars().next();
        let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
        !word(before) && !word(after)
    })
}

/// The capabilities **neither** draw path names — the box the inventory below cannot judge.
fn silent_capabilities(workspace: &std::path::Path) -> Vec<String> {
    let game = draw_path_source(workspace.join("crates/gizmo/src/systems/render"));
    let editor = draw_path_source(workspace.join("crates/gizmo-studio/src/render_pipeline"));
    renderer_component_types(workspace)
        .into_iter()
        .filter(|c| !names_type(&game, c) && !names_type(&editor, c))
        .collect()
}

#[test]
fn every_render_capability_is_known_to_both_draw_paths() {
    /// Declared asymmetries: (component, which path, why).
    const EXCEPTIONS: &[(&str, Path, &str)] = &[
        (
            "EditorRenderTarget",
            Path::EditorOnly,
            "the editor's own viewport texture; a game has no second viewport to render into.",
        ),
        (
            "GameRenderTarget",
            Path::EditorOnly,
            "the play-mode preview texture inside the editor, for the same reason.",
        ),
        (
            "RenderStats",
            Path::EditorOnly,
            "what the last frame cost, published from the batch list the editor is about to draw \
             and read by its RENDER STATS overlay. The game path could publish the same numbers, \
             but nothing there reads them — and this type exists precisely because \
             `StudioState::draw_call_count` was a field nothing ever wrote. Publishing stats with \
             no consumer would be the same mistake facing the other way. Wire it into the game \
             path when a game overlay wants it, not before.",
        ),
        (
            "EditorOnly",
            Path::EditorOnly,
            "the marker that says an entity is the editor's own furniture — grid, light icons, \
             handles. The game path has no furniture to exclude, so there is nothing for it to \
             read. It exists because the editor draws two pictures from one world and only one of \
             them may contain this: without it the game view showed the editor's light icons, \
             since an icon is an ordinary unlit cube and no material flag can tell them apart.",
        ),
        (
            "lod_vbufs",
            Path::GameOnly,
            "mesh-INTERNAL level of detail: alternative vertex buffers flattened into one Mesh,              switched by distance in the engine's batcher. The editor's LOD is the `LodGroup`              component, which selects a different `Mesh` outright, so these buffers have nothing              to select between there. The consequence is that the viewport always shows the              highest detail — which is what an editor wants, and is why this stays one-sided.",
        ),
        (
            "lod_vertex_counts",
            Path::GameOnly,
            "the second half of `lod_vbufs` — a buffer without its vertex count is not usable,              so the two are one capability and are declined together.",
        ),
    ];

    #[derive(PartialEq, Debug, Clone, Copy)]
    enum Path {
        GameOnly,
        EditorOnly,
    }

    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf();

    // Subjects: every component type the renderer exports — see `renderer_component_types`,
    // which `every_capability_neither_path_names_is_accounted_for` shares so the two tests cannot
    // drift into judging different sets.
    let components = renderer_component_types(&workspace);
    assert!(components.len() > 15, "only {} components scanned", components.len());

    // …and the public FIELDS of `Material` and `Mesh`, because a capability does not have to be a
    // whole component. `is_double_sided` was a field, honoured by the editor and ignored by the
    // engine's deferred path for as long as both existed, and this test at component granularity
    // could not see it: `Material` itself is named by both paths on every line that reads a colour.
    let comp_dir = workspace.join("crates/gizmo-renderer/src/components");
    let mut fields = Vec::new();
    for (file, ty) in [("material.rs", "Material"), ("mesh.rs", "Mesh")] {
        let src = std::fs::read_to_string(comp_dir.join(file)).unwrap_or_default();
        let body = src
            .split_once(&format!("pub struct {ty} {{"))
            .and_then(|(_, rest)| rest.split_once("\n}"))
            .map(|(body, _)| body.to_string())
            .unwrap_or_else(|| panic!("{ty} struct"));
        for line in body.lines() {
            if let Some(rest) = line.trim_start().strip_prefix("pub ") {
                let name: String =
                    rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                if !name.is_empty() && rest[name.len()..].starts_with(':') {
                    fields.push(name);
                }
            }
        }
    }
    fields.sort();
    fields.dedup();
    assert!(fields.len() > 10, "only {} fields scanned", fields.len());

    // `shared.rs` sits inside the game path's directory but belongs to both, so a component named
    // there is named by neither in particular.
    //
    // Each file is cut at its `#[cfg(test)]`, because a test naming a component is not the path
    // *handling* it — and the difference is not hypothetical: light collection lives entirely in
    // `shared.rs` (excluded above, correctly), and this scan reported `PointLight` as
    // "game path only" the moment a GPU guard in `mod.rs`'s test module spawned one. Verified in
    // this repo on 2026-08-17; every one of these files keeps its tests as the tail, which the
    // assertion below holds true.
    let game = draw_path_source(workspace.join("crates/gizmo/src/systems/render"));
    let editor = draw_path_source(workspace.join("crates/gizmo-studio/src/render_pipeline"));
    // If a file ever puts production code *after* its tests, the cut above would silently stop
    // scanning it — so require that the cut removed every test module rather than assuming it.
    for (label, src) in [("game", &game), ("editor", &editor)] {
        assert!(
            !src.contains("#[cfg(test)]"),
            "{label} path: a `#[cfg(test)]` survived the cut — a file has tests that are not its \
             tail, and the scan is now reading them as capabilities"
        );
    }

    /// A field ACCESS, which a local variable of the same name cannot fake.
    fn accesses(haystack: &str, field: &str) -> bool {
        haystack.match_indices(&format!(".{field}")).any(|(i, _)| {
            let after = haystack[i + field.len() + 1..].chars().next();
            !after.is_some_and(|c| c.is_alphanumeric() || c == '_')
        })
    }

    let subjects: Vec<(&String, bool)> = components
        .iter()
        .map(|c| (c, false))
        .chain(fields.iter().map(|f| (f, true)))
        .collect();

    let mut undeclared = Vec::new();
    let mut stale = Vec::new();
    for (name, is_field) in subjects {
        let (g, e) = if is_field {
            (accesses(&game, name), accesses(&editor, name))
        } else {
            (names_type(&game, name), names_type(&editor, name))
        };
        let asymmetry = match (g, e) {
            (true, false) => Some(Path::GameOnly),
            (false, true) => Some(Path::EditorOnly),
            // Known to both, or to neither. "Neither" is a different question — nothing anywhere
            // draws it — and not one this test can answer from these two directories.
            _ => None,
        };
        let declared = EXCEPTIONS.iter().find(|(c, _, _)| c == name).map(|(_, p, _)| *p);
        match (asymmetry, declared) {
            (Some(actual), None) => undeclared.push(format!(
                "{name}: known to the {} path only. Wire it into the other, or add it to \
                 EXCEPTIONS with why it cannot be.",
                if actual == Path::GameOnly { "game" } else { "editor" }
            )),
            (Some(actual), Some(want)) if actual != want => undeclared.push(format!(
                "{name}: declared {want:?}, actually {actual:?} — the asymmetry flipped direction."
            )),
            (None, Some(want)) => stale.push(format!(
                "{name}: declared {want:?} but both paths know it now — delete the entry."
            )),
            _ => {}
        }
    }

    assert!(
        undeclared.is_empty() && stale.is_empty(),
        "render capability inventory:\n  {}\n  {}",
        undeclared.join("\n  "),
        stale.join("\n  ")
    );
}

/// The two render paths' sources as text, `(game, editor)`.
///
/// `shared.rs` sits inside the game path's directory but belongs to both, so it is not either
/// path's own code and is left out.
/// Source with its comments removed.
///
/// A positive `contains` over raw source is satisfied by a **comment**, and every assertion that
/// uses one exists because a line was deleted or made inert — the two states a comment cannot tell
/// apart. Audited 2026-08-18 by commenting the guarded line out: four such tests across the
/// workspace stayed green, this one among them.
///
/// `//` preceded by `:` is left alone so a `https://` inside a string is not mistaken for the
/// start of a comment. A trailing comment that *contains* the pattern is cut, which is the point.
fn code_only(src: &str) -> String {
    src.lines()
        .map(|line| {
            let mut end = line.len();
            let bytes = line.as_bytes();
            let mut i = 0;
            while i + 1 < bytes.len() {
                if bytes[i] == b'/' && bytes[i + 1] == b'/' && (i == 0 || bytes[i - 1] != b':') {
                    end = i;
                    break;
                }
                i += 1;
            }
            &line[..end]
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn draw_path_sources() -> (String, String) {
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf();
    let read_all = |dir: std::path::PathBuf| {
        let mut files = Vec::new();
        collect_rs(&dir, &mut files);
        assert!(!files.is_empty(), "no sources under {}", dir.display());
        files
            .iter()
            .filter(|f| f.file_name().is_some_and(|n| n != "shared.rs"))
            .map(|f| std::fs::read_to_string(f).unwrap_or_default())
            .collect::<Vec<_>>()
            .join("\n")
    };
    (
        read_all(workspace.join("crates/gizmo/src/systems/render")),
        read_all(workspace.join("crates/gizmo-studio/src/render_pipeline")),
    )
}

fn collect_rs(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
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

/// **Both draw paths must CALL the same per-frame drivers**, not merely know their names.
///
/// The capability inventory above counts a name in a comment as knowing a type, deliberately: a
/// component only ever touched through a system cannot be named any other way. That makes it the
/// wrong guard for this, because the comment explaining the drivers would satisfy it on its own —
/// which is exactly how the state machine stayed engine-only. So this one cuts comments and looks
/// for the calls.
///
/// Three defects of one shape, all found on 2026-08-19 and all invisible to the inventory above:
///
/// - `animation_state_machine_update_system` — engine-only. A state-machine-driven entity played
///   in an exported game and stood still in the viewport.
/// - `BoneAttachmentSystem` — editor-only, the mirror image. A sword parented to a hand bone
///   followed the animation in the viewport and stayed frozen in the shipped game.
/// - `gpu_physics_submit_system` / `gpu_physics_readback_system` — engine-only. A scene using GPU
///   rigid bodies simulated in the game and sat still in the editor, whose passes were already
///   drawing from `renderer.gpu_physics`.
///
/// The inventory could not see any of them: two of the three components are named in neither
/// path's source (they are reached through systems), and it only reports a capability known to
/// exactly ONE path — a capability known to neither is silence.
#[test]
fn both_draw_paths_run_the_same_per_frame_drivers() {
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf();

    for (path_name, file, clock) in [
        (
            "editor",
            "crates/gizmo-studio/src/render_pipeline/mod.rs",
            "animation_delta(world)",
        ),
        ("game", "crates/gizmo/src/systems/render/mod.rs", ".dt()"),
    ] {
        let src = std::fs::read_to_string(workspace.join(file)).expect("draw path source");
        let code: String = code_only(&src).chars().filter(|c| !c.is_whitespace()).collect();
        // Every per-frame driver either path runs. A capability driven by one picture and not
        // the other is the editor's oldest failure mode, and the list is short enough to state.
        for driver in [
            "ensure_global_transforms(",
            "animation_update_system(",
            "animation_state_machine_update_system(",
            "BoneAttachmentSystem",
            "gpu_physics_submit_system(",
            "gpu_physics_readback_system(",
        ] {
            assert!(
                code.contains(driver),
                "the {path_name} path does not run {driver} — whatever it drives moves in one \
                 picture and is frozen in the other"
            );
        }

        for driver in ["animation_update_system(", "animation_state_machine_update_system("] {
            assert!(
                code.contains(&format!("{driver}world,animation_dt,")),
                "the {path_name} path must advance animation by `animation_dt`, the delta both \
                 paths take from the clock — a raw frame delta here is a viewport that ignores \
                 `set_time_scale` and keeps walking through ⏸"
            );
        }
        assert!(
            code.contains(clock),
            "the {path_name} path must derive `animation_dt` from the clock ({clock})"
        );
    }
}

/// **What the capability inventory did not judge, and why.**
///
/// `every_render_capability_is_known_to_both_draw_paths` reports a capability known to exactly
/// **one** path. A capability named by **neither** falls out of it silently — and on 2026-08-19
/// that box held 20 of the 35 subjects, i.e. the inventory was judging fewer than half of them.
/// Two of the three one-sided drivers found that day sat in it (`BoneAttachment` and the GPU
/// physics link), because a component reached only through a system is named in neither path.
///
/// Making "neither" a failure outright would be wrong — most of the box is legitimate: lights come
/// from the shared collector this scan excludes on purpose, and half the entries are members of a
/// type that *is* judged. So the box is enumerated instead, with the reason each name is in it.
/// The set is asserted **exactly**: a new silent capability fails here, and so does an entry that
/// has stopped being silent, which is what keeps this from rotting into a list nobody rereads.
#[test]
fn every_capability_neither_path_names_is_accounted_for() {
    /// (capability, why neither draw path names it).
    const SILENT: &[(&str, &str)] = &[
        // ── Reached only through a driver both paths run ───────────────────────────────────
        ("ActiveBlend", "cross-fade state inside the animation drivers; the drivers are guarded by `both_draw_paths_run_the_same_per_frame_drivers`."),
        ("AnimationClip", "the clip data those drivers sample; same guard."),
        ("AnimationState", "a state machine's state; same guard."),
        ("AnimationTransition", "an edge between two of them; same guard."),
        ("SkeletonHierarchy", "the joint tree behind `Skeleton`, which both paths do name."),
        ("BoneAttachment", "read only inside `BoneAttachmentSystem` — which is precisely how it stayed editor-only until 2026-08-19. Both paths run that system now, and the driver guard is what holds them to it."),

        // ── Collected in `shared.rs`, which this scan excludes on purpose ──────────────────
        ("PointLight", "lights are gathered by the shared collector both paths call."),
        ("DirectionalLight", "same collector."),
        ("SpotLight", "same collector."),
        ("LightRole", "the tag that collector sorts by."),

        // ── A member of a type the inventory does judge ────────────────────────────────────
        ("LodLevel", "one level inside `LodGroup`, which both paths know."),
        ("ProjectionMode", "the enum behind `Camera::projection`, which both paths know."),
        ("Terrain", "a mesh *recipe*: the primitive builder turns it into a `Mesh`, and `Mesh` is what gets drawn."),
        ("RenderTarget", "an app-level render-to-texture handle (`gizmo-app`'s editor runtime), not a per-picture capability; the editor's own two are in EXCEPTIONS above."),

        // ── Genuinely one-sided, measured, and NOT a one-line fix ──────────────────────────
        ("FluidHandle", FLUID),
        ("FluidInteractor", FLUID),
        ("FluidParticle", FLUID),
        ("FluidPhase", FLUID),
        ("FluidPhaseType", FLUID),

        // ── No consumer at all ────────────────────────────────────────────────────────────
        ("Camera2D", "nothing renders from it: the engine ships no 2D pipeline, and neither path so much as reads it. The scene registry serialises it, so a scene can carry one that never draws. Trigger for wiring it: a 2D pipeline."),
    ];

    /// The fluid family shares one reason, and it is an open asymmetry rather than a benign one.
    const FLUID: &str = "the GPU fluid subsystem. The game path steps and draws it \
        (`renderer.gpu_fluid`, forward pass); the studio's pipeline does not mention it at all, so \
        a fluid scene renders in a shipped game and shows nothing in the viewport. Unlike the \
        drivers fixed on 2026-08-19 this is not a call away — the editor would need the fluid pass \
        itself. Trigger: authoring fluid in the editor.";

    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf();

    let measured = silent_capabilities(&workspace);
    let declared: std::collections::BTreeSet<&str> = SILENT.iter().map(|(n, _)| *n).collect();
    let measured_set: std::collections::BTreeSet<&str> =
        measured.iter().map(String::as_str).collect();

    let unaccounted: Vec<&&str> = measured_set.difference(&declared).collect();
    assert!(
        unaccounted.is_empty(),
        "neither draw path names {unaccounted:?}, and nothing here says why. Either a path should \
         be driving it — that is how `BoneAttachment` and the GPU physics link were found — or it \
         belongs in SILENT with the reason."
    );

    let stale: Vec<&&str> = declared.difference(&measured_set).collect();
    assert!(
        stale.is_empty(),
        "SILENT still lists {stale:?}, but a draw path names it now. Delete the entry — a list of \
         reasons that no longer apply is worse than no list."
    );
}
