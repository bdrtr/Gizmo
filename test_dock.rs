use egui_dock::DockState;
pub fn test(dock: &mut DockState<String>) {
    if let Some(index) = dock.find_tab(&"A".to_string()) {
        dock.set_active_tab(index);
    }
}
