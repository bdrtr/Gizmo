use notify::{Watcher, RecursiveMode, Event, EventKind};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

/// Dosya değişikliklerini izleyerek hot-reload tetikleyen Asset Watcher
pub struct AssetWatcher {
    _watcher: notify::RecommendedWatcher,
    rx: mpsc::Receiver<Result<Event, notify::Error>>,
    watched_paths: HashSet<PathBuf>,
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

        let mut watched_paths = HashSet::new();
        for dir in watch_dirs {
            let path = dir.as_ref().to_path_buf();
            if path.exists() {
                if let Err(e) = watcher.watch(&path, RecursiveMode::Recursive) {
                    eprintln!("AssetWatcher: Dizin izlenemedi {:?}: {:?}", path, e);
                } else {
                    println!("AssetWatcher: İzleniyor → {:?}", path);
                    watched_paths.insert(path);
                }
            }
        }

        Some(Self {
            _watcher: watcher,
            rx,
            watched_paths,
        })
    }

    /// Bu frame'de değişen dosyaların yollarını döndürür (her frame çağrılmalı)
    pub fn poll_changes(&self) -> Vec<PathBuf> {
        let mut changed = Vec::new();
        
        // Kuyrukta biriken tüm olayları al (non-blocking)
        while let Ok(event_result) = self.rx.try_recv() {
            if let Ok(event) = event_result {
                match event.kind {
                    EventKind::Modify(_) | EventKind::Create(_) => {
                        for path in event.paths {
                            if !changed.contains(&path) {
                                changed.push(path);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        
        changed
    }
}
