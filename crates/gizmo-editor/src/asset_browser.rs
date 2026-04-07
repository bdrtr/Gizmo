//! Asset Browser — Alt panel'de proje dosyalarını gösterir

use egui;
use crate::editor_state::EditorState;
use std::path::Path;

/// Asset Browser panelini çizer
pub fn draw_asset_browser(ctx: &egui::Context, state: &mut EditorState) {
    egui::TopBottomPanel::bottom("asset_browser_panel")
        .default_height(150.0)
        .min_height(80.0)
        .max_height(300.0)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("🗂️ Asset Browser");
                ui.separator();
                ui.label("🔍");
                ui.text_edit_singleline(&mut state.asset_filter);
                ui.separator();
                ui.label(format!("📁 {}", state.asset_root));
            });
            ui.separator();
            
            egui::ScrollArea::horizontal().show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    // Asset dizinini tara
                    let root = Path::new(&state.asset_root);
                    if root.exists() && root.is_dir() {
                        if let Ok(entries) = std::fs::read_dir(root) {
                            let mut files: Vec<_> = entries
                                .filter_map(|e| e.ok())
                                .collect();
                            files.sort_by_key(|e| e.file_name());
                            
                            for entry in files {
                                let path = entry.path();
                                let name = entry.file_name().to_string_lossy().to_string();
                                
                                // Filtre
                                if !state.asset_filter.is_empty() && 
                                   !name.to_lowercase().contains(&state.asset_filter.to_lowercase()) {
                                    continue;
                                }
                                
                                // Dosya tipi ikonu
                                let icon = get_file_icon(&name);
                                let is_dir = path.is_dir();
                                
                                ui.vertical(|ui| {
                                    ui.set_width(80.0);
                                    
                                    let label_text = if is_dir {
                                        egui::RichText::new(icon.to_string()).size(28.0)
                                    } else {
                                        egui::RichText::new(icon.to_string()).size(28.0)
                                    };
                                    
                                    let response = ui.add(
                                        egui::Button::new(label_text)
                                            .min_size(egui::vec2(70.0, 50.0))
                                    );
                                    
                                    if response.clicked() {
                                        if is_dir {
                                            state.asset_root = path.to_string_lossy().to_string();
                                        } else {
                                            state.status_message = format!("Seçilen: {}", name);
                                        }
                                    }
                                    
                                    // Dosya adı (kısa gösterim)
                                    let short_name = if name.len() > 12 {
                                        format!("{}...", &name[..10])
                                    } else {
                                        name.clone()
                                    };
                                    ui.label(egui::RichText::new(short_name).small());
                                });
                            }
                        }
                    } else {
                        ui.label(egui::RichText::new("⚠ Asset dizini bulunamadı").color(egui::Color32::YELLOW));
                    }
                    
                    // Geri git butonu
                    if state.asset_root != "demo/assets"
                        && ui.button("⬆ Üst Dizin").clicked() {
                            if let Some(parent) = Path::new(&state.asset_root).parent() {
                                state.asset_root = parent.to_string_lossy().to_string();
                            }
                        }
                });
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
        "json" | "ron" | "toml" => "📋",
        "wgsl" | "glsl" | "hlsl" => "🎨",
        _ => "📄",
    }
}
