//! **One render path.** The editor's panels are drawn by the game's.
//!
//! # What this was for, and what is left
//!
//! The engine had two renderers: the game's deferred path (`gizmo::systems::render`) and the
//! editor's forward one (`gizmo_studio::render_pipeline`). Until this file, nothing compared them
//! but a person looking at two windows. Everything that drifted between them was found by reading:
//! `BakedLit` routed by one path and defaulted by the other, a light array read from the raw
//! `Transform` here and `GlobalTransform` there, spot cone angles passed as radians to a shader
//! expecting cosines, the editor's depth of field linearising depth against a hardcoded range.
//! Each was a fix in two places, applied to one. So this file held the two paths side by side: the
//! scene setup each built, the per-frame drivers each called, and an inventory of every renderer
//! component each one named, with an exception list for what one path alone handled.
//!
//! Since 2026-11-22 both of the editor's panels are drawn through the game path
//! (`draw_camera_into`, with the `ViewOptions` an editor's view adds), and the studio's renderer is
//! gone, some 1 800 lines. There is nothing left to compare. What stays:
//! - the guard that keeps it that way (`the_studio_draws_both_panels_through_the_game_path`);
//! - the game path's own structure checks the comparison grew;
//! - the terrain recipe, which both hosts prepare.

/// The scene setup is built in one place. This is the ratchet: a render path that collects its own lights or fits its own
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

/// **Both hosts must turn a `Terrain` recipe into a mesh**, and it must be the same conversion.
///
/// The inventory below marks `Terrain` legitimate because it is a *recipe* and `Mesh` is what gets
/// drawn — which answers who draws it and takes for granted that something converts it. Something
/// did, in exactly one place: `gizmo-studio`'s render file, driven by an editor-only request queue
/// that only two edits push to (a slider moving, the ➕ menu adding the component). Neither is
/// true when a scene **loads**, and `Mesh` owns GPU buffers so no file can carry one — so a saved
/// level came back as an entity that says it is a terrain and draws nothing, and an exported
/// game, whose frame is `PlayLoop::step` + `default_render_pass`, never named `Terrain` at all.
///
/// Asserted at the two hosts rather than inside the draw loops because that is where the call
/// belongs: the conversion needs a `wgpu::Device`, and building geometry is not the batcher's job.
#[test]
fn both_hosts_build_the_mesh_a_terrain_recipe_names() {
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf();
    // The game builds it in its frame's preparation; the editor prepares its frame with that
    // same function.
    let hosts = [
        ("game", workspace.join("crates/gizmo/src/systems/render/mod.rs"), "terrain_mesh_system("),
        ("editor", workspace.join("crates/gizmo-studio/src/render_pipeline/mod.rs"), "prepare_frame(world, renderer)"),
    ];
    for (host, file, call) in hosts {
        let src = std::fs::read_to_string(&file)
            .unwrap_or_else(|e| panic!("{}: {e}", file.display()));
        assert!(
            code_only(&src).contains(call),
            "the {host} host never builds the mesh a `Terrain` names, so a scene that loads with \
             one shows an entity that draws nothing"
        );
    }
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

/// The editor's draw-path source **with the preparation it runs**: when the studio's pipeline
/// prepares its frame with the game's `prepare_frame`, the bodies of that function and of the frame
/// clocks it calls are code the editor runs, and count as what the editor path knows. Without the
/// call they are not added — a pipeline that stopped calling it would show up as the gaps it
/// leaves.
fn with_its_preparation(editor: String, workspace: &std::path::Path) -> String {
    if !code_only(&editor).contains("prepare_frame(world, renderer)") {
        return editor;
    }
    // Line endings normalised: a Windows checkout reads `\r\n`, and the end of a body is found
    // by its `\n}\n`.
    let game = std::fs::read_to_string(workspace.join("crates/gizmo/src/systems/render/mod.rs"))
        .expect("the game's frame source")
        .replace("\r\n", "\n");
    let body = |signature: &str| -> String {
        let start = game.find(signature).unwrap_or_else(|| panic!("`{signature}` is gone"));
        let end = game[start..].find("\n}\n").map(|e| start + e + 3).expect("its body ends");
        game[start..end].to_string()
    };
    // `apply_root_motion` too: `prepare_frame` calls it, and it is where `RootMotion` is read.
    let mut sources = format!(
        "{editor}\n{}\n{}\n{}",
        body("pub fn prepare_frame("),
        body("pub fn advance_frame_clocks("),
        body("pub fn apply_root_motion(")
    );
    // The game path's shared pass the editor calls for itself: its file is code it runs.
    if code_only(&sources).contains("record_sprites(") {
        sources.push('\n');
        sources.push_str(
            &std::fs::read_to_string(workspace.join("crates/gizmo/src/systems/render/sprite.rs"))
                .expect("the shared pass's source"),
        );
    }
    sources
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
        with_its_preparation(read_all(workspace.join("crates/gizmo-studio/src/render_pipeline")), &workspace),
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

/// **The studio draws both of its panels through the game path and records no pass of its own.**
///
/// It had a renderer of its own until 2026-11-22: a batcher, a cull loop, shadow, main and
/// transparent passes, and the copies of the game's rules they had to keep. Each rule was a fix in
/// two places, applied to one, and this file was the long list of what had to agree. The panels are
/// `draw_camera_into` now, so the comparison is the call itself: the Scene panel and the Game panel
/// both go through it, and nothing in the studio's render code sets a pipeline or draws. The one
/// pass it begins is the clear of the window's own surface, which frames the panels.
#[test]
fn the_studio_draws_both_panels_through_the_game_path() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/render_pipeline");
    let mut files = Vec::new();
    collect_rs(&dir, &mut files);
    assert!(!files.is_empty(), "no sources under {}", dir.display());
    let code: String = files
        .iter()
        .map(|f| std::fs::read_to_string(f).unwrap_or_default().replace("\r\n", "\n"))
        .map(|src| src.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n"))
        .collect::<Vec<_>>()
        .join("\n");
    let calls = code.matches("draw_camera_into(").count();
    assert!(calls >= 2, "the studio's panels are not both drawn through the game path ({calls} calls of `draw_camera_into`)");
    for pass_work in ["set_pipeline(", "draw_indexed(", ".draw(", "collect_scene_setup(", "record_shadow_passes("] {
        assert!(
            !code.contains(pass_work),
            "the studio's render code does `{pass_work}` itself: a second renderer is growing back"
        );
    }
    let passes = code.matches("begin_render_pass(").count();
    assert_eq!(passes, 1, "the studio begins {passes} render passes of its own; the one it needs clears the window");
}
