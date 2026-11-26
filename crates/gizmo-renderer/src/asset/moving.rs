//! Moving an asset with its identity: the file or folder, the `.meta` sidecars with it, and the
//! registry's paths.

use super::error::AssetError;
use super::{read_asset_meta, sidecar_path, write_asset_meta, AssetManager, AssetMeta, ContentId};
use std::path::{Path, PathBuf};

impl AssetManager {
    /// Moves (or renames) the asset file or folder at `from` to `to`, with the `.meta` sidecars
    /// that carry its identity, and updates the registry: an asset keeps its UUID at its new path,
    /// so a scene that names it by identity finds it there (`SceneData::repair_asset_paths`).
    ///
    /// Returns every file that moved as `(old, new)`, both normalised
    /// ([`normalize_path`](Self::normalize_path)), sidecars left out — what a host rewrites the
    /// references in its open world with. A moved sidecar that recorded no content records it.
    ///
    /// # Errors
    ///
    /// [`AssetError::MoveSourceMissing`] when there is nothing at `from`;
    /// [`AssetError::MoveDestinationTaken`] when `to`, or the sidecar it would have, exists —
    /// nothing is overwritten; [`AssetError::MoveFailed`] when the filesystem refuses (another
    /// device, permissions, a missing parent folder), with the file moved back if its sidecar
    /// could not follow it. Nothing is moved on an error.
    pub fn move_asset(&mut self, from: &Path, to: &Path) -> Result<Vec<(String, String)>, AssetError> {
        if !from.exists() {
            return Err(AssetError::MoveSourceMissing { path: from.to_path_buf() });
        }
        if to.exists() {
            return Err(AssetError::MoveDestinationTaken { path: to.to_path_buf() });
        }
        let failed = |source| AssetError::MoveFailed { from: from.to_path_buf(), to: to.to_path_buf(), source };
        let is_dir = from.is_dir();
        let sidecar = (!is_dir).then(|| (sidecar_path(from), sidecar_path(to))).filter(|(old, _)| old.exists());
        if let Some((_, new)) = &sidecar {
            if new.exists() {
                return Err(AssetError::MoveDestinationTaken { path: new.clone() });
            }
        }

        std::fs::rename(from, to).map_err(failed)?;
        if let Some((old, new)) = &sidecar {
            if let Err(e) = std::fs::rename(old, new) {
                // The file without its identity would be the orphan this exists to prevent.
                if let Err(back) = std::fs::rename(to, from) {
                    tracing::error!("[AssetManager] {} is at {} without its sidecar: {back}", from.display(), to.display());
                }
                return Err(failed(e));
            }
        }

        let (from_n, to_n) = (Self::normalize_path(&from.to_string_lossy()), Self::normalize_path(&to.to_string_lossy()));
        let mut moved = Vec::new();
        if is_dir {
            files_under(to, &mut |file| {
                let new = Self::normalize_path(&file.to_string_lossy());
                let rest = &new[to_n.len()..];
                moved.push((format!("{from_n}{rest}"), new));
            });
        } else {
            moved.push((from_n, to_n));
        }
        for (_, new) in &moved {
            let new_path = PathBuf::from(new);
            if let Some(meta) = read_asset_meta(&new_path) {
                self.register(&new_path, meta.uuid);
                if meta.content.is_none() {
                    write_asset_meta(&sidecar_path(&new_path), &AssetMeta { uuid: meta.uuid, content: ContentId::of_file(&new_path) });
                }
            }
        }
        Ok(moved)
    }
}

/// Every file under `dir` but the sidecars, depth first.
fn files_under(dir: &Path, each: &mut dyn FnMut(&Path)) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            files_under(&path, each);
        } else if path.extension().is_none_or(|x| x != "meta") {
            each(&path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{read_asset_meta, sidecar_path, AssetManager, AssetMeta, ContentId};
    use super::AssetError;
    use std::path::{Path, PathBuf};

    /// A fresh directory for one test, named by `tag` and wiped first.
    fn tree(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gizmo_move_{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn put(path: &Path, bytes: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    fn n(path: &Path) -> String {
        AssetManager::normalize_path(&path.to_string_lossy())
    }

    /// A rename carries the sidecar, and the identity answers at the new path and not at the old.
    #[test]
    fn a_renamed_asset_keeps_its_identity() {
        let dir = tree("rename");
        let (old, new) = (dir.join("tree.obj"), dir.join("oak.obj"));
        put(&old, b"o tree");
        let mut m = AssetManager::new();
        m.import_assets_directory(&dir);
        let uuid = m.path_to_uuid[&n(&old)];

        let moved = m.move_asset(&old, &new).unwrap();
        assert_eq!(moved, vec![(n(&old), n(&new))]);
        assert!(!old.exists() && !sidecar_path(&old).exists(), "the old name is gone, sidecar too");
        assert_eq!(read_asset_meta(&new).map(|meta| meta.uuid), Some(uuid), "the sidecar went with it");
        assert_eq!(m.path_to_uuid.get(&n(&new)), Some(&uuid));
        assert_eq!(m.get_path(&uuid), Some(n(&new)));
        assert!(!m.path_to_uuid.contains_key(&n(&old)), "the old path no longer names it");

        // And a later scan of the tree agrees with the registry.
        let mut fresh = AssetManager::new();
        fresh.scan_assets_directory(&dir);
        assert_eq!(fresh.get_path(&uuid), Some(n(&new)));
    }

    /// A folder moves with everything in it, each asset keeping its identity, and the files that
    /// moved are listed.
    #[test]
    fn a_moved_folder_takes_its_assets_and_their_identities() {
        let dir = tree("folder");
        put(&dir.join("props/crate.obj"), b"o crate");
        put(&dir.join("props/wood/plank.png"), b"png");
        put(&dir.join("props/notes.txt"), b"no identity");
        let mut m = AssetManager::new();
        m.import_assets_directory(&dir);
        let plank = m.path_to_uuid[&n(&dir.join("props/wood/plank.png"))];

        let moved = m.move_asset(&dir.join("props"), &dir.join("level/props")).unwrap_err();
        assert!(matches!(moved, AssetError::MoveFailed { .. }), "no `level` folder to move into: {moved:?}");
        assert!(dir.join("props/crate.obj").exists(), "and nothing moved");

        std::fs::create_dir(dir.join("level")).unwrap();
        let moved = m.move_asset(&dir.join("props"), &dir.join("level/props")).unwrap();
        let olds: Vec<&str> = moved.iter().map(|(o, _)| o.as_str()).collect();
        assert_eq!(olds, vec![n(&dir.join("props/crate.obj")), n(&dir.join("props/notes.txt")), n(&dir.join("props/wood/plank.png"))]);
        assert_eq!(m.get_path(&plank), Some(n(&dir.join("level/props/wood/plank.png"))));
    }

    /// Nothing is overwritten: not a file at the destination, and not a sidecar there either.
    #[test]
    fn a_move_onto_something_moves_nothing() {
        let dir = tree("taken");
        put(&dir.join("a.png"), b"a");
        put(&dir.join("b.png"), b"b");
        put(&dir.join("c.png.meta"), b"(uuid: \"00000000-0000-0000-0000-000000000001\")");
        let mut m = AssetManager::new();
        m.import_assets_directory(&dir);
        assert!(matches!(m.move_asset(&dir.join("a.png"), &dir.join("b.png")), Err(AssetError::MoveDestinationTaken { .. })));
        assert!(matches!(m.move_asset(&dir.join("a.png"), &dir.join("c.png")), Err(AssetError::MoveDestinationTaken { .. })));
        assert!(matches!(m.move_asset(&dir.join("gone.png"), &dir.join("d.png")), Err(AssetError::MoveSourceMissing { .. })));
        assert_eq!(std::fs::read(dir.join("b.png")).unwrap(), b"b");
        assert!(dir.join("a.png").exists() && sidecar_path(&dir.join("a.png")).exists());
    }

    /// A rename outside the editor leaves the sidecar behind. A scan finds the identity by the
    /// content the sidecar recorded, writing nothing; an import moves the sidecar. Until
    /// 2026-11-26 the identity was orphaned and an import minted a new one.
    #[test]
    fn an_asset_renamed_without_its_sidecar_is_found_again() {
        let dir = tree("orphan");
        let (old, new) = (dir.join("rock.obj"), dir.join("boulder.obj"));
        put(&old, b"o rock\nv 0 0 0");
        let mut m = AssetManager::new();
        m.import_assets_directory(&dir);
        let uuid = m.path_to_uuid[&n(&old)];
        std::fs::rename(&old, &new).unwrap(); // a file manager, a `mv`: the sidecar stays

        let mut scanned = AssetManager::new();
        scanned.scan_assets_directory(&dir);
        assert_eq!(scanned.get_path(&uuid), Some(n(&new)), "the scan did not find it");
        assert!(sidecar_path(&old).exists() && !sidecar_path(&new).exists(), "a scan writes nothing");

        let mut imported = AssetManager::new();
        imported.import_assets_directory(&dir);
        assert_eq!(imported.get_path(&uuid), Some(n(&new)));
        assert_eq!(read_asset_meta(&new).map(|meta| meta.uuid), Some(uuid), "the import moved the sidecar");
        assert!(!sidecar_path(&old).exists());
    }

    /// Recognition by content is only trusted when it is unambiguous: two files with the same bytes
    /// and one orphan, or a file changed in the move, get no borrowed identity.
    #[test]
    fn an_ambiguous_or_changed_file_does_not_take_an_orphans_identity() {
        let dir = tree("ambiguous");
        put(&dir.join("a.png"), b"same bytes");
        let mut m = AssetManager::new();
        m.import_assets_directory(&dir);
        let uuid = m.path_to_uuid[&n(&dir.join("a.png"))];
        std::fs::rename(dir.join("a.png"), dir.join("b.png")).unwrap();
        put(&dir.join("c.png"), b"same bytes");
        let mut scanned = AssetManager::new();
        scanned.scan_assets_directory(&dir);
        assert_eq!(scanned.get_path(&uuid), None, "two candidates: neither is it");

        std::fs::remove_file(dir.join("c.png")).unwrap();
        put(&dir.join("b.png"), b"edited bytes");
        let mut scanned = AssetManager::new();
        scanned.scan_assets_directory(&dir);
        assert_eq!(scanned.get_path(&uuid), None, "changed in the move: not recognised");
    }

    /// A sidecar from before content was recorded gets it on the next import, and an edited file's
    /// is kept current, so a rename after either is still recognised.
    #[test]
    fn an_import_records_and_refreshes_the_content() {
        let dir = tree("refresh");
        let asset = dir.join("old.png");
        put(&asset, b"first");
        let uuid = uuid::Uuid::new_v4();
        std::fs::write(sidecar_path(&asset), ron::ser::to_string(&AssetMeta::new(uuid)).unwrap()).unwrap();
        let mut m = AssetManager::new();
        m.import_assets_directory(&dir);
        assert_eq!(read_asset_meta(&asset).and_then(|meta| meta.content), Some(ContentId::of(b"first")));

        put(&asset, b"second, edited");
        AssetManager::new().import_assets_directory(&dir);
        assert_eq!(read_asset_meta(&asset).and_then(|meta| meta.content), Some(ContentId::of(b"second, edited")));

        std::fs::rename(&asset, dir.join("new.png")).unwrap();
        let mut scanned = AssetManager::new();
        scanned.scan_assets_directory(&dir);
        assert_eq!(scanned.get_path(&uuid), Some(n(&dir.join("new.png"))));
    }

    /// The recorded content is FNV-1a over the bytes, which is written to disk and must not change
    /// with the build.
    #[test]
    fn the_content_id_is_fnv1a_and_stable() {
        assert_eq!(ContentId::of(b""), ContentId { len: 0, fnv1a: 0xcbf2_9ce4_8422_2325 });
        assert_eq!(ContentId::of(b"a").fnv1a, 0xaf63_dc4c_8601_ec8c);
        assert_eq!(ContentId::of(b"foobar").fnv1a, 0x8594_4171_f739_67e8);
        // A sidecar written before content was recorded still reads.
        let old: AssetMeta = ron::from_str("(uuid: \"00000000-0000-0000-0000-000000000002\")").unwrap();
        assert_eq!(old.content, None);
    }
}
