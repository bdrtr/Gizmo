//! Asset Browser — Alt panel'de proje dosyalarını gösterir

use crate::editor_state::EditorState;
use egui;
use std::path::Path;

/// Asset Browser sekmesini çizer
pub fn ui_asset_browser(ui: &mut egui::Ui, state: &mut EditorState) {
    if let Some(rx) = &state.assets.workspace_rx {
        if let Ok(path) = rx.lock().unwrap().try_recv() {
            state.assets.root = path;
        }
    }

    ui.horizontal(|ui| {
        ui.heading("🗂️ Asset Browser");
        ui.separator();

        // Geri git
        if ui.button("⬅").on_hover_text("Üst Dizin (Geri)").clicked() {
            if let Some(parent) = Path::new(&state.assets.root).parent() {
                state.assets.root = parent.to_string_lossy().to_string();
            }
        }

        // Workspace seçici
        if state.assets.workspace_rx.is_none() {
            if ui
                .button("📁 Workspace Aç")
                .on_hover_text("Bilgisayardan bir çalışma dizini seçin")
                .clicked()
            {
                let (tx, rx) = std::sync::mpsc::channel();
                state.assets.workspace_rx = Some(std::sync::Mutex::new(rx));
                std::thread::spawn(move || {
                    if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                        let _ = tx.send(folder.to_string_lossy().to_string());
                    }
                });
            }
        } else {
            let _ = ui
                .add_enabled(false, egui::Button::new("📁 Workspace Aç"))
                .on_hover_text("Dizin seçimi bekleniyor...");
        }

        ui.separator();

        ui.label("🔍");
        ui.text_edit_singleline(&mut state.assets.filter);
        ui.separator();

        // Breadcrumb tarzında yol gösterimi
        let current_root = state.assets.root.clone();
        ui.horizontal(|ui| {
            let components: Vec<_> = Path::new(&current_root).components().collect();
            let mut current_path = std::path::PathBuf::new();

            for (i, comp) in components.iter().enumerate() {
                current_path.push(comp);
                let part_str = comp.as_os_str().to_string_lossy();

                if ui.add(egui::Button::new(part_str).frame(false)).clicked() {
                    state.assets.root = current_path.to_string_lossy().to_string();
                }

                if i < components.len() - 1 {
                    ui.label("›"); // Breadcrumb separator
                }
            }
        });
    });

    // Hızlı aksiyon butonu satırı
    ui.horizontal(|ui| {
        if ui.small_button("📦 Sahneden Prefab Oluştur").clicked() {
            if let Some(&selected) = state.selection.entities.iter().next() {
                let path =
                    Path::new(&state.assets.root).join(format!("prefab_{}.prefab", selected));
                state.prefab_save_request = Some((selected, path.to_string_lossy().to_string()));
            } else {
                state.log_warning("Önce bir entity seçin.");
            }
        }
    });

    ui.separator();

    egui::ScrollArea::both().show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            let root = Path::new(&state.assets.root);
            if !root.exists() || !root.is_dir() {
                ui.label(
                    egui::RichText::new("⚠ Asset dizini bulunamadı").color(egui::Color32::YELLOW),
                );
                return;
            }

            let now = std::time::Instant::now();
            let mut need_refresh = true;
            if let Some((cached_path, last_update, _)) = &state.assets.cached_dir {
                if cached_path == &state.assets.root
                    && now.duration_since(*last_update).as_secs_f32() < 1.0
                {
                    need_refresh = false;
                }
            }

            if need_refresh {
                if let Ok(entries) = std::fs::read_dir(root) {
                    let mut file_cache = Vec::new();
                    for entry in entries.filter_map(|e| e.ok()) {
                        let is_dir = entry.path().is_dir();
                        let name = entry.file_name().to_string_lossy().to_string();
                        file_cache.push((entry.path(), name, is_dir));
                    }
                    file_cache.sort_by(|a, b| b.2.cmp(&a.2).then(a.1.cmp(&b.1)));
                    state.assets.cached_dir = Some((state.assets.root.clone(), now, file_cache));
                }
            }

            let file_entries = if let Some((_, _, cache)) = &state.assets.cached_dir {
                cache.clone()
            } else {
                return;
            };

            let filter_lower = state.assets.filter.to_lowercase();

            for (path, name, is_dir) in file_entries {
                // Filtre
                let name_lower = name.to_lowercase();
                if !filter_lower.is_empty() && !name_lower.contains(&filter_lower) {
                    continue;
                }

                let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
                let is_prefab = ext == "prefab";
                let is_scene = ext == "gizmo" || ext == "giz";
                let is_model = ext == "glb" || ext == "gltf" || ext == "obj";
                let is_script = ext == "lua";

                let icon = get_file_icon(&name);
                let path_str = path.to_string_lossy().to_string();

                ui.vertical(|ui| {
                    ui.set_width(82.0);

                    // Renk: prefab=yeşilimsi, scene=mavimsi, dir=sarı, script=turuncu
                    let icon_color = if is_dir {
                        egui::Color32::from_rgb(255, 220, 80)
                    } else if is_prefab {
                        egui::Color32::from_rgb(100, 255, 160)
                    } else if is_scene {
                        egui::Color32::from_rgb(100, 180, 255)
                    } else if is_script {
                        egui::Color32::from_rgb(255, 160, 80)
                    } else {
                        egui::Color32::from_rgb(200, 200, 200)
                    };

                    let btn_text = egui::RichText::new(icon).size(30.0).color(icon_color);
                    let response = ui.add(
                        egui::Button::new(btn_text)
                            .min_size(egui::vec2(72.0, 52.0))
                            .fill(egui::Color32::from_rgba_premultiplied(30, 30, 30, 180)),
                    );

                    // Tooltip (Reassign response because on_hover_text consumes it)
                    let response = response.on_hover_text(format!(
                        "{}\n{}",
                        name,
                        if is_prefab {
                            "Tek tık: Sahneye ekle"
                        } else if is_scene {
                            "Tek tık: Sahneyi yükle"
                        } else if is_dir {
                            "Çift tık: Klasöre gir"
                        } else {
                            "Sağ tık: Seçenekler"
                        }
                    ));

                    // Sağ tık menüsü en sonda atanacak

                    // Drag & Drop başlatma (viewport'ta yakalanır)
                    let drag_id = egui::Id::new("drag_asset").with(path.as_os_str());
                    let drag_response = ui.interact(response.rect, drag_id, egui::Sense::drag());
                    if drag_response.drag_started() {
                        state.dragged_asset = Some(path_str.clone());
                    }

                    if response.double_clicked() {
                        if is_dir {
                            // Klasöre gir (çift tık)
                            state.assets.root = path_str.clone();
                        }
                    } else if response.clicked() {
                        if is_prefab {
                            // ✅ TEK TIKLA prefab sahneye ekle
                            state.prefab_load_request = Some((path_str.clone(), None, None));
                            state.status_message = format!("Prefab eklendi: {}", name);
                        } else if is_scene {
                            // ✅ TEK TIKLA sahneyi yükle
                            if state.has_unsaved_changes {
                                state.scene.load_confirm_dialog = Some(path_str.clone());
                            } else {
                                state.scene.load_request = Some(path_str.clone());
                                state.status_message = format!("Sahne yükleniyor: {}", name);
                            }
                        } else if is_model {
                            state.status_message = format!("Seçilen: {} (Model)", name);
                        } else {
                            state.status_message = format!("Seçilen: {}", name);
                        }
                    }

                    // Dosya adı (kısa gösterim)
                    let char_count = name.chars().count();
                    let short_name = if char_count > 11 {
                        let truncated: String = name.chars().take(9).collect();
                        format!("{}...", truncated)
                    } else {
                        name.clone()
                    };
                    ui.label(
                        egui::RichText::new(short_name)
                            .small()
                            .color(egui::Color32::from_rgb(200, 200, 200)),
                    );

                    // Context Menu tüketimini güvenli hale getirmek için scope'un en sonunda çağrılır
                    response.context_menu(|ui| {
                        if is_model && ui.button("⚙️ Sahneye Ekle").clicked() {
                            state.spawn_asset_request = Some(path_str.clone());
                            ui.close_menu();
                        }
                        if is_prefab && ui.button("⚙️ Prefab Olarak Ekle").clicked() {
                            state.prefab_load_request = Some((path_str.clone(), None, None));
                            ui.close_menu();
                        }
                        if is_scene && ui.button("📂 Bu Sahneyi Yükle").clicked() {
                            state.scene.load_request = Some(path_str.clone());
                            ui.close_menu();
                        }
                        if ui.button("📋 Yolu Kopyala").clicked() {
                            ui.output_mut(|o| o.copied_text = path_str.clone());
                            ui.close_menu();
                        }
                    });
                });
            }
        });
    });
}

/// Dosya uzantısına göre ikon döndürür
fn get_file_icon(filename: &str) -> &'static str {
    let ext = filename.rsplit('.').next().unwrap_or("");
    if ext.eq_ignore_ascii_case("obj")
        || ext.eq_ignore_ascii_case("glb")
        || ext.eq_ignore_ascii_case("gltf")
        || ext.eq_ignore_ascii_case("fbx")
    {
        return "🗿";
    }
    if ext.eq_ignore_ascii_case("jpg")
        || ext.eq_ignore_ascii_case("jpeg")
        || ext.eq_ignore_ascii_case("png")
        || ext.eq_ignore_ascii_case("bmp")
        || ext.eq_ignore_ascii_case("tga")
    {
        return "🖼️";
    }
    if ext.eq_ignore_ascii_case("wav")
        || ext.eq_ignore_ascii_case("ogg")
        || ext.eq_ignore_ascii_case("mp3")
        || ext.eq_ignore_ascii_case("flac")
    {
        return "🔊";
    }
    if ext.eq_ignore_ascii_case("lua") {
        return "📜";
    }
    if ext.eq_ignore_ascii_case("json")
        || ext.eq_ignore_ascii_case("toml")
        || ext.eq_ignore_ascii_case("ron")
    {
        return "📋";
    }
    if ext.eq_ignore_ascii_case("prefab") {
        return "📦";
    }
    if ext.eq_ignore_ascii_case("gizmo") || ext.eq_ignore_ascii_case("giz") {
        return "🎬";
    }
    if ext.eq_ignore_ascii_case("wgsl")
        || ext.eq_ignore_ascii_case("glsl")
        || ext.eq_ignore_ascii_case("hlsl")
    {
        return "🎨";
    }
    if filename.contains('.') {
        return "📄";
    }
    "📁"
}
