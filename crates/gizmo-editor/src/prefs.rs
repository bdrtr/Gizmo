use crate::error::EditorError;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[non_exhaustive]
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

    pub fn save(&self) -> Result<(), EditorError> {
        let path = prefs_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| EditorError::Io {
                context: format!("tercih dizini oluşturulamadı: {}", parent.display()),
                source,
            })?;
        }
        let data = toml::to_string_pretty(self)?;
        std::fs::write(&path, data).map_err(|source| EditorError::Io {
            context: format!("tercihler yazılamadı: {}", path.display()),
            source,
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_clamps_values_below_minimum() {
        let mut p = EditorPrefs::default();
        p.camera_speed = 0.0;
        p.snap_translate = 0.0;
        p.snap_rotate_deg = 0.0;
        p.snap_scale = 0.0;
        p.gizmo_size = 0.0;
        p.max_history = 0;
        p.validate();
        assert_eq!(p.camera_speed, 0.1);
        assert_eq!(p.snap_translate, 0.001);
        assert_eq!(p.snap_rotate_deg, 1.0);
        assert_eq!(p.snap_scale, 0.001);
        assert_eq!(p.gizmo_size, 10.0);
        assert_eq!(p.max_history, 1);
    }

    #[test]
    fn validate_clamps_values_above_maximum() {
        let mut p = EditorPrefs::default();
        p.camera_speed = 1e9;
        p.snap_translate = 1e9;
        p.snap_rotate_deg = 1e9;
        p.snap_scale = 1e9;
        p.gizmo_size = 1e9;
        p.max_history = usize::MAX;
        p.validate();
        assert_eq!(p.camera_speed, 1000.0);
        assert_eq!(p.snap_translate, 100.0);
        assert_eq!(p.snap_rotate_deg, 90.0);
        assert_eq!(p.snap_scale, 10.0);
        assert_eq!(p.gizmo_size, 500.0);
        assert_eq!(p.max_history, 1000);
    }

    /// Aralık içindeki geçerli değerler validate() ile DEĞİŞMEMELİ; ayrıca
    /// clamp edilmeyen alanlara (show_grid, camera_focus_distance) dokunulmamalı.
    #[test]
    fn validate_is_noop_for_valid_values() {
        let mut p = EditorPrefs::default();
        p.camera_speed = 25.0;
        p.snap_rotate_deg = 45.0;
        p.camera_focus_distance = 7.5;
        p.show_grid = false;
        let before = p.clone();
        p.validate();
        assert_eq!(p, before);
    }

    /// Default → TOML → default: alanlar korunmalı (serde round-trip).
    #[test]
    fn toml_round_trip_preserves_default_fields() {
        let p = EditorPrefs::default();
        let s = toml::to_string(&p).expect("serialize");
        let p2: EditorPrefs = toml::from_str(&s).expect("deserialize");
        assert_eq!(p, p2);
    }

    /// Default olmayan değerler de round-trip'te korunmalı.
    #[test]
    fn toml_round_trip_preserves_custom_fields() {
        let mut p = EditorPrefs::default();
        p.camera_speed = 42.5;
        p.snap_enabled = true;
        p.snap_rotate_deg = 30.0;
        p.max_history = 123;
        p.show_grid = false;
        let s = toml::to_string(&p).expect("serialize");
        let p2: EditorPrefs = toml::from_str(&s).expect("deserialize");
        assert_eq!(p, p2);
    }

    /// `dirty` alanı `#[serde(skip)]` → asla diske yazılmaz, deserialize'da
    /// daima `false` döner (kirli bayrağı kalıcı state değildir).
    #[test]
    fn dirty_flag_is_not_persisted() {
        let mut p = EditorPrefs::default();
        p.mark_dirty();
        assert!(p.dirty);
        let s = toml::to_string(&p).expect("serialize");
        assert!(!s.contains("dirty"), "dirty TOML çıktısında olmamalı: {s}");
        let p2: EditorPrefs = toml::from_str(&s).expect("deserialize");
        assert!(!p2.dirty, "deserialize sonrası dirty false olmalı");
    }

    #[test]
    fn new_equals_default() {
        assert_eq!(EditorPrefs::new(), EditorPrefs::default());
    }
}
