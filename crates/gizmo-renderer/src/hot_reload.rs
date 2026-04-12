//! Dosya izleme (`AssetWatcher`) ile hot-reload.
//!
//! Görüntü dosyaları için decode işini ana iş parçacığını kilitlememek adına
//! [`crate::async_assets::AsyncAssetLoader`] kuyruğuna verin; her karede
//! `drain_completed` sonrası [`crate::asset::AssetManager::install_decoded_material_texture`]
//! ile GPU yüklemesi yapın (ör. `demo` render döngüsü).

use notify::{Event, EventKind, RecursiveMode, Watcher};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

/// Dosya değişikliklerini izleyerek hot-reload tetikleyen Asset Watcher
pub struct AssetWatcher {
    _watcher: notify::RecommendedWatcher,
    rx: mpsc::Receiver<Result<Event, notify::Error>>,
}

impl AssetWatcher {
    /// Yeni bir AssetWatcher oluşturur ve belirtilen dizinleri izlemeye başlar
    pub fn new<P: AsRef<Path>>(watch_dirs: &[P]) -> Option<Self> {
        let (tx, rx) = mpsc::channel();

        let mut watcher = match notify::recommended_watcher(tx) {
            Ok(w) => w,
            Err(e) => {
                eprintln!("AssetWatcher: Dosya izleyici oluşturulamadı: {:?}", e);
                return None;
            }
        };

        for dir in watch_dirs {
            let path = dir.as_ref().to_path_buf();
            if path.exists() {
                if let Err(e) = watcher.watch(&path, RecursiveMode::Recursive) {
                    eprintln!("AssetWatcher: Dizin izlenemedi {:?}: {:?}", path, e);
                } else {
                    println!("AssetWatcher: İzleniyor → {:?}", path);
                }
            }
        }

        Some(Self {
            _watcher: watcher,
            rx,
        })
    }

    /// Bu frame'de değişen dosyaların yollarını döndürür (her frame çağrılmalı)
    pub fn poll_changes(&self) -> Vec<PathBuf> {
        let mut seen = HashSet::new(); // O(1) dedup (eskiden Vec::contains ile O(N²))

        // Kuyrukta biriken tüm olayları al (non-blocking)
        while let Ok(event_result) = self.rx.try_recv() {
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

        seen.into_iter().collect()
    }
}
