//! Asset Browser — Alt panel'de proje dosyalarını gösterir

use crate::editor_state::EditorState;
use egui;
use std::path::Path;

/// Asset Browser sekmesini çizer
pub fn ui_asset_browser(ui: &mut egui::Ui, state: &mut EditorState) {
    ui.horizontal(|ui| {
        ui.heading("🗂️ Asset Browser");
        ui.separator();

        // Geri git
        if state.asset_root != "demo/assets" {
            if ui.button("⬅").on_hover_text("Üst Dizin (Geri)").clicked() {
                if let Some(parent) = Path::new(&state.asset_root).parent() {
                    state.asset_root = parent.to_string_lossy().to_string();
                }
            }
        }

        ui.label("🔍");
        ui.text_edit_singleline(&mut state.asset_filter);
        ui.separator();

        // Breadcrumb tarzında yol gösterimi
        let current_root = state.asset_root.clone();
        ui.horizontal(|ui| {
            let path_parts: Vec<&str> = current_root.split('/').collect();
            let mut current_path = String::new();

            for (i, part) in path_parts.iter().enumerate() {
                if i > 0 {
                    current_path.push('/');
                }
                current_path.push_str(part);

                if ui.add(egui::Button::new(*part).frame(false)).clicked() {
                    state.asset_root = current_path.clone();
                }

                if i < path_parts.len() - 1 {
                    ui.label("›"); // Breadcrumb separator
                }
            }
        });
    });

    // Hızlı aksiyon butonu satırı
    ui.horizontal(|ui| {
        if ui.small_button("📦 Sahneden Prefab Oluştur").clicked() {
            if let Some(&selected) = state.selected_entities.iter().next() {
                let path = format!("demo/assets/prefabs/prefab_{}.prefab", selected);
                state.prefab_save_request = Some((selected, path));
            } else {
                state.log_warning("Önce bir entity seçin.");
            }
        }
    });

    ui.separator();

    egui::ScrollArea::both().show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            let root = Path::new(&state.asset_root);
            if !root.exists() || !root.is_dir() {
                ui.label(
                    egui::RichText::new("⚠ Asset dizini bulunamadı").color(egui::Color32::YELLOW),
                );
                return;
            }

            let Ok(entries) = std::fs::read_dir(root) else {
                return;
            };
            let mut files: Vec<_> = entries.filter_map(|e| e.ok()).collect();
            files.sort_by(|a, b| {
                // Klasörler önce
                let a_dir = a.path().is_dir();
                let b_dir = b.path().is_dir();
                b_dir.cmp(&a_dir).then(a.file_name().cmp(&b.file_name()))
            });

            for entry in files {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();

                // Filtre
                if !state.asset_filter.is_empty()
                    && !name
                        .to_lowercase()
                        .contains(&state.asset_filter.to_lowercase())
                {
                    continue;
                }

                let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
                let is_dir = path.is_dir();
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

                    // Tooltip
                    response.clone().on_hover_text(format!(
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

                    // Sağ tık menüsü
                    response.context_menu(|ui| {
                        if is_model {
                            if ui.button("⚙️ Sahneye Ekle").clicked() {
                                state.spawn_asset_request = Some(path_str.clone());
                                ui.close_menu();
                            }
                        }
                        if is_prefab {
                            if ui.button("⚙️ Prefab Olarak Ekle").clicked() {
                                state.prefab_load_request = Some((path_str.clone(), None, None));
                                ui.close_menu();
                            }
                        }
                        if is_scene {
                            if ui.button("📂 Bu Sahneyi Yükle").clicked() {
                                state.scene_load_request = Some(path_str.clone());
                                ui.close_menu();
                            }
                        }
                        if ui.button("📋 Yolu Kopyala").clicked() {
                            ui.output_mut(|o| o.copied_text = path_str.clone());
                            ui.close_menu();
                        }
                    });

                    // Drag & Drop başlatma
                    let drag_id = egui::Id::new("drag_asset").with(&path);
                    let drag_response = ui.interact(response.rect, drag_id, egui::Sense::drag());
                    if drag_response.drag_started() {
                        ui.memory_mut(|m| {
                            m.data
                                .insert_temp(egui::Id::new("dragged_asset_path"), path_str.clone())
                        });
                    }

                    if response.double_clicked() {
                        if is_dir {
                            // Klasöre gir (çift tık)
                            state.asset_root = path_str.clone();
                        }
                    }

                    // Tek tıklama mantığı
                    if response.clicked() {
                        if is_prefab {
                            // ✅ TEK TIKLA prefab sahneye ekle
                            state.prefab_load_request = Some((path_str.clone(), None, None));
                            state.status_message = format!("Prefab eklendi: {}", name);
                        } else if is_scene {
                            // ✅ TEK TIKLA sahneyi yükle
                            state.scene_load_request = Some(path_str.clone());
                            state.status_message = format!("Sahne yükleniyor: {}", name);
                        } else {
                            state.status_message = format!("Seçilen: {}", name);
                        }
                    }

                    // Dosya adı (kısa gösterim)
                    let short_name = if name.len() > 11 {
                        format!("{}...", &name[..9])
                    } else {
                        name.clone()
                    };
                    ui.label(
                        egui::RichText::new(short_name)
                            .small()
                            .color(egui::Color32::from_rgb(200, 200, 200)),
                    );
                });
            }
        });
    });
}

/// Dosya uzantısına göre ikon döndürür
fn get_file_icon(filename: &str) -> &'static str {
    let ext = filename.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "obj" | "glb" | "gltf" | "fbx" => "🗿",
        "jpg" | "jpeg" | "png" | "bmp" | "tga" => "🖼️",
        "wav" | "ogg" | "mp3" | "flac" => "🔊",
        "lua" => "📜",
        "json" | "toml" | "ron" => "📋",
        "prefab" => "📦",
        "gizmo" | "giz" => "🎬",
        "wgsl" | "glsl" | "hlsl" => "🎨",
        _ if filename.contains('.') => "📄",
        _ => "📁", // Kazıca veya dizin
    }
}
