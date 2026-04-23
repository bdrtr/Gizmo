use gizmo::editor::{BuildTarget, EditorState};

use crate::update::copy_dir_all;
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
                    let stderr = child.stderr.take().unwrap();
                    let stdout = child.stdout.take().unwrap();

                    let tx_err = tx.clone();
                    let tx_out = tx.clone();

                    let stderr_thread = std::thread::spawn(move || {
                        use std::io::{BufRead, BufReader};
                        let reader = BufReader::new(stderr);
                        for line in reader.lines() {
                            if let Ok(l) = line {
                                let _ = tx_err.send(l);
                            }
                        }
                    });

                    let stdout_thread = std::thread::spawn(move || {
                        use std::io::{BufRead, BufReader};
                        let reader = BufReader::new(stdout);
                        for line in reader.lines() {
                            if let Ok(l) = line {
                                let _ = tx_out.send(l);
                            }
                        }
                    });

                    let status = child.wait().unwrap();
                    let _ = stderr_thread.join();
                    let _ = stdout_thread.join();

                    if status.success() {
                        log("\n== [Adım 2/3] Derleme Başarılı! Dosyalar Kopyalanıyor ==");
                        let export_dir = std::path::Path::new("export/gizmo_game");
                        let _ = std::fs::remove_dir_all(export_dir);
                        let _ = std::fs::create_dir_all(export_dir);

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
                            #[cfg(unix)]
                            {
                                use std::os::unix::fs::PermissionsExt;
                                if let Ok(metadata) = std::fs::metadata(&dst_exe) {
                                    let mut perms = metadata.permissions();
                                    perms.set_mode(0o755);
                                    let _ = std::fs::set_permissions(&dst_exe, perms);
                                }
                            }
                        }

                        log("\n== [Adım 3/3] Assetler Taşınıyor ==");
                        let _ = copy_dir_all("demo/assets", export_dir.join("assets"), &log);
                        log("Kopyalandı -> assets/");
                        let _ = copy_dir_all("demo/scenes", export_dir.join("scenes"), &log);
                        log("Kopyalandı -> scenes/");
                        let _ = copy_dir_all("demo/scripts", export_dir.join("scripts"), &log);
                        log("Kopyalandı -> scripts/");
                        let _ =
                            crate::update::copy_dir_all("media", export_dir.join("media"), &log);
                        log("Kopyalandı -> media/");

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
