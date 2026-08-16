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
    /// Whether the last write failed. Only used to keep the report to one line per run — see
    /// [`EditorPrefs::flush_if_dirty`].
    #[serde(skip)]
    pub save_failed: bool,
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
            save_failed: false,
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

    /// Write the preferences if anything changed, and hand back a failure the first time it
    /// happens.
    ///
    /// This used to be `let _ = self.save();` — every reason a write can fail (no permission on
    /// the config directory, a full disk, a serialisation error) produced exactly nothing. The
    /// user moves a slider, quits, comes back to the old value and has never been told why.
    ///
    /// Nothing is lost by clearing `dirty` on a failure: `save` writes the whole struct, which
    /// lives in memory, so the next successful write carries everything. What was missing was the
    /// telling, and `Some(_)` is returned only on the *first* failure of a run — this is called at
    /// the end of every frame, and a permanently unwritable config directory must not become
    /// sixty console lines a second.
    pub fn flush_if_dirty(&mut self) -> Option<EditorError> {
        self.flush_if_dirty_to(&prefs_path())
    }

    /// [`flush_if_dirty`](Self::flush_if_dirty) against a caller-chosen path — see
    /// [`save_to`](Self::save_to) for why the seam exists.
    pub(crate) fn flush_if_dirty_to(&mut self, path: &std::path::Path) -> Option<EditorError> {
        if !self.dirty {
            return None;
        }
        self.dirty = false;
        match self.save_to(path) {
            Ok(()) => {
                self.save_failed = false;
                None
            }
            Err(e) => {
                if self.save_failed {
                    None
                } else {
                    self.save_failed = true;
                    Some(e)
                }
            }
        }
    }

    /// Write the preferences to their usual place.
    pub fn save(&self) -> Result<(), EditorError> {
        self.save_to(&prefs_path())
    }

    /// Write the preferences to `path`.
    ///
    /// Split out from [`save`](Self::save) so a test can aim it at a path it controls. The
    /// alternative — pointing `XDG_CONFIG_HOME` somewhere — mutates process-global state that
    /// `EditorPrefs::load` (via `EditorState::new`) reads from other tests running in parallel in
    /// the same binary, and `env::set_var` is unsafe in a threaded program for exactly that reason.
    pub(crate) fn save_to(&self, path: &std::path::Path) -> Result<(), EditorError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| EditorError::Io {
                context: format!("tercih dizini oluşturulamadı: {}", parent.display()),
                source,
            })?;
        }
        let data = toml::to_string_pretty(self)?;
        std::fs::write(path, data).map_err(|source| EditorError::Io {
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
        let mut p = EditorPrefs {
            camera_speed: 0.0,
            snap_translate: 0.0,
            snap_rotate_deg: 0.0,
            snap_scale: 0.0,
            gizmo_size: 0.0,
            max_history: 0,
            ..Default::default()
        };
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
        let mut p = EditorPrefs {
            camera_speed: 1e9,
            snap_translate: 1e9,
            snap_rotate_deg: 1e9,
            snap_scale: 1e9,
            gizmo_size: 1e9,
            max_history: usize::MAX,
            ..Default::default()
        };
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
        let mut p = EditorPrefs {
            camera_speed: 25.0,
            snap_rotate_deg: 45.0,
            camera_focus_distance: 7.5,
            show_grid: false,
            ..Default::default()
        };
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
        let p = EditorPrefs {
            camera_speed: 42.5,
            snap_enabled: true,
            snap_rotate_deg: 30.0,
            max_history: 123,
            show_grid: false,
            ..Default::default()
        };
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

#[cfg(test)]
mod flush_reporting_tests {
    use super::*;

    /// A path that cannot be written, because its parent is a regular file rather than a directory.
    ///
    /// `save_to` starts with `create_dir_all(parent)`, which cannot succeed over a file — the same
    /// failure a read-only or missing-permission config directory produces, reached without
    /// touching `XDG_CONFIG_HOME`. That matters: `EditorPrefs::load` reads it through
    /// `EditorState::new`, which other tests in this same binary call in parallel.
    fn unwritable(name: &str) -> std::path::PathBuf {
        let blocker = std::env::temp_dir()
            .join(format!("gizmo_prefs_block_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&blocker);
        std::fs::write(&blocker, b"a file where a directory must be").expect("engel");
        blocker.join("editor_prefs.toml")
    }

    /// A preferences write that fails must say so — once.
    ///
    /// The call was `let _ = self.save();`, so an unwritable config directory produced silence and
    /// the user found out by restarting and seeing their old settings. Reporting it naively is the
    /// other failure: `flush_if_dirty` runs at the end of every frame, so a permanently unwritable
    /// directory would be sixty console lines a second.
    #[test]
    fn a_failed_write_is_reported_exactly_once() {
        let path = unwritable("once");
        let mut prefs = EditorPrefs::new();

        prefs.mark_dirty();
        assert!(
            prefs.flush_if_dirty_to(&path).is_some(),
            "an unwritable path produced no error at all — which is the defect"
        );

        for _ in 0..100 {
            prefs.mark_dirty();
            assert!(
                prefs.flush_if_dirty_to(&path).is_none(),
                "the same failure was reported again; at frame rate that is a console nobody \
                 can read"
            );
        }
        let _ = std::fs::remove_file(path.parent().unwrap());
    }

    /// Once it starts working, a later failure is news again.
    #[test]
    fn a_failure_after_a_success_is_reported_again() {
        let dir = std::env::temp_dir().join(format!("gizmo_prefs_ok_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dizin");
        let good = dir.join("editor_prefs.toml");
        let bad = unwritable("again");

        let mut prefs = EditorPrefs::new();
        prefs.mark_dirty();
        assert!(prefs.flush_if_dirty_to(&bad).is_some());
        prefs.mark_dirty();
        assert!(prefs.flush_if_dirty_to(&good).is_none(), "a good write must be silent");
        assert!(good.is_file(), "the good write did not produce a file");

        prefs.mark_dirty();
        assert!(
            prefs.flush_if_dirty_to(&bad).is_some(),
            "after a run of successes a new failure is new information and must be reported"
        );

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_file(bad.parent().unwrap());
    }

    /// A clean flush is silent, and nothing is written when nothing changed.
    #[test]
    fn a_clean_flush_says_nothing() {
        let path = std::env::temp_dir().join(format!("gizmo_prefs_never_{}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut prefs = EditorPrefs::new();
        assert!(prefs.flush_if_dirty_to(&path).is_none());
        assert!(!path.exists(), "an unchanged prefs set wrote a file anyway");
    }

    /// Clearing `dirty` on failure loses nothing: `save_to` writes the whole struct, which lives in
    /// memory, so the next successful write carries the change that failed. This pins that — it is
    /// the reason the fix is "report" rather than "retry every frame forever".
    #[test]
    fn a_failed_write_does_not_lose_the_setting() {
        let dir = std::env::temp_dir().join(format!("gizmo_prefs_keep_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dizin");
        let bad = unwritable("keep");
        let good = dir.join("editor_prefs.toml");

        let mut prefs = EditorPrefs::new();
        prefs.camera_speed = 42.0;
        prefs.mark_dirty();
        assert!(prefs.flush_if_dirty_to(&bad).is_some());
        assert_eq!(prefs.camera_speed, 42.0, "the failed write took the setting with it");

        prefs.mark_dirty();
        assert!(prefs.flush_if_dirty_to(&good).is_none());
        let written = std::fs::read_to_string(&good).expect("dosya");
        assert!(
            written.contains("42"),
            "the next successful write did not carry the value the failed one dropped: {written}"
        );

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_file(bad.parent().unwrap());
    }
}
