use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct EditorPrefs {
    pub camera_speed: f32,
    pub camera_focus_distance: f32,
    pub show_grid: bool,
    pub snap_enabled: bool,
    pub snap_translate: f32,
    pub snap_rotate_deg: f32,
    pub snap_scale: f32,
    pub gizmo_size: f32,
}

impl Default for EditorPrefs {
    fn default() -> Self {
        Self {
            camera_speed: 10.0,
            camera_focus_distance: 10.0,
            show_grid: true,
            snap_enabled: false,
            snap_translate: 1.0,
            snap_rotate_deg: 15.0,
            snap_scale: 0.1,
            gizmo_size: 75.0,
        }
    }
}

impl EditorPrefs {
    pub fn load() -> Self {
        if let Ok(data) = std::fs::read_to_string("editor_prefs.toml") {
            if let Ok(prefs) = toml::from_str(&data) {
                return prefs;
            }
        }
        Self::default()
    }

    pub fn save(&self) {
        if let Ok(data) = toml::to_string_pretty(self) {
            let _ = std::fs::write("editor_prefs.toml", data);
        }
    }
}
