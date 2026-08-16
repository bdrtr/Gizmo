use gizmo::editor::{BuildTarget, EditorState};
use gizmo::prelude::World;

use crate::update::copy_dir_all;

/// The directories an export ships beside the executable, as `(destination, source)`.
///
/// The sources are the paths the *runtime* uses, which is not what this list said before:
///   * `assets` — `demo/assets`, which is where the shaders and models the demo runtime loads live.
///   * `scripts` — `scripts/`, **not** `demo/scripts`. The editor stamps new scripts as
///     `Script::new("scripts/new_script.lua")` and `ScriptEngine::load_script` reads that path
///     straight through `std::fs::read_to_string`, i.e. relative to the working directory. The old
///     source was a directory nothing writes to and nothing reads from.
///   * `scenes` — `scenes/`, for the same reason: `demo/scenes` is written by nobody.
///   * `media` — `media/`, unchanged; that one was right.
///
/// `demo/scenes` and `demo/scripts` are both absent from the repository, so the two mistakes were
/// invisible: the copies quietly did nothing and the log said they had worked.
const EXPORT_DIRS: [(&str, &str); 4] = [
    ("assets", "demo/assets"),
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
/// This used to be `demo`, whose default binary is `bevy_3d_scene` — a fixed floor, cube, light
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
        // Captured before the thread starts — see `stage_scene_for_export`.
        let staged_scene = stage_scene_for_export(world);

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

                        // The claim is made only where it is true. An export whose scene did not
                        // land still produces a runnable binary — it just opens an empty window,
                        // and saying "Oyununuz hazır" over that is the defect this path had.
                        if scene_shipped {
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
    /// `bevy_3d_scene`: a fixed floor, cube, light and camera that opens no scene file and runs no
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
