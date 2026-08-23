use gizmo::editor::{BuildTarget, EditorState};
use gizmo::prelude::World;

use crate::update::copy_dir_all;

/// The directories an export ships beside the executable, as `(destination, source)`.
///
/// **Destination and source are the same string in every entry, and that is the invariant, not a
/// coincidence.** A scene file stores the path a reference was authored with, and the exported
/// binary makes its own directory the working directory — so a package that files an asset
/// somewhere other than where the scene names it has broken that reference, on every machine
/// including the one that built it. `every_export_dir_ships_to_the_path_the_scene_names` holds the
/// list to that rule.
///
/// The sources are the paths the *runtime* uses, which is not what this list said before:
///   * `demo/assets` — where the shaders and models the demo runtime loads live, and what the
///     asset browser writes into a scene: its workspace root is `demo/assets`, so a dragged
///     texture is stored as `demo/assets/foo.png`. This entry used to ship that tree as `assets/`
///     while nothing rewrote the references, so **every texture assigned through the browser was
///     missing in the exported game** — the shipped build opened untextured and the build log said
///     "🎉 BUILD TAMAMLANDI!". The runtime already scans both layouts
///     (`gizmo_runtime.rs`: `for root in ["assets", "demo/assets"]`), so shipping it under its own
///     name is what that code was already prepared for.
///   * `scripts` — `scripts/`, **not** `demo/scripts`. The editor stamps new scripts as
///     `Script::new("scripts/new_script.lua")` and `ScriptEngine::load_script` reads that path
///     straight through `std::fs::read_to_string`, i.e. relative to the working directory. The old
///     source was a directory nothing writes to and nothing reads from.
///   * `scenes` — `scenes/`, for the same reason: `demo/scenes` is written by nobody.
///   * `media` — `media/`, unchanged; that one was right.
///
/// `demo/scenes` and `demo/scripts` are both absent from the repository, so those two mistakes were
/// invisible: the copies quietly did nothing and the log said they had worked. The `assets` one was
/// the opposite kind — it copied a real tree, successfully, to a name nothing looked for.
///
/// A reference **outside** these four trees is handled separately and not by this list: see
/// [`audit_scene_assets`] for how it is found and [`ship_referenced_file`] for how it travels —
/// at its own relative path, so that nothing has to be rewritten. What still cannot travel (an
/// absolute path, a `.gltf` with sidecars) is named in the build log rather than left for the
/// player to discover.
const EXPORT_DIRS: [(&str, &str); 4] = [
    ("demo/assets", "demo/assets"),
    ("scenes", "scenes"),
    ("scripts", "scripts"),
    ("media", "media"),
];

/// What copying one export directory actually did.
#[derive(Debug)]
enum CopyOutcome {
    /// Copied, with the number of files that were under the source.
    Copied(usize),
    /// No such source directory. Normal for a project with no scripts; not a success either.
    SourceMissing,
    Failed(std::io::Error),
}

/// Copy one directory into the export, reporting what happened instead of discarding it.
fn copy_export_dir(src: impl AsRef<std::path::Path>, dst: std::path::PathBuf) -> CopyOutcome {
    let src = src.as_ref();
    if !src.is_dir() {
        return CopyOutcome::SourceMissing;
    }
    match copy_dir_all(src, dst) {
        Ok(()) => CopyOutcome::Copied(count_files(src)),
        Err(e) => CopyOutcome::Failed(e),
    }
}

/// Files under `dir`, recursively. Unreadable sub-directories count as empty rather than aborting:
/// this only ever produces a number for the log, and a wrong number must not cost a real export.
fn count_files(dir: &std::path::Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => count_files(&e.path()),
            Ok(_) => 1,
            Err(_) => 0,
        })
        .sum()
}

/// The log line for one copy. Split out from the build thread so it can be tested without cargo.
fn describe_copy(label: &str, src: &str, outcome: CopyOutcome) -> String {
    match outcome {
        CopyOutcome::Copied(n) => format!("Kopyalandı -> {label}/ ({n} dosya)"),
        CopyOutcome::SourceMissing => {
            format!("Atlandı -> {label}/ (kaynak '{src}' yok)")
        }
        CopyOutcome::Failed(e) => format!("HATA: {label}/ kopyalanamadı ('{src}'): {e}"),
    }
}

/// The binary an export ships: the runtime that opens a scene file, not a demo.
///
/// This used to be `demo`, whose default binary is `3d_scene` — a fixed floor, cube, light
/// and camera that reads no scene and runs no script. The export copied the user's `scenes/` and
/// `scripts/` next to it and nothing on the other side ever opened them, while the log said
/// "Oyununuz hazır". `gizmo_runtime` is the other side; see its module docs for the contract.
const RUNTIME_BIN: &str = "gizmo_runtime";

/// The scene file name the export writes, and the one [`RUNTIME_BIN`] opens with no argument.
/// The two are one contract — `the_exported_scene_lands_where_the_runtime_looks` holds them.
const EXPORT_SCENE_NAME: &str = "main.scene";

/// What one build target compiles to. Split out of the build thread because *which binary ships*
/// is the export's most consequential decision and it was previously a literal buried inside a
/// closure, where no test could see it.
struct BuildPlan {
    triple: Option<&'static str>,
    exe: &'static str,
    label: &'static str,
}

fn build_plan(target: BuildTarget) -> BuildPlan {
    let native_exe = if cfg!(windows) {
        "gizmo_runtime.exe"
    } else {
        RUNTIME_BIN
    };
    match target {
        BuildTarget::Linux => BuildPlan {
            triple: Some("x86_64-unknown-linux-gnu"),
            exe: RUNTIME_BIN,
            label: "Linux (ELF)",
        },
        BuildTarget::Windows => BuildPlan {
            triple: Some("x86_64-pc-windows-gnu"),
            exe: "gizmo_runtime.exe",
            label: "Windows (.exe)",
        },
        BuildTarget::MacOs => BuildPlan {
            triple: Some("x86_64-apple-darwin"),
            exe: RUNTIME_BIN,
            label: "macOS",
        },
        // `Native` and anything added to the enum later: build for this machine.
        _ => BuildPlan {
            triple: None,
            exe: native_exe,
            label: "Native",
        },
    }
}

/// The exact cargo invocation for a plan — a value, so the log can echo what actually runs and a
/// test can read which binary is being built without starting cargo.
fn cargo_args(plan: &BuildPlan) -> Vec<String> {
    let mut args = vec![
        "build".to_string(),
        "--release".to_string(),
        "-p".to_string(),
        "demo".to_string(),
        "--bin".to_string(),
        RUNTIME_BIN.to_string(),
    ];
    if let Some(triple) = plan.triple {
        args.push(format!("--target={triple}"));
    }
    args
}

/// Write the world the editor is showing to a temporary file, for the build thread to copy into
/// the export once cargo succeeds.
///
/// Two decisions worth stating. **Now, on this thread:** the build runs on a worker that cannot
/// borrow the world, so the alternative is shipping whatever was last saved to disk — which is
/// not what the user is looking at, and an export that silently ships an older scene is the same
/// class of lie this whole path was fixed for. **A temp file, not `scenes/main.scene`:** exporting
/// must not write into the project tree, least of all over a file the user may already keep there.
fn stage_scene_for_export(world: &World) -> Result<std::path::PathBuf, String> {
    let path = std::env::temp_dir().join(format!(
        "gizmo_export_scene_{}_{}.scene",
        std::process::id(),
        EXPORT_SCENE_NAME
    ));
    let as_str = path.to_string_lossy().to_string();
    gizmo::scene::SceneData::save(world, &as_str, &gizmo::full_scene_registry())
        .map(|()| path)
        .map_err(|e| e.to_string())
}

/// An asset the scene names that the exported game will not find.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingAsset {
    /// Which field named it, for a log line a user can act on.
    pub what: String,
    /// The path as the scene stores it.
    pub path: String,
    /// Why it will be missing.
    pub reason: MissingReason,
}

/// The two ways a reference fails an export, which need different words to the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissingReason {
    /// The file is there, and the package will not contain it — the silent case.
    WillNotShip,
    /// The file is not where the scene says, so it is already broken in the editor.
    Unresolved,
}

/// Which field a reference came from — the encodings differ, so the extraction does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RefKind {
    /// `MeshSource`: a bare path, an `obj:` prefix, a `gltf_mesh_<path><sub>` key, or the name of
    /// a built-in primitive that is no file at all.
    Mesh,
    /// A texture path — or a synthetic glTF texture name, which is also no file.
    Texture,
    /// A plain path: heightmap, script.
    Path,
    /// `AudioSource::sound_name`, which is a registered *name* first and a path second.
    Sound,
}

/// The built-in meshes that are generated rather than loaded. A scene naming one references no
/// file, and reporting it as a missing asset would be crying wolf on every default cube.
const PROCEDURAL_MESHES: [&str; 5] = [
    "inverted_cube",
    "plane",
    "standard_cube",
    "sphere",
    "sprite_quad",
];

/// The file a stored reference names, or `None` when it names no file at all.
///
/// **Every one of these encodings is a way to be wrong about what a scene references**, which is
/// why this is one function with a test rather than a match at the call site:
///
/// - a `MeshSource` may be a built-in primitive (`standard_cube`), an `obj:`-prefixed path, or a
///   glTF sub-mesh key with the file path baked into the middle of it
///   (`gltf_mesh_assets/car.glb_Body_p0`) — the key is split on the **extension**, not on a
///   separator, which is what `MeshSource::split_gltf_key` exists for;
/// - a texture reference may be the synthetic name a dropped glTF model writes
///   (`gltf_tex_base_0`), which is not a path and never was. That is not an edge case: it is what
///   every textured model dragged into the editor stores.
fn referenced_file(value: &str, kind: RefKind) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    match kind {
        RefKind::Mesh => {
            if PROCEDURAL_MESHES.contains(&value) {
                return None;
            }
            if let Some((path, _sub)) = gizmo::core::component::MeshSource::split_gltf_key(value) {
                return Some(path.to_string());
            }
            Some(value.strip_prefix("obj:").unwrap_or(value).to_string())
        }
        // `gltf_tex_` names a texture *inside* an imported model, resolved from the model rather
        // than from the filesystem.
        RefKind::Texture if value.starts_with("gltf_tex_") => None,
        RefKind::Texture | RefKind::Path | RefKind::Sound => Some(value.to_string()),
    }
}

/// Will the exported package contain the file at this path?
///
/// The package ships whole trees, each under the path it already has (see [`EXPORT_DIRS`]), and
/// the shipped binary makes the package its working directory. So a reference ships exactly when
/// it is **relative** and sits under one of those trees. An absolute path — which is what the
/// native file dialog behind "📁 Workspace Aç" returns, and what then gets stored in the scene —
/// never ships and would not resolve on another machine even if it did.
fn ships_with_export(path: &str) -> bool {
    let path = gizmo::renderer::asset::AssetManager::normalize_path(path);
    if std::path::Path::new(&path).is_absolute() {
        return false;
    }
    EXPORT_DIRS
        .iter()
        .any(|(_, src)| path == *src || path.starts_with(&format!("{src}/")))
}

/// Every asset the current scene names that the exported game will not find.
///
/// **This is the half of asset packaging that can be done correctly today.** The export copies
/// four whole trees and never looks at what the scene actually references, so anything outside
/// them — the repository's own `assets/`, or a directory chosen with "📁 Workspace Aç", which
/// hands back an absolute path — is silently absent from the package while the build log says
/// "🎉 BUILD TAMAMLANDI!". Copying those files in and rewriting the scene's paths is the other
/// half and is a larger change than it looks (a glTF's `.bin` and textures resolve relative to the
/// glTF, a rewritten path must also clear the asset UUID or the exported game's `repair_asset_paths`
/// converts it straight back, and `.prefab` files carry the same fields again). Naming what will be
/// missing needs none of that and turns a silent package into an honest one.
///
/// Walks the **live world** rather than the staged file: these are exactly the components the save
/// path reads, so what is audited is what will be written, and no parsing sits between the two.
fn audit_scene_assets(world: &World, project_root: &std::path::Path) -> Vec<MissingAsset> {
    let mut refs: Vec<(String, String, RefKind)> = Vec::new();

    {
        let meshes = world.borrow::<gizmo::core::component::MeshSource>();
        for (_, m) in meshes.iter() {
            refs.push(("Mesh".into(), m.0.clone(), RefKind::Mesh));
        }
    }
    {
        let mats = world.borrow::<gizmo::core::component::MaterialSource>();
        for (_, m) in mats.iter() {
            if let Some(t) = &m.texture_source {
                refs.push(("Materyal dokusu".into(), t.clone(), RefKind::Texture));
            }
        }
    }
    {
        let descs = world.borrow::<gizmo::renderer::components::MaterialDesc>();
        for (_, d) in descs.iter() {
            if let Some(t) = &d.texture_source {
                refs.push(("Materyal dokusu".into(), t.clone(), RefKind::Texture));
            }
        }
    }
    {
        let terrains = world.borrow::<gizmo::renderer::components::Terrain>();
        for (_, t) in terrains.iter() {
            refs.push(("Arazi heightmap".into(), t.heightmap_path.clone(), RefKind::Path));
        }
    }
    {
        let emitters = world.borrow::<gizmo::renderer::components::ParticleEmitter>();
        for (_, e) in emitters.iter() {
            if let Some(t) = &e.texture_source {
                refs.push(("Partikül dokusu".into(), t.clone(), RefKind::Texture));
            }
        }
    }
    {
        let scripts = world.borrow::<gizmo::scripting::Script>();
        for (_, s) in scripts.iter() {
            refs.push(("Script".into(), s.file_path.clone(), RefKind::Path));
        }
    }
    {
        let sounds = world.borrow::<gizmo::prelude::AudioSource>();
        for (_, s) in sounds.iter() {
            refs.push(("Ses".into(), s.sound_name.clone(), RefKind::Sound));
        }
    }

    let mut missing = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (what, value, kind) in refs {
        let Some(path) = referenced_file(&value, kind) else {
            continue;
        };
        if !seen.insert((what.clone(), path.clone())) {
            continue;
        }
        let on_disk = project_root.join(&path).exists();
        let reason = match (on_disk, ships_with_export(&path)) {
            (_, true) => continue,
            (true, false) => MissingReason::WillNotShip,
            // A sound name that is not a file is the documented normal case — the game registers
            // it with `load_sound`, and the scene only named it. Reporting those would bury the
            // real misses under every logical name in the project.
            (false, false) if kind == RefKind::Sound => continue,
            (false, false) => MissingReason::Unresolved,
        };
        missing.push(MissingAsset { what, path, reason });
    }
    missing
}

/// What happened when the export tried to take a referenced file with it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ShipOutcome {
    /// Copied into the package at the path the scene already stores.
    Copied,
    /// Deliberately not attempted, with the sentence the user gets.
    Skipped(&'static str),
    /// Attempted and failed.
    Failed(String),
}

/// Takes one referenced file into the package **at the path the scene already stores for it**.
///
/// **No path is rewritten, and that is the whole design.** Rewriting is where every hazard in this
/// area lives: a `MeshSource` may be a glTF sub-mesh key with the path buried in its middle, a
/// rewritten path must also clear its asset UUID or the exported game's `repair_asset_paths`
/// converts it straight back, and `.prefab` files carry the same fields again. A **relative**
/// reference needs none of that — the shipped binary makes the package its working directory, so
/// putting the file at the same relative path inside the package makes the stored path resolve
/// exactly as it does in the editor.
///
/// Two kinds are refused rather than half-done:
///
/// - **An absolute path** — what the native dialog behind "📁 Workspace Aç" returns and what then
///   gets stored — cannot be shipped without rewriting, and would not resolve on another machine
///   even if it were copied. It stays reported.
/// - **A `.gltf`** is not one file: its `.bin` buffers and its images are separate files named by
///   URIs inside it, resolved relative to the `.gltf` and free to climb out with `../`. Copying
///   only the `.gltf` would produce a package that looks complete and loads a model with no
///   geometry — worse than the honest warning, because it moves the failure from build time to
///   run time. (A `.glb` embeds all of it and ships fine. An `.obj`'s `.mtl` is discarded at load
///   — `tobj::load_obj`'s material half goes to `_` — so an `.obj` is self-contained *for this
///   engine* and does ship.)
fn ship_referenced_file(
    path: &str,
    project_root: &std::path::Path,
    export_dir: &std::path::Path,
) -> ShipOutcome {
    let source = std::path::Path::new(path);
    if source.is_absolute() {
        return ShipOutcome::Skipped(
            "mutlak yol — paketlenmesi sahnedeki yolun yeniden yazılmasını gerektirir",
        );
    }
    if source
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("gltf"))
    {
        return ShipOutcome::Skipped(
            "`.gltf` tek dosya değil — yanındaki .bin ve dokuları da taşınmalı (`.glb` kullanın \
             ya da modeli ihraç edilen bir dizine taşıyın)",
        );
    }
    // A stored path is data, and this routine WRITES with it. `..` survives `normalize_path` by
    // design, so `../../x` would place a file outside the export directory — the one directory
    // this build wiped and owns. Refused rather than sanitised: a reference that climbs out of the
    // project is not a thing to quietly relocate, it is a thing to tell the user about.
    if source
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return ShipOutcome::Skipped(
            "yol `..` içeriyor — paketin dışına çıkar, bu yüzden taşınmadı",
        );
    }

    let destination = export_dir.join(source);
    let prepared = match destination.parent() {
        Some(dir) => std::fs::create_dir_all(dir),
        None => Ok(()),
    };
    match prepared.and_then(|()| std::fs::copy(project_root.join(source), &destination)) {
        Ok(_) => ShipOutcome::Copied,
        Err(e) => ShipOutcome::Failed(e.to_string()),
    }
}

/// Acts on a build request from the toolbar: packages the current scene and the runtime for the
/// chosen target, and reports the result back into the editor state.
pub fn handle_build_requests(world: &World, editor_state: &mut EditorState) {
    // --- BUILD SİSTEMİ (STANDALONE EXPORTER) ---
    if editor_state.build.request {
        editor_state.build.request = false;
        editor_state
            .build
            .is_building
            .store(true, std::sync::atomic::Ordering::SeqCst);
        editor_state.build.cached_logs.clear();

        let is_building_flag = editor_state.build.is_building.clone();

        let (tx, rx) = std::sync::mpsc::channel();
        editor_state.build.logs_rx = Some(std::sync::Mutex::new(rx));
        let build_target = editor_state.build.target;
        // Captured before the thread starts — see `stage_scene_for_export`. The audit is captured
        // here for the same reason: it reads the live world, which the build thread cannot borrow.
        let staged_scene = stage_scene_for_export(world);
        let missing_assets = audit_scene_assets(world, std::path::Path::new("."));

        std::thread::spawn(move || {
            let log = |msg: &str| {
                let _ = tx.send(msg.to_string());
            };

            let plan = build_plan(build_target);
            let (target_triple, exe_name, target_label) = (plan.triple, plan.exe, plan.label);

            log(&format!(
                "== [Adım 1/3] Gizmo Build Başlıyor — Hedef: {} ==",
                target_label
            ));

            // One line, echoing the arguments actually about to run: the two branches used to
            // print different things and the no-target branch printed a command it was not
            // running.
            let args = cargo_args(&plan);
            log(&format!("> cargo {}", args.join(" ")));

            let mut command = std::process::Command::new("cargo");
            command
                .args(&args)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());

            match command.spawn() {
                Ok(mut child) => {
                    // Graceful: piped stdout/stderr normalde Some'dur; yine de panik yerine
                    // eksik handle'i atlayıp build thread'inin canlı kalmasını sağlıyoruz.
                    let mut stderr_thread = None;
                    let mut stdout_thread = None;

                    if let Some(stderr) = child.stderr.take() {
                        let tx_err = tx.clone();
                        stderr_thread = Some(std::thread::spawn(move || {
                            use std::io::{BufRead, BufReader};
                            let reader = BufReader::new(stderr);
                            for l in reader.lines().map_while(Result::ok) {
                                let _ = tx_err.send(l);
                            }
                        }));
                    }

                    if let Some(stdout) = child.stdout.take() {
                        let tx_out = tx.clone();
                        stdout_thread = Some(std::thread::spawn(move || {
                            use std::io::{BufRead, BufReader};
                            let reader = BufReader::new(stdout);
                            for l in reader.lines().map_while(Result::ok) {
                                let _ = tx_out.send(l);
                            }
                        }));
                    }

                    // child.wait() hata verirse panik yerine başarısız-durum gibi davran.
                    let status = match child.wait() {
                        Ok(s) => s,
                        Err(e) => {
                            log(&format!("\n!! Derleme süreci beklenirken hata: {} !!", e));
                            is_building_flag.store(false, std::sync::atomic::Ordering::SeqCst);
                            return;
                        }
                    };
                    if let Some(t) = stderr_thread {
                        let _ = t.join();
                    }
                    if let Some(t) = stdout_thread {
                        let _ = t.join();
                    }

                    if status.success() {
                        log("\n== [Adım 2/3] Derleme Başarılı! Dosyalar Kopyalanıyor ==");
                        let export_dir = std::path::Path::new("export/gizmo_game");
                        // `remove_dir_all` may legitimately fail (nothing there yet), so its result
                        // is dropped. `create_dir_all` may not: without the directory every copy
                        // below fails, and the run has nothing left to do but say so.
                        let _ = std::fs::remove_dir_all(export_dir);
                        if let Err(e) = std::fs::create_dir_all(export_dir) {
                            log(&format!(
                                "HATA: Çıktı dizini oluşturulamadı ({}): {}",
                                export_dir.display(),
                                e
                            ));
                            is_building_flag
                                .store(false, std::sync::atomic::Ordering::SeqCst);
                            return;
                        }

                        // Hedef triple varsa output target/TRIPLE/release/ altında olur
                        let src_base = if let Some(triple) = target_triple {
                            std::path::PathBuf::from("target")
                                .join(triple)
                                .join("release")
                        } else {
                            std::path::PathBuf::from("target/release")
                        };
                        let src_exe = src_base.join(exe_name);
                        let dst_exe = export_dir.join(exe_name);

                        if let Err(e) = std::fs::copy(&src_exe, &dst_exe) {
                            log(&format!(
                                "HATA: Executable kopyalanamadı ({:?}): {}",
                                src_exe, e
                            ));
                        } else {
                            log(&format!("Kopyalandı -> {:?}", dst_exe));
                            // On unix the copy does not carry the executable bit, so an export the
                            // user cannot run is exactly one silent `set_permissions` away.
                            #[cfg(unix)]
                            {
                                use std::os::unix::fs::PermissionsExt;
                                let marked = std::fs::metadata(&dst_exe).and_then(|meta| {
                                    let mut perms = meta.permissions();
                                    perms.set_mode(0o755);
                                    std::fs::set_permissions(&dst_exe, perms)
                                });
                                if let Err(e) = marked {
                                    log(&format!(
                                        "UYARI: Çalıştırma izni verilemedi ({:?}): {} — \
                                         `chmod +x` gerekebilir",
                                        dst_exe, e
                                    ));
                                }
                            }
                        }

                        log("\n== [Adım 3/3] Assetler Taşınıyor ==");
                        // Every one of these used to be `let _ = copy_dir_all(..)` followed by an
                        // unconditional "Kopyalandı ->", so the log reported four successes it had
                        // thrown the results of away. Two of the four sources did not even exist.
                        for (label, src) in EXPORT_DIRS {
                            log(&describe_copy(
                                label,
                                src,
                                copy_export_dir(src, export_dir.join(label)),
                            ));
                        }

                        // The scene the editor was showing, under the name the runtime opens with
                        // no argument. After the directory copies on purpose: `scenes/` may ship a
                        // project's other scenes, and this is the one the game starts in.
                        let scene_dst = export_dir.join("scenes").join(EXPORT_SCENE_NAME);
                        let scene_shipped = match &staged_scene {
                            Ok(src) => {
                                let placed = std::fs::create_dir_all(export_dir.join("scenes"))
                                    .and_then(|()| std::fs::copy(src, &scene_dst));
                                let _ = std::fs::remove_file(src);
                                match placed {
                                    Ok(_) => {
                                        log(&format!("Sahne yazıldı -> scenes/{EXPORT_SCENE_NAME}"));
                                        true
                                    }
                                    Err(e) => {
                                        log(&format!("HATA: sahne yazılamadı ({scene_dst:?}): {e}"));
                                        false
                                    }
                                }
                            }
                            Err(e) => {
                                log(&format!("HATA: açık sahne kaydedilemedi: {e}"));
                                false
                            }
                        };

                        // The assets the scene names from outside the four shipped trees. Each one
                        // that CAN travel is taken at the path the scene already stores for it —
                        // no rewriting, which is where every hazard in this area lives — and what
                        // is left is said out loud rather than left for the player to discover.
                        let mut unshipped: Vec<(&MissingAsset, String)> = Vec::new();
                        let mut taken = 0usize;
                        for m in &missing_assets {
                            match m.reason {
                                MissingReason::Unresolved => unshipped.push((
                                    m,
                                    "bu yolda dosya yok; sahne bunu editörde de açamıyor".into(),
                                )),
                                MissingReason::WillNotShip => {
                                    match ship_referenced_file(
                                        &m.path,
                                        std::path::Path::new("."),
                                        export_dir,
                                    ) {
                                        ShipOutcome::Copied => taken += 1,
                                        ShipOutcome::Skipped(why) => {
                                            unshipped.push((m, why.to_string()))
                                        }
                                        ShipOutcome::Failed(e) => {
                                            unshipped.push((m, format!("kopyalanamadı: {e}")))
                                        }
                                    }
                                }
                            }
                        }
                        if taken > 0 {
                            log(&format!(
                                "Sahnenin adlandırdığı {taken} varlık, ihraç edilen ağaçların \
                                 dışındaydı ve kendi göreli yoluyla pakete alındı"
                            ));
                        }

                        if !unshipped.is_empty() {
                            log(&format!(
                                "\n== UYARI: {} varlık pakete giremedi ==",
                                unshipped.len()
                            ));
                            for (m, why) in &unshipped {
                                log(&format!("  {} '{}' — {why}", m.what, m.path));
                            }
                            log(
                                "  Çözüm: bu dosyaları projenin ihraç edilen dizinlerinden birine \
                                 (demo/assets, scenes, scripts, media) taşıyıp sahnede yeniden \
                                 atayın.",
                            );
                        }

                        // The claim is made only where it is true. An export whose scene did not
                        // land still produces a runnable binary — it just opens an empty window,
                        // and saying "Oyununuz hazır" over that is the defect this path had. The
                        // same rule now covers a package whose assets are incomplete: it runs, and
                        // saying nothing about the missing ones is the same lie one size smaller.
                        if scene_shipped && !unshipped.is_empty() {
                            log(&format!(
                                "\n⚠ BUILD TAMAMLANDI — {} varlık eksik",
                                unshipped.len()
                            ));
                            log(&format!(
                                "Oyun 'export/gizmo_game/' dizininde çalışır ama yukarıdaki \
                                 varlıklar olmadan — çalıştır: ./{}",
                                exe_name
                            ));
                        } else if scene_shipped {
                            log("\n🎉 BUILD TAMAMLANDI! 🎉");
                            log(&format!(
                                "Oyununuz 'export/gizmo_game/' dizininde hazır — çalıştır: ./{}",
                                exe_name
                            ));
                        } else {
                            log("\n⚠ BUILD BİTTİ, AMA SAHNE GİTMEDİ.");
                            log(&format!(
                                "'export/gizmo_game/{}' çalışır durumda, ancak açacağı sahne yok: \
                                 boş pencere gelir.",
                                exe_name
                            ));
                        }
                    } else {
                        log("\n❌ HATA: Cargo derlemesi başarısız oldu.");
                    }
                }
                Err(e) => {
                    log(&format!("HATA: Cargo işlemi başlatılamadı: {}", e));
                }
            }

            is_building_flag.store(false, std::sync::atomic::Ordering::SeqCst);
        });
    }
}

#[cfg(test)]
mod export_copy_tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("gizmo_export_{}_{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dizini");
        dir
    }

    /// **What a stored reference actually names**, across every encoding a scene can hold.
    ///
    /// Each of these is a way for a packager — or an audit — to be wrong about what a scene
    /// references, and two of them are the common case rather than the exotic one: a glTF sub-mesh
    /// key has the file path buried in its middle, and every textured model dropped into the editor
    /// stores its texture as a synthetic name that is no file at all.
    #[test]
    fn a_reference_is_read_through_the_encoding_it_was_stored_in() {
        // Built-ins are generated, not loaded — reporting them would cry wolf on every cube.
        for builtin in PROCEDURAL_MESHES {
            assert_eq!(referenced_file(builtin, RefKind::Mesh), None, "{builtin}");
        }

        assert_eq!(
            referenced_file("gltf_mesh_assets/car.glb_Body_p0", RefKind::Mesh).as_deref(),
            Some("assets/car.glb"),
            "the path is in the middle of a glTF key, split on the extension"
        );
        assert_eq!(
            referenced_file("obj:models/rock.obj", RefKind::Mesh).as_deref(),
            Some("models/rock.obj")
        );
        assert_eq!(
            referenced_file("demo/assets/tree.glb", RefKind::Mesh).as_deref(),
            Some("demo/assets/tree.glb"),
            "a bare path is itself"
        );

        assert_eq!(
            referenced_file("gltf_tex_base_0", RefKind::Texture),
            None,
            "a texture inside an imported model is resolved from the model, not from disk — and \
             this is what EVERY textured model dragged into the editor stores"
        );
        assert_eq!(
            referenced_file("demo/assets/grass.jpg", RefKind::Texture).as_deref(),
            Some("demo/assets/grass.jpg")
        );

        assert_eq!(referenced_file("", RefKind::Path), None, "an unset field names nothing");
        assert_eq!(referenced_file("   ", RefKind::Path), None);
    }

    /// **Which paths the package will contain**: relative, and under a shipped tree.
    #[test]
    fn only_a_relative_path_under_a_shipped_tree_reaches_the_package() {
        assert!(ships_with_export("demo/assets/grass.jpg"));
        assert!(ships_with_export("./demo/assets/grass.jpg"), "a leading ./ is the same file");
        assert!(ships_with_export("scripts/player.lua"));
        assert!(ships_with_export("media/logo.png"));

        assert!(
            !ships_with_export("assets/grass.jpg"),
            "the repository's own assets/ is not one of the shipped trees"
        );
        assert!(
            !ships_with_export("demo/assetsX/grass.jpg"),
            "the prefix has to end at a path boundary, or a sibling directory would look shipped"
        );
        assert!(
            !ships_with_export("/home/someone/pictures/grass.jpg"),
            "an absolute path — what the native dialog behind 📁 Workspace Aç returns — never ships"
        );
    }

    /// **The audit, over a world that has one of each outcome.**
    ///
    /// The point is the third assertion as much as the first two: a sound name that is not a file
    /// is the documented normal case (the game registers it with `load_sound`, the scene only names
    /// it), and reporting those would bury the real misses under every logical name in the project.
    #[test]
    fn the_audit_names_what_will_be_missing_and_nothing_else() {
        use gizmo::prelude::*;

        let real = std::env::temp_dir().join(format!("gizmo_audit_{}.png", std::process::id()));
        std::fs::write(&real, b"not really a png").expect("scratch dosyası");

        let mut world = World::new();

        // (1) Exists, outside every shipped tree: the silent case this audit exists for.
        let outside = world.spawn();
        world.add_component(
            outside,
            gizmo::renderer::components::Terrain::new(
                real.to_string_lossy().to_string(),
                100.0,
                100.0,
                20.0,
            ),
        );

        // (2) Under a shipped tree: must not be reported, whether or not the test's working
        //     directory happens to contain it.
        let shipped = world.spawn();
        world.add_component(
            shipped,
            gizmo::scripting::Script::new("scripts/player.lua"),
        );

        // (3) A sound named rather than pathed — the game's to register, not the packager's.
        let sound = world.spawn();
        world.add_component(sound, gizmo::prelude::AudioSource::new("boom"));

        // (4) A path that resolves nowhere: already broken in the editor, worth saying so.
        let broken = world.spawn();
        world.add_component(
            broken,
            gizmo::core::component::MeshSource("assets/does_not_exist.glb".to_string()),
        );

        // (5) A built-in primitive: no file, no report.
        let cube = world.spawn();
        world.add_component(
            cube,
            gizmo::core::component::MeshSource("standard_cube".to_string()),
        );

        let missing = audit_scene_assets(&world, std::path::Path::new("."));
        let _ = std::fs::remove_file(&real);

        let reported: Vec<(&str, MissingReason)> = missing
            .iter()
            .map(|m| (m.path.as_str(), m.reason))
            .collect();

        assert!(
            reported.contains(&(real.to_string_lossy().as_ref(), MissingReason::WillNotShip)),
            "a file that exists and ships with nothing is the whole point: {reported:?}"
        );
        assert!(
            reported.contains(&("assets/does_not_exist.glb", MissingReason::Unresolved)),
            "a reference that resolves nowhere is worth a different sentence: {reported:?}"
        );
        assert_eq!(
            missing.len(),
            2,
            "the shipped script, the named sound and the built-in cube must all stay quiet: \
             {reported:?}"
        );
    }

    /// **A referenced file travels at the path the scene already stores for it** — which is what
    /// makes shipping it possible without rewriting anything.
    ///
    /// The rewriting is where every hazard in this area lives (a glTF sub-mesh key with the path
    /// buried in its middle, an asset UUID that would drag the path back to the development tree,
    /// `.prefab` files carrying the same fields again). A relative reference needs none of it: the
    /// shipped binary makes the package its working directory, so the same relative path resolves.
    #[test]
    fn a_relative_reference_is_copied_to_the_same_relative_path() {
        let export = scratch("ship_relative");
        let project = scratch("ship_project");
        let asset_dir = project.join("assets/textures");
        std::fs::create_dir_all(&asset_dir).expect("kaynak dizin");
        std::fs::write(asset_dir.join("grass.png"), b"pixels").expect("kaynak dosya");

        // The project root is a parameter rather than the process's working directory: changing
        // the cwd is global, and the other tests in this binary run in parallel with this one.
        let outcome = ship_referenced_file("assets/textures/grass.png", &project, &export);

        assert_eq!(outcome, ShipOutcome::Copied);
        assert_eq!(
            std::fs::read(export.join("assets/textures/grass.png")).expect("pakete alınmalı"),
            b"pixels",
            "the file has to land under the SAME relative path, or the stored reference misses it"
        );
    }

    /// The two kinds that are refused rather than half-done, and why each refusal is the honest
    /// answer instead of the lazy one.
    #[test]
    fn an_absolute_path_and_a_gltf_are_refused_with_a_reason() {
        let export = scratch("ship_refused");

        let absolute = std::env::temp_dir().join("gizmo_ship_abs.png");
        std::fs::write(&absolute, b"pixels").expect("kaynak dosya");
        let outcome = ship_referenced_file(&absolute.to_string_lossy(), &export, &export);
        let _ = std::fs::remove_file(&absolute);
        assert!(
            matches!(outcome, ShipOutcome::Skipped(_)),
            "an absolute path cannot ship without rewriting the scene, and would not resolve on \
             another machine even if it were copied: {outcome:?}"
        );

        assert!(
            matches!(
                ship_referenced_file("assets/models/car.gltf", &export, &export),
                ShipOutcome::Skipped(_)
            ),
            "a `.gltf` is not one file — copying only the .gltf would produce a package that looks \
             complete and loads a model with no geometry, moving the failure from build time to \
             run time"
        );
        assert!(
            !export.join("assets/models/car.gltf").exists(),
            "and a refusal must not leave half a model behind"
        );
    }

    /// **A stored path is data, and this routine writes with it.**
    ///
    /// `..` survives the engine's `normalize_path` by design, so a reference like `../../x` would
    /// place a file outside the export directory — the one directory the build wiped and owns.
    /// Refused rather than sanitised: a reference that climbs out of the project is something to
    /// tell the user about, not something to quietly relocate.
    #[test]
    fn a_reference_that_climbs_out_of_the_project_is_refused() {
        let export = scratch("ship_traversal");
        let project = scratch("ship_traversal_src");
        std::fs::write(project.join("secret.png"), b"pixels").expect("kaynak dosya");

        let outcome = ship_referenced_file("../secret.png", &project.join("sub"), &export);

        assert!(
            matches!(outcome, ShipOutcome::Skipped(_)),
            "a path with `..` must not be written through: {outcome:?}"
        );
        assert!(
            !export.parent().is_some_and(|p| p.join("secret.png").exists()),
            "and nothing may land beside the export directory"
        );
    }

    /// A `.glb` embeds its buffers and images, and an `.obj`'s `.mtl` is discarded at load, so both
    /// are self-contained **for this engine** and both ship. Asserted because the `.gltf` refusal
    /// above is one `extension()` check away from swallowing them too.
    #[test]
    fn a_glb_and_an_obj_are_self_contained_and_do_ship() {
        let export = scratch("ship_selfcontained");
        let project = scratch("ship_selfcontained_src");
        std::fs::create_dir_all(project.join("models")).expect("kaynak dizin");
        std::fs::write(project.join("models/car.glb"), b"glb").expect("glb");
        std::fs::write(project.join("models/rock.obj"), b"obj").expect("obj");

        let glb = ship_referenced_file("models/car.glb", &project, &export);
        let obj = ship_referenced_file("models/rock.obj", &project, &export);

        assert_eq!(glb, ShipOutcome::Copied, "a .glb embeds everything it needs");
        assert_eq!(obj, ShipOutcome::Copied, "and tobj discards the .mtl half anyway");
        assert!(export.join("models/car.glb").exists());
        assert!(export.join("models/rock.obj").exists());
    }

    /// **An export must file every asset where the scene names it.**
    ///
    /// A scene stores the path a reference was authored with, and the shipped binary makes its own
    /// directory the working directory — so a package that copies a tree to a *different* name has
    /// broken every reference into it, on the machine that built it as much as on any other. That
    /// is what `("assets", "demo/assets")` did: the asset browser's workspace root is
    /// `demo/assets`, so a dragged texture is stored as `demo/assets/foo.png`, and the export
    /// shipped it as `assets/foo.png`. Every texture assigned through the browser was missing in
    /// the exported game, and the build log ended with "🎉 BUILD TAMAMLANDI!".
    ///
    /// Asserted as a rule over the whole list rather than as one corrected entry, because the same
    /// mistake is available to the next directory anyone adds — and it is invisible in testing
    /// unless someone runs the exported binary from the export directory.
    #[test]
    fn every_export_dir_ships_to_the_path_the_scene_names() {
        let renamed: Vec<&str> = EXPORT_DIRS
            .iter()
            .filter(|(dest, src)| dest != src)
            .map(|(_, src)| *src)
            .collect();
        assert!(
            renamed.is_empty(),
            "these trees are shipped under a different name than the one a scene stores for them, \
             so every reference into them breaks in the exported game: {renamed:?}. Either ship \
             them under their own path, or rewrite the scene's references while staging."
        );
    }

    /// A copy that did nothing must not be logged as a copy that worked.
    ///
    /// This is the whole defect: the export ran `let _ = copy_dir_all(..)` and then printed
    /// "Kopyalandı -> scripts/" unconditionally. Two of the four sources did not exist, so a user
    /// exporting a game was told their scenes and scripts had shipped when nothing had — and the
    /// run finished with "🎉 BUILD TAMAMLANDI! 🎉" on top of it.
    #[test]
    fn a_missing_source_is_reported_as_skipped_not_copied() {
        let root = scratch("missing");
        let line = describe_copy(
            "scripts",
            "scripts",
            copy_export_dir(root.join("nothing_here"), root.join("out")),
        );

        assert!(
            line.contains("Atlandı") && line.contains("scripts"),
            "a missing source produced {line:?}"
        );
        assert!(
            !line.contains("Kopyalandı"),
            "a source that does not exist was reported as copied: {line:?}"
        );
        assert!(
            !root.join("out").exists(),
            "an absent source still created an empty destination directory"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A real copy reports the count, and the files are actually there — including nested ones.
    #[test]
    fn a_real_copy_reports_how_many_files_it_moved() {
        let root = scratch("real");
        let src = root.join("scripts");
        std::fs::create_dir_all(src.join("enemies")).expect("kaynak ağacı");
        std::fs::write(src.join("player.lua"), b"-- player").expect("dosya");
        std::fs::write(src.join("enemies/slime.lua"), b"-- slime").expect("iç dosya");

        let dst = root.join("out/scripts");
        let line = describe_copy("scripts", "scripts", copy_export_dir(&src, dst.clone()));

        assert!(line.contains("Kopyalandı") && line.contains('2'), "{line:?}");
        assert!(dst.join("player.lua").is_file(), "üst seviye dosya taşınmadı");
        assert!(
            dst.join("enemies/slime.lua").is_file(),
            "alt dizindeki dosya taşınmadı — recursion kırık"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A failed copy is reported as a failure, with the error attached.
    ///
    /// Provoked with a destination that already exists as a *file*: `copy_dir_all` starts with
    /// `create_dir_all(&dst)`, which cannot succeed over a regular file.
    #[test]
    fn a_failed_copy_is_reported_as_an_error() {
        let root = scratch("failed");
        let src = root.join("assets");
        std::fs::create_dir_all(&src).expect("kaynak");
        std::fs::write(src.join("a.txt"), b"a").expect("dosya");

        let dst = root.join("blocked");
        std::fs::write(&dst, b"a file, not a directory").expect("engel");

        let line = describe_copy("assets", "demo/assets", copy_export_dir(&src, dst));
        assert!(
            line.starts_with("HATA:") && line.contains("assets"),
            "a copy that could not run produced {line:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// **What ships is the runtime, not a demo.**
    ///
    /// The export ran `cargo build --release -p demo` and copied `demo`'s default binary, which is
    /// `3d_scene`: a fixed floor, cube, light and camera that opens no scene file and runs no
    /// script. Every target has to name the runtime, cross-compiled ones included — a Windows
    /// export shipping `demo.exe` is the same defect with a different extension.
    #[test]
    fn every_target_builds_and_ships_the_runtime() {
        for target in [
            BuildTarget::Native,
            BuildTarget::Linux,
            BuildTarget::Windows,
            BuildTarget::MacOs,
        ] {
            let plan = build_plan(target);
            assert!(
                plan.exe.starts_with(RUNTIME_BIN),
                "{:?} ships {:?}, which is not the runtime",
                target,
                plan.exe
            );

            let args = cargo_args(&plan);
            let bin = args.iter().position(|a| a == "--bin").map(|i| i + 1);
            assert_eq!(
                bin.and_then(|i| args.get(i)).map(String::as_str),
                Some(RUNTIME_BIN),
                "{:?} builds {:?} — without --bin, cargo builds demo's default binary",
                target,
                args
            );
            assert!(
                args.contains(&"--release".to_string()),
                "an exported game is a release build"
            );
        }
    }

    /// The export writes the scene where the runtime looks for it with no argument. Two constants
    /// in two crates, one contract — so this reads the other end rather than restating it.
    #[test]
    fn the_exported_scene_lands_where_the_runtime_looks() {
        let runtime = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../demo/src/bin/gizmo_runtime.rs");
        let src = std::fs::read_to_string(&runtime)
            .unwrap_or_else(|e| panic!("runtime kaynağı okunamadı ({runtime:?}): {e}"));
        assert!(
            src.contains(&format!("scenes/{EXPORT_SCENE_NAME}")),
            "the runtime no longer defaults to scenes/{EXPORT_SCENE_NAME}, so an exported game \
             would start with an empty window"
        );
    }

    /// The staged scene is the live world, and it is staged outside the project.
    ///
    /// Both halves matter: an export that ships the last *saved* scene ships something the user
    /// is not looking at, and an export that writes `scenes/main.scene` into the project tree
    /// overwrites a file the user may keep there.
    #[test]
    fn the_staged_scene_is_the_live_world_and_lands_outside_the_project() {
        use gizmo::core::component::EntityName;
        use gizmo::prelude::*;

        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, EntityName::new("Sahnedeki Kutu"));
        world.add_component(e, Transform::new(Vec3::new(1.0, 2.0, 3.0)));

        let staged = stage_scene_for_export(&world).expect("açık sahne kaydedilebilmeli");
        assert!(staged.is_file(), "staged scene {staged:?} yazılmadı");
        assert!(
            staged.starts_with(std::env::temp_dir()),
            "staging must not touch the project tree, but wrote {staged:?}"
        );

        let mut loaded = World::new();
        gizmo::scene::SceneData::load_into(
            &staged.to_string_lossy(),
            &mut loaded,
            &gizmo::full_scene_registry(),
        )
        .expect("staged sahne geri yüklenebilmeli");
        let _ = std::fs::remove_file(&staged);

        let names = loaded.borrow::<EntityName>();
        assert!(
            names.iter().any(|(_, n)| n.0 == "Sahnedeki Kutu"),
            "the world the editor was showing is not what got staged"
        );
    }

    /// The export ships the paths the runtime reads, not paths nobody writes.
    ///
    /// `demo/scenes` and `demo/scripts` were the old sources and neither exists in the repository
    /// — which is why copying from them looked fine. Scripts are the concrete case: the editor
    /// stamps `Script::new("scripts/new_script.lua")` and the engine resolves that against the
    /// working directory, so `scripts/` is the only source that can ship a project's scripts.
    #[test]
    fn the_export_reads_the_paths_the_runtime_uses() {
        let scripts = EXPORT_DIRS
            .iter()
            .find(|(label, _)| *label == "scripts")
            .expect("scripts is an export directory");
        assert_eq!(
            scripts.1, "scripts",
            "the script source must match the path `Script::new` stamps and `load_script` opens"
        );
        let spawned = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/systems/scene_ops.rs"),
        )
        .expect("scene_ops.rs sits next to this file");
        assert!(
            spawned.contains(&format!("Script::new(\"{}/", scripts.1)),
            "the editor no longer stamps scripts under {:?}, so the export source is stale",
            scripts.1
        );
        assert!(
            !EXPORT_DIRS.iter().any(|(_, src)| *src == "demo/scenes"
                || *src == "demo/scripts"),
            "an export source points at a directory nothing in the engine writes to"
        );
    }
}
