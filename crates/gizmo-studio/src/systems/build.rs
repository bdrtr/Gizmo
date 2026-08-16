use gizmo::editor::{BuildTarget, EditorState};

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

pub fn handle_build_requests(editor_state: &mut EditorState) {
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

        std::thread::spawn(move || {
            let log = |msg: &str| {
                let _ = tx.send(msg.to_string());
            };

            // Hedefe göre cargo args belirle
            let (target_triple, exe_name, target_label) = match build_target {
                BuildTarget::Native => (
                    None,
                    if cfg!(windows) { "demo.exe" } else { "demo" },
                    "Native",
                ),
                BuildTarget::Linux => (Some("x86_64-unknown-linux-gnu"), "demo", "Linux (ELF)"),
                BuildTarget::Windows => {
                    (Some("x86_64-pc-windows-gnu"), "demo.exe", "Windows (.exe)")
                }
                BuildTarget::MacOs => (Some("x86_64-apple-darwin"), "demo", "macOS"),
                _ => (
                    None,
                    if cfg!(windows) { "demo.exe" } else { "demo" },
                    "Native",
                ),
            };

            log(&format!(
                "== [Adım 1/3] Gizmo Build Başlıyor — Hedef: {} ==",
                target_label
            ));

            let mut args = vec!["build", "--release", "-p", "demo"];
            let target_str;
            if let Some(triple) = target_triple {
                target_str = format!("--target={}", triple);
                args.push(&target_str);
                log(&format!("> cargo {}", args.join(" ")));
            } else {
                log("> cargo build --release -p demo");
            }

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

                        log("\n🎉 BUILD TAMAMLANDI! 🎉");
                        log("Oyununuz 'export/gizmo_game/' dizininde hazır.");
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
