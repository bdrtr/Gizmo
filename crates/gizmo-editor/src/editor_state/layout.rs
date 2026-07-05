//! EditorState — dock/tab layout: open/close tabs, reset, save & load.
use super::*;

impl EditorState {
    pub fn is_tab_open(&self, tab: &EditorTab) -> bool {
        self.dock_state.iter_all_tabs().any(|node| node.1 == tab)
    }

    pub fn toggle_tab(&mut self, tab: EditorTab) {
        if let Some(index) = self.dock_state.find_tab(&tab) {
            self.dock_state.remove_tab(index);
        } else {
            self.dock_state.push_to_first_leaf(tab);
        }
    }

    pub fn open_tab(&mut self, tab: EditorTab) {
        if let Some(index) = self.dock_state.find_tab(&tab) {
            let _ = self.dock_state.set_active_tab(index);
        } else {
            self.dock_state.push_to_first_leaf(tab.clone());
            if let Some(index) = self.dock_state.find_tab(&tab) {
                let _ = self.dock_state.set_active_tab(index);
            }
        }
    }

    pub fn reset_layout(&mut self) {
        self.dock_state = create_default_dock_state();
    }

    pub fn save_layout(&mut self) -> Result<(), crate::error::EditorError> {
        let json = serde_json::to_string(&self.dock_state)?;
        std::fs::write("editor_layout.json", json).map_err(|source| {
            crate::error::EditorError::Io {
                context: "layout yazılamadı: editor_layout.json".to_string(),
                source,
            }
        })?;
        self.log_info("Pencere düzeni başarıyla kaydedildi.");
        Ok(())
    }

    pub fn load_layout() -> Option<egui_dock::DockState<EditorTab>> {
        if let Ok(content) = std::fs::read_to_string("editor_layout.json") {
            if let Ok(dock) = serde_json::from_str(&content) {
                return Some(dock);
            } else {
                gizmo_core::logger::log_message(
                    gizmo_core::logger::LogLevel::Error,
                    "editor_layout.json parse hatasi!".to_string(),
                    file!(),
                    line!(),
                );
            }
        }
        None
    }
}
