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
pub mod editor_state;
pub mod gui;
pub mod hierarchy;
pub mod history;
pub mod inspector;
pub mod prefs;
pub mod toolbar;
pub mod scene_view;
pub mod game_view;
pub mod console;
pub mod windows;

pub use editor_state::{BuildTarget, EditorMode, EditorState, GizmoMode, EditorTab};
pub use gui::EditorContext;

use gizmo_core::World;
use egui_dock::{DockArea, TabViewer};

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
            EditorTab::BuildConsole => "Build".into(),
            EditorTab::Settings => "Ayarlar".into(),
            EditorTab::ScriptEditor => "Script Editor".into(),
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
            EditorTab::BuildConsole => windows::ui_build_console(ui, self.state),
            EditorTab::Settings => windows::ui_settings_window(ui, self.state),
            EditorTab::ScriptEditor => windows::ui_script_editor(ui, self.state),
        }
    }
}

/// Tüm editör panellerini tek çağrıyla çizer
pub fn draw_editor(ctx: &egui::Context, world: &World, state: &mut EditorState) {
    // ==== Asenkron İletişim (Dialog vb.) Olay Döngüsü ====
    if let Some(rx) = &state.pending_dialog_rx {
        match rx.try_recv() {
            Ok((is_save, Some(path_str))) => {
                state.scene_path = path_str.clone();
                if is_save {
                    state.status_message = format!("Sahne kaydediliyor → {}", path_str);
                    state.scene.save_request = Some(path_str);
                } else {
                    state.status_message = format!("Sahne yüklendi ← {}", path_str);
                    state.scene.load_request = Some(path_str);
                }
                state.pending_dialog_rx = None;
            }
            Ok((_, None)) | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                state.pending_dialog_rx = None;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {} // Still waiting
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
    let mut dock_state = std::mem::replace(&mut state.dock_state, egui_dock::DockState::new(vec![]));
    
    let mut viewer = EditorTabViewer {
        world,
        state,
    };

    DockArea::new(&mut dock_state)
        .style(egui_dock::Style::from_egui(ctx.style().as_ref()))
        .show(ctx, &mut viewer);
        
    viewer.state.dock_state = dock_state;

    // Her çerçevenin sonunda I/O optimizasyonu olarak prefs kirlendiyse dosyaya yaz
    state.prefs.flush_if_dirty();
}
