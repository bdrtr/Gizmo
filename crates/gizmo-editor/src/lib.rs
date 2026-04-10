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
pub use editor_state::{EditorState, GizmoMode, EditorMode, DragAxis};

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
                
                let response = ui.allocate_response(ui.available_size(), egui::Sense::click_and_drag());
                let rect = response.rect;

                self.state.scene_view_rect = Some(rect);

                if let Some(texture_id) = self.state.scene_texture_id {
                    let mut mesh = egui::Mesh::with_texture(texture_id);
                    mesh.add_rect_with_uv(
                        rect,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );
                    ui.painter().add(mesh);
                } else {
                    ui.allocate_ui_at_rect(rect, |ui| {
                        ui.centered_and_justified(|ui| {
                            ui.label(egui::RichText::new("Gizmo Scene View").color(egui::Color32::from_white_alpha(50)));
                        });
                    });
                }

                // --- GIZMO FARE (MOUSE) ETKİLEŞİMLERİ ---
                if let Some(hover_pos) = ui.input(|i| i.pointer.hover_pos()) {
                    if response.contains_pointer() || response.dragged() {
                        // Fare sahne içinde veya sürükleniyor ise NDC (-1.0 ile 1.0) hesapla
                        let nx = ((hover_pos.x - rect.left()) / rect.width()) * 2.0 - 1.0;
                        let ny = 1.0 - ((hover_pos.y - rect.top()) / rect.height()) * 2.0;

                        self.state.mouse_ndc = Some(gizmo_math::Vec2::new(nx, ny));

                        if response.clicked_by(egui::PointerButton::Primary) || response.drag_started_by(egui::PointerButton::Primary) {
                            self.state.do_raycast = true;
                        }
                        
                        // Sağ tık kamerayı çevirmek için (Egui ham input'u yuttuğu için burdan geçirmeliyiz)
                        if response.dragged_by(egui::PointerButton::Secondary) {
                            let delta = response.drag_delta();
                            self.state.camera_look_delta = Some(gizmo_math::Vec2::new(delta.x, delta.y));
                        } else {
                            self.state.camera_look_delta = None;
                        }
                    } else {
                        self.state.mouse_ndc = None;
                        self.state.camera_look_delta = None;
                    }
                }

                // Fareyi bırakırsa sürükleme (axis_drag) iptal olur
                if ui.input(|i| i.pointer.any_released()) && self.state.dragging_axis.is_some() {
                    self.state.dragging_axis = None;
                }

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
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        if let Ok(logs) = gizmo_core::logger::GLOBAL_LOGS.lock() {
                            for log in logs.iter() {
                                let color = match log.level {
                                    gizmo_core::logger::LogLevel::Info => egui::Color32::WHITE,
                                    gizmo_core::logger::LogLevel::Warning => egui::Color32::from_rgb(255, 200, 0),
                                    gizmo_core::logger::LogLevel::Error => egui::Color32::RED,
                                };
                                ui.label(egui::RichText::new(&log.message).color(color));
                            }
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
