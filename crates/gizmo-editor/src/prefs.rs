use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct EditorPrefs {
    pub camera_speed: f32,
    pub camera_focus_distance: f32,
    pub show_grid: bool,
    pub snap_enabled: bool,
    pub snap_translate: f32,
    pub snap_rotate_deg: f32,
    pub snap_scale: f32,
    pub gizmo_size: f32,
    pub max_history: usize,

    #[serde(skip)]
    pub dirty: bool,
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
            max_history: 50,
            dirty: false,
        }
    }
}

pub fn prefs_path() -> std::path::PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("gizmo_editor")
        .join("editor_prefs.toml")
}

impl EditorPrefs {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn load() -> Self {
        let path = prefs_path();
        match std::fs::read_to_string(&path) {
            Ok(data) => match toml::from_str::<Self>(&data) {
                Ok(mut prefs) => {
                    prefs.validate();
                    prefs
                }
                Err(e) => {
                    tracing::error!("[EditorPrefs] Parse hatası: {}, varsayılan kullanılıyor", e);
                    // Bozuk dosyayı yedekle
                    let _ = std::fs::rename(&path, path.with_extension("toml.bak"));
                    Self::default()
                }
            },
            Err(_) => Self::default(), // Dosya yok, normal durum
        }
    }

    pub fn validate(&mut self) {
        self.camera_speed = self.camera_speed.clamp(0.1, 1000.0);
        self.snap_translate = self.snap_translate.clamp(0.001, 100.0);
        self.snap_rotate_deg = self.snap_rotate_deg.clamp(1.0, 90.0);
        self.snap_scale = self.snap_scale.clamp(0.001, 10.0);
        self.gizmo_size = self.gizmo_size.clamp(10.0, 500.0);
        self.max_history = self.max_history.clamp(1, 1000);
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub fn flush_if_dirty(&mut self) {
        if self.dirty {
            let _ = self.save();
            self.dirty = false;
        }
    }

    pub fn save(&self) -> Result<(), Box<dyn std::error::Error>> {
        let path = prefs_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, toml::to_string_pretty(self)?)?;
        Ok(())
    }
}
