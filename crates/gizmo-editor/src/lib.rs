//! Gizmo Editor — Egui tabanlı sahne editörü
//!
//! ## Paneller
//! - **Toolbar** — Üst çubuk: Save/Load, Play/Pause, Gizmo modu
//! - **Hierarchy** — Sol panel: Entity ağacı
//! - **Inspector** — Sağ panel: Component düzenleyici
//! - **Asset Browser** — Alt panel: Dosya gezgini

pub mod gui;
pub mod editor_state;
pub mod hierarchy;
pub mod inspector;
pub mod toolbar;
pub mod asset_browser;

pub use gui::EditorContext;
pub use editor_state::{EditorState, GizmoMode, EditorMode};

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
                self.state.scene_view_visible = true;
                
                let response = ui.allocate_response(ui.available_size(), egui::Sense::hover());
                let rect = response.rect;

                ui.allocate_ui_at_rect(rect, |ui| {
                    ui.centered_and_justified(|ui| {
                        ui.label(egui::RichText::new("Gizmo Scene View").color(egui::Color32::from_white_alpha(50)));
                    });
                });

                // Dışarıdan veya UI'dan sürüklenen objeyi Scene View'a bırakma yakakalayıcısı
                if let Some(dragged_path) = ui.memory(|m| m.data.get_temp::<String>(egui::Id::new("dragged_asset_path"))) {
                    if response.hovered() && ui.input(|i| i.pointer.any_released()) {
                        self.state.spawn_asset_request = Some(dragged_path);
                        // İşaretçi bırakılan noktayı (Şimdilik None diyebiliriz, main raycast veya kamera kullanacak)
                        // Biz şimdilik özel bir tetikleyici olarak Some(ZERO) atayalım.
                        self.state.spawn_asset_position = Some(gizmo_math::Vec3::ZERO); 
                        
                        ui.memory_mut(|m| m.data.remove::<String>(egui::Id::new("dragged_asset_path")));
                    }
                }
            }
            "Game View" => {
                self.state.game_view_visible = true;
                ui.centered_and_justified(|ui| {
                    ui.label(egui::RichText::new("Gizmo Game View\n\n(Oyun Kamerası)").size(22.0).color(egui::Color32::from_white_alpha(50)));
                });
            }
            "Console" => {
                ui.heading("Geliştirici Konsolu");
                ui.separator();
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for (log, color) in &self.state.console_logs {
                            ui.label(egui::RichText::new(log).color(*color));
                        }
                    });
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
    
    // Editör çiziminden hemen önce kamera çizim durumlarını resetleyelim (Tab görünürlüğünü ölçeceğiz)
    state.scene_view_visible = false;
    state.game_view_visible = false;

    // 2. Docking Alanı (Geri kalan tüm alanı kaplar)
    let mut viewer = EditorTabViewer {
        world,
        state: unsafe { &mut *(state as *mut _) }, // Geçici ödünç alma bypass'ı çünkü state dock'a mutable gidiyor
    };
    
    DockArea::new(&mut state.dock_state)
        .style(egui_dock::Style::from_egui(ctx.style().as_ref()))
        .show(ctx, &mut viewer);
}
