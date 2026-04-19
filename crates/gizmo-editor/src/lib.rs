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

pub use editor_state::{BuildTarget, EditorMode, EditorState, GizmoMode};
pub use gui::EditorContext;

use gizmo_core::World;
use egui_dock::{DockArea, TabViewer};

pub struct EditorTabViewer<'a> {
    pub world: &'a World,
    pub state: &'a mut EditorState,
}

impl<'a> TabViewer for EditorTabViewer<'a> {
    type Tab = String;

    fn title(&mut self, tab: &mut Self::Tab) -> egui::WidgetText {
        tab.as_str().into()
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Self::Tab) {
        match tab.as_str() {
            "Hierarchy" => {
                hierarchy::ui_hierarchy(ui, self.world, self.state);
            }
            "Inspector" => {
                inspector::ui_inspector(ui, self.world, self.state);
            }
            "Asset Browser" => {
                asset_browser::ui_asset_browser(ui, self.state);
            }
            "Scene View" => {
                scene_view::ui_scene_view(ui, self.world, self.state);
            }
            "Game View" => {
                game_view::ui_game_view(ui, self.state);
            }
            "Console" => {
                console::ui_console(ui);
            }
            _ => {
                ui.label(format!("Bilinmeyen Tab: {}", tab));
            }
        }
    }
}

/// Tüm editör panellerini tek çağrıyla çizer
pub fn draw_editor(ctx: &egui::Context, world: &World, state: &mut EditorState) {
    // 1. Toolbar (en üstte kalmaya devam etmeli, dock'un dışında)
    if state.show_toolbar {
        toolbar::draw_toolbar(ctx, state);
    }

    // Editör çiziminden hemen önce kamera çizim durumlarını resetleyelim
    state.scene_view_visible = false;
    state.game_view_visible = false;

    // 2. Docking Alanı (Geri kalan tüm alanı kaplar)
    let mut viewer = EditorTabViewer {
        world,
        state: unsafe { &mut *(state as *mut _) },
    };

    DockArea::new(&mut state.dock_state)
        .style(egui_dock::Style::from_egui(ctx.style().as_ref()))
        .show(ctx, &mut viewer);

    // 3. Build Konsolu (Yüzücü Pencere)
    windows::ui_build_console(ctx, state);

    // 4. Ayarlar Penceresi (Yüzücü)
    windows::ui_settings_window(ctx, state);

    // 5. Script Editor Penceresi (Yüzücü)
    windows::ui_script_editor(ctx, state);
}
