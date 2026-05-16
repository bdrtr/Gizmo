//! Gizmo Editor — Egui tabanlı sahne editörü
//!
//! ## Paneller
//! - **Toolbar** — Üst çubuk: Save/Load, Play/Pause, Gizmo modu
//! - **Hierarchy** — Sol panel: Entity ağacı
//! - **Inspector** — Sağ panel: Component düzenleyici
//! - **Asset Browser** — Alt panel: Dosya gezgini
//! - **Scene View** — Orta panel: 3 Boyutlu sahne penceresi
//! - **Game View** — Oyunu oynarkenki pencere

pub mod asset_browser;
pub mod console;
pub mod editor_state;
pub mod game_view;
pub mod gui;
pub mod hierarchy;
pub mod history;
pub mod inspector;
pub mod prefs;
pub mod profiler_panel;
pub mod scene_view;
pub mod toolbar;
pub mod windows;

pub use editor_state::{BuildTarget, EditorMode, EditorState, EditorTab, GizmoMode};
pub use gui::EditorContext;

use egui_dock::{DockArea, TabViewer};
use gizmo_core::World;

pub struct EditorTabViewer<'a> {
    pub world: &'a World,
    pub state: &'a mut EditorState,
}

impl<'a> TabViewer for EditorTabViewer<'a> {
    type Tab = EditorTab;

    fn title(&mut self, tab: &mut Self::Tab) -> egui::WidgetText {
        match tab {
            EditorTab::Hierarchy => "Hierarchy".into(),
            EditorTab::Inspector => "Inspector".into(),
            EditorTab::AssetBrowser => "Asset Browser".into(),
            EditorTab::SceneView => "Scene".into(),
            EditorTab::GameView => "Game".into(),
            EditorTab::Console => "Console".into(),
            EditorTab::Settings => "Ayarlar".into(),
            EditorTab::ScriptEditor => "Script Editor".into(),
            EditorTab::Profiler => "⚡ Profiler".into(),
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Self::Tab) {
        match tab {
            EditorTab::Hierarchy => hierarchy::ui_hierarchy(ui, self.world, self.state),
            EditorTab::Inspector => inspector::ui_inspector(ui, self.world, self.state),
            EditorTab::AssetBrowser => asset_browser::ui_asset_browser(ui, self.state),
            EditorTab::SceneView => scene_view::ui_scene_view(ui, self.world, self.state),
            EditorTab::GameView => game_view::ui_game_view(ui, self.state),
            EditorTab::Console => console::ui_console(ui, self.state),
            EditorTab::Settings => windows::ui_settings_window(ui, self.state),
            EditorTab::ScriptEditor => windows::ui_script_editor(ui, self.state),
            EditorTab::Profiler => profiler_panel::ui_profiler(ui, self.world, self.state),
        }
    }
}

/// Tüm editör panellerini tek çağrıyla çizer
pub fn draw_editor(ctx: &egui::Context, world: &World, state: &mut EditorState) {
    // ==== Global Klavye Kısayolları (Sadece text alanları odakta değilken) ====
    if !ctx.wants_keyboard_input() {
        ctx.input(|i| {
            if i.key_pressed(egui::Key::Q) { state.gizmo_mode = GizmoMode::Select; }
            if i.key_pressed(egui::Key::W) { state.gizmo_mode = GizmoMode::Translate; }
            if i.key_pressed(egui::Key::E) { state.gizmo_mode = GizmoMode::Rotate; }
            if i.key_pressed(egui::Key::R) { state.gizmo_mode = GizmoMode::Scale; }
            // Delete kısayolu shortcuts.rs'de işleniyor (BUG-11 düzeltmesi: çift tetikleme önlendi)
        });
    }

    // ==== Asenkron İletişim (Dialog vb.) Olay Döngüsü ====
    let msg = if let Some(rx) = &state.pending_dialog_rx {
        match rx.lock().unwrap().try_recv() {
            Ok(v) => Some(v),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Some((false, None)),
            Err(_) => None,
        }
    } else {
        None
    };

    if let Some((is_save, opt_path)) = msg {
        if let Some(path_str) = opt_path {
            state.scene_path = path_str.clone();
            if is_save {
                state.status_message = format!("Sahne kaydediliyor → {}", path_str);
                state.scene.save_request = Some(path_str);
            } else {
                state.status_message = format!("Sahne yüklendi ← {}", path_str);
                state.scene.load_request = Some(path_str);
            }
        }
        state.pending_dialog_rx = None;
    }

    // ==== Ctrl+S ile tetiklenen kaydetme dialog isteği (shortcuts.rs'den gelir) ====
    if state.scene.request_save_dialog {
        state.scene.request_save_dialog = false;
        if state.pending_dialog_rx.is_none() {
            let (tx, rx) = std::sync::mpsc::channel();
            state.pending_dialog_rx = Some(std::sync::Mutex::new(rx));
            std::thread::spawn(move || {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let res = rfd::FileDialog::new()
                        .add_filter("Gizmo Scene", &["scene"])
                        .set_directory(".")
                        .save_file();
                    let _ = tx.send((
                        true,
                        res.map(|p: std::path::PathBuf| {
                            let s = p.to_string_lossy().to_string();
                            if s.starts_with(r"\\?\") {
                                s[4..].to_string()
                            } else {
                                s
                            }
                        }),
                    ));
                }
                #[cfg(target_arch = "wasm32")]
                let _ = tx.send((true, None));
            });
        }
    }

    // 1. Status Bar (En altta)
    egui::TopBottomPanel::bottom("status_bar")
        .exact_height(24.0)
        .show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                ui.label(egui::RichText::new(&state.status_message).weak().small());
            });
        });

    // 2. Toolbar (en üstte kalmaya devam etmeli, dock'un dışında)
    if state.show_toolbar {
        toolbar::draw_toolbar(ctx, state);
    }

    // Kamera çizim durumları dock içerisinde güncellenecek, frame sonunda/başında başka yerde sıfırlanmalıdır veya flag kilitlenmelidir.

    // 2. Docking Alanı (Geri kalan tüm alanı kaplar)
    let mut dock_state =
        std::mem::replace(&mut state.dock_state, egui_dock::DockState::new(vec![]));

    let mut viewer = EditorTabViewer { world, state };

    let mut dock_style = egui_dock::Style::from_egui(ctx.style().as_ref());
    dock_style.separator.width = 2.0;
    dock_style.separator.color_idle = egui::Color32::from_rgb(20, 20, 22);
    dock_style.separator.color_hovered = egui::Color32::from_rgb(64, 120, 240);
    dock_style.separator.color_dragged = egui::Color32::from_rgb(80, 140, 255);
    
    // Tab styling
    dock_style.tab_bar.bg_fill = egui::Color32::from_rgb(22, 22, 24);
    dock_style.tab.active.bg_fill = egui::Color32::from_rgb(34, 34, 36);
    dock_style.tab.inactive.bg_fill = egui::Color32::from_rgb(28, 28, 30);
    dock_style.tab.active.text_color = egui::Color32::WHITE;
    dock_style.tab.inactive.text_color = egui::Color32::from_rgb(150, 150, 150);

    DockArea::new(&mut dock_state)
        .style(dock_style)
        .show(ctx, &mut viewer);

viewer.state.dock_state = dock_state;

    // Handle delayed tab opening safely outside the dock tree loop
    if state.script.open {
        state.open_tab(EditorTab::ScriptEditor);
        state.script.open = false;
    }

    // Her çerçevenin sonunda I/O optimizasyonu olarak prefs kirlendiyse dosyaya yaz
    state.prefs.flush_if_dirty();
}
