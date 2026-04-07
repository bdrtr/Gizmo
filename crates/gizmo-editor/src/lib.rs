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

/// Tüm editör panellerini tek çağrıyla çizer
pub fn draw_editor(ctx: &egui::Context, world: &World, state: &mut EditorState) {
    // 1. Toolbar (en üstte)
    if state.show_toolbar {
        toolbar::draw_toolbar(ctx, state);
    }
    
    // 2. Asset Browser (en altta)
    if state.show_asset_browser {
        asset_browser::draw_asset_browser(ctx, state);
    }
    
    // 3. Hierarchy (sol)
    if state.show_hierarchy {
        hierarchy::draw_hierarchy(ctx, world, state);
    }
    
    // 4. Inspector (sağ)
    if state.show_inspector {
        inspector::draw_inspector(ctx, world, state);
    }
}
