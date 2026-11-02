//! [`SaveData`](crate::save::SaveData) — what a game keeps between runs: settings, best scores, progress.
//!
//! Every game in `demo/` lost its best score and its menu settings when it closed; the engine had
//! a scene format (a level) and the editor's own preferences, and nothing for a *game* to keep.
//!
//! ```no_run
//! use gizmo_app::save::SaveData;
//!
//! let mut save = SaveData::open("kristal_avcisi");
//! let best: u32 = save.get("best").unwrap_or(0);
//! save.set("best", &best.max(12)).unwrap();
//! save.save().unwrap();
//! ```
//!
//! **Where it lives:** on the desktop, `<data dir>/<game>/save.json` (`~/.local/share` on Linux,
//! `%APPDATA%` on Windows, `~/Library/Application Support` on macOS); in the browser,
//! `localStorage` under `gizmo:<game>`.
//!
//! **What it promises:**
//! - **A write never leaves a half-written file.** It goes to `save.json.tmp`, then is renamed over
//!   the old one — a rename is atomic, so a crash or a power cut mid-save leaves the previous save.
//! - **A file it cannot read is moved aside, never overwritten.** A corrupt or hand-broken save is
//!   renamed to `save.json.bad` (then `.bad.1`, …) and the game starts empty, so the next save
//!   does not destroy a player's progress that someone could still recover.
//! - Values are anything `serde` can write: numbers, strings, `Vec`s, your own structs.

use std::collections::BTreeMap;
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};

use serde::{de::DeserializeOwned, Serialize};

/// Why a [`SaveData`] operation failed.
#[derive(Debug)]
#[non_exhaustive]
pub enum SaveError {
    /// The file or directory could not be written.
    Io(std::io::Error),
    /// The value cannot be stored as JSON (a map with non-string keys, a float that is NaN).
    Value(String),
    /// The browser's storage refused the write (quota, private mode).
    Storage(String),
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "save file: {e}"),
            Self::Value(e) => write!(f, "value cannot be saved: {e}"),
            Self::Storage(e) => write!(f, "browser storage: {e}"),
        }
    }
}

impl std::error::Error for SaveError {}

#[derive(Debug, Clone)]
enum Location {
    #[cfg(not(target_arch = "wasm32"))]
    File(PathBuf),
    #[cfg(target_arch = "wasm32")]
    Browser(String),
    Memory,
}

/// A game's saved values — see the [module docs](self).
#[derive(Debug, Clone)]
pub struct SaveData {
    values: BTreeMap<String, serde_json::Value>,
    location: Location,
}

/// `game` as one safe path segment / storage key: letters, digits, `-` and `_`; anything else
/// becomes `_`. An empty name is `game`.
fn segment(game: &str) -> String {
    let s: String = game
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    if s.is_empty() { "game".into() } else { s }
}

impl SaveData {
    /// The save for `game` in the platform's place for it, loaded if it exists — see the
    /// [module docs](self) for where, and for what happens to a file that cannot be read.
    #[must_use]
    pub fn open(game: &str) -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let dir = dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join(segment(game));
            Self::at(dir.join("save.json"))
        }
        #[cfg(target_arch = "wasm32")]
        {
            let key = format!("gizmo:{}", segment(game));
            let values = web_storage()
                .and_then(|s| s.get_item(&key).ok().flatten())
                .and_then(|text| match serde_json::from_str(&text) {
                    Ok(v) => Some(v),
                    Err(e) => {
                        tracing::warn!(key, error = %e, "[Save] stored data could not be read; starting empty");
                        None
                    }
                })
                .unwrap_or_default();
            Self { values, location: Location::Browser(key) }
        }
    }

    /// The save at an explicit `path`, loaded if it exists. For tests, and for a game that keeps
    /// its own save slots.
    #[cfg(not(target_arch = "wasm32"))]
    #[must_use]
    pub fn at(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let values = match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str(&text) {
                Ok(v) => v,
                Err(e) => {
                    let aside = move_aside(&path);
                    tracing::warn!(
                        path = %path.display(),
                        moved_to = ?aside,
                        error = %e,
                        "[Save] the save could not be read; it was moved aside and the game starts empty"
                    );
                    BTreeMap::new()
                }
            },
            Err(_) => BTreeMap::new(),
        };
        Self { values, location: Location::File(path) }
    }

    /// A save that is never written anywhere — `save` succeeds and does nothing.
    #[must_use]
    pub fn in_memory() -> Self {
        Self { values: BTreeMap::new(), location: Location::Memory }
    }

    /// The value under `key`, if there is one of type `T`. A value of another shape reads as
    /// `None` (a save from an older version of the game, say), not as an error.
    #[must_use]
    pub fn get<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        self.values.get(key).and_then(|v| serde_json::from_value(v.clone()).ok())
    }

    /// Stores `value` under `key`, replacing what was there. Kept in memory until
    /// [`save`](Self::save).
    ///
    /// # Errors
    ///
    /// [`SaveError::Value`] when `value` cannot be written as JSON.
    pub fn set<T: Serialize + ?Sized>(&mut self, key: &str, value: &T) -> Result<(), SaveError> {
        let v = serde_json::to_value(value).map_err(|e| SaveError::Value(e.to_string()))?;
        self.values.insert(key.to_string(), v);
        Ok(())
    }

    /// Removes `key`; whether it was there.
    pub fn remove(&mut self, key: &str) -> bool {
        self.values.remove(key).is_some()
    }

    /// Whether `key` holds anything.
    #[must_use]
    pub fn contains(&self, key: &str) -> bool {
        self.values.contains_key(key)
    }

    /// Writes everything to where it came from — atomically on the desktop (see the
    /// [module docs](self)).
    ///
    /// # Errors
    ///
    /// [`SaveError::Io`] when the directory or file cannot be written, [`SaveError::Storage`] when
    /// the browser refuses.
    pub fn save(&self) -> Result<(), SaveError> {
        let text = serde_json::to_string_pretty(&self.values).map_err(|e| SaveError::Value(e.to_string()))?;
        match &self.location {
            #[cfg(not(target_arch = "wasm32"))]
            Location::File(path) => {
                if let Some(dir) = path.parent() {
                    std::fs::create_dir_all(dir).map_err(SaveError::Io)?;
                }
                let tmp = path.with_extension("json.tmp");
                {
                    use std::io::Write;
                    let mut f = std::fs::File::create(&tmp).map_err(SaveError::Io)?;
                    f.write_all(text.as_bytes()).map_err(SaveError::Io)?;
                    f.sync_all().map_err(SaveError::Io)?;
                }
                std::fs::rename(&tmp, path).map_err(SaveError::Io)
            }
            #[cfg(target_arch = "wasm32")]
            Location::Browser(key) => web_storage()
                .ok_or_else(|| SaveError::Storage("no localStorage".into()))?
                .set_item(key, &text)
                .map_err(|e| SaveError::Storage(format!("{e:?}"))),
            Location::Memory => Ok(()),
        }
    }

    /// The file this save lives in, on the desktop.
    #[cfg(not(target_arch = "wasm32"))]
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        match &self.location {
            Location::File(p) => Some(p),
            Location::Memory => None,
        }
    }
}

/// Renames an unreadable save out of the way — `save.json.bad`, or `.bad.1`, `.bad.2`, … if
/// those exist — and returns where it went.
#[cfg(not(target_arch = "wasm32"))]
fn move_aside(path: &Path) -> Option<PathBuf> {
    let base = path.with_extension("json.bad");
    let target = std::iter::once(base.clone())
        .chain((1..1000).map(|i| PathBuf::from(format!("{}.{i}", base.display()))))
        .find(|p| !p.exists())?;
    std::fs::rename(path, &target).ok().map(|()| target)
}

#[cfg(target_arch = "wasm32")]
fn web_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    /// A fresh directory under the system temp dir, removed on drop.
    struct Dir(PathBuf);
    impl Dir {
        fn new(name: &str) -> Self {
            let p = std::env::temp_dir().join(format!("gizmo-save-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            Self(p)
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
    struct Settings {
        volume: u32,
        fullscreen: bool,
    }

    #[test]
    fn values_survive_a_save_and_a_reopen() {
        let dir = Dir::new("roundtrip");
        let path = dir.0.join("nested").join("save.json");
        let mut save = SaveData::at(&path);
        assert_eq!(save.get::<u32>("best"), None, "a missing file is an empty save");
        save.set("best", &42u32).unwrap();
        save.set("name", "Ada").unwrap();
        save.set("settings", &Settings { volume: 75, fullscreen: true }).unwrap();
        save.save().unwrap();
        assert!(!path.with_extension("json.tmp").exists(), "the temporary file was left behind");

        let again = SaveData::at(&path);
        assert_eq!(again.get::<u32>("best"), Some(42));
        assert_eq!(again.get::<String>("name").as_deref(), Some("Ada"));
        assert_eq!(again.get::<Settings>("settings"), Some(Settings { volume: 75, fullscreen: true }));
        assert_eq!(again.get::<String>("best"), None, "a value of another type reads as None");
    }

    /// A save it cannot read is moved aside — twice gives two copies — and never overwritten.
    #[test]
    fn an_unreadable_save_is_moved_aside_not_overwritten() {
        let dir = Dir::new("corrupt");
        std::fs::create_dir_all(&dir.0).unwrap();
        let path = dir.0.join("save.json");
        std::fs::write(&path, "{ this is not json").unwrap();
        let mut save = SaveData::at(&path);
        assert!(!save.contains("anything"));
        let bad = path.with_extension("json.bad");
        assert_eq!(std::fs::read_to_string(&bad).unwrap(), "{ this is not json", "the original is kept");
        save.set("best", &1u32).unwrap();
        save.save().unwrap();
        assert_eq!(std::fs::read_to_string(&bad).unwrap(), "{ this is not json", "a save overwrote it");

        std::fs::write(&path, "also broken").unwrap();
        let _ = SaveData::at(&path);
        assert!(PathBuf::from(format!("{}.1", bad.display())).exists(), "the second one overwrote the first");
    }

    #[test]
    fn remove_and_in_memory() {
        let mut save = SaveData::in_memory();
        save.set("k", &1).unwrap();
        assert!(save.remove("k") && !save.remove("k"));
        save.save().unwrap();
        assert!(save.path().is_none());
        assert!(matches!(save.set("bad", &f32::NAN), Ok(())), "NaN becomes null in serde_json, not an error");
    }

    #[test]
    fn a_game_name_is_one_safe_segment() {
        assert_eq!(segment("kristal_avcisi"), "kristal_avcisi");
        assert_eq!(segment("../../etc"), "______etc");
        assert_eq!(segment("My Game!"), "My_Game_");
        assert_eq!(segment(""), "game");
    }
}
