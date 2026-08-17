//! Hot reload through file watching (`AssetWatcher`).
//!
//! For image files, hand the decode to [`crate::async_assets::AsyncAssetLoader`]'s queue rather
//! than blocking the main thread; each frame, after `drain_completed`, upload to the GPU with
//! [`crate::asset::AssetManager::install_decoded_material_texture`] (as the `demo` render loop
//! does).

use notify::{Event, EventKind, RecursiveMode, Watcher};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::Mutex;

/// The asset watcher: it watches for file changes and triggers hot reloads.
pub struct AssetWatcher {
    _watcher: notify::RecommendedWatcher,
    rx: Mutex<mpsc::Receiver<Result<Event, notify::Error>>>,
}

impl AssetWatcher {
    /// Creates a new AssetWatcher and starts watching the given directories.
    pub fn new<P: AsRef<Path>>(watch_dirs: &[P]) -> Option<Self> {
        let (tx, rx) = mpsc::channel();

        let mut watcher = match notify::recommended_watcher(tx) {
            Ok(w) => w,
            Err(e) => {
                tracing::error!("AssetWatcher: Dosya izleyici oluşturulamadı: {:?}", e);
                return None;
            }
        };

        for dir in watch_dirs {
            let path = dir.as_ref().to_path_buf();
            if path.exists() {
                if let Err(e) = watcher.watch(&path, RecursiveMode::Recursive) {
                    tracing::error!("AssetWatcher: Dizin izlenemedi {:?}: {:?}", path, e);
                } else {
                    tracing::info!("AssetWatcher: İzleniyor → {:?}", path);
                }
            }
        }

        Some(Self {
            _watcher: watcher,
            rx: Mutex::new(rx),
        })
    }

    /// Returns the paths of the files that changed this frame (call it every frame).
    pub fn poll_changes(&self) -> Vec<PathBuf> {
        let mut seen = HashSet::new(); // O(1) dedup (eskiden Vec::contains ile O(N²))

        // Kuyrukta biriken tüm olayları al (non-blocking)
        if let Ok(rx) = self.rx.lock() {
            while let Ok(event_result) = rx.try_recv() {
                if let Ok(event) = event_result {
                    match event.kind {
                        EventKind::Modify(_) | EventKind::Create(_) => {
                            for path in event.paths {
                                seen.insert(path);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        seen.into_iter().collect()
    }
}
