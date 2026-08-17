//! The bridge between an asset registry and the scene format's identity fallback.
//!
//! `gizmo-scene` asks about identity through [`AssetIdentity`](gizmo_scene::AssetIdentity) because
//! it sits *below* `gizmo-renderer` and cannot reach the [`AssetManager`] that owns the path↔UUID
//! registry. This facade has both, so the implementation lives here — one impl, no new dependency
//! in either direction.
//!
//! # What this makes work, and what it does not
//!
//! With a scanned registry, a scene saved through
//! [`SceneData::save_with_identity`](gizmo_scene::SceneData::save_with_identity) records each
//! asset's UUID next to its path, and a later
//! [`load_into_with_identity`](gizmo_scene::SceneData::load_into_with_identity) repoints any path
//! that has gone stale. That covers the case a path reference cannot survive: an asset **moved**
//! with its `.meta` sidecar — dragging a folder, `git mv`, reorganising `demo/assets`.
//!
//! It does not cover a rename that leaves the sidecar behind: the sidecar is named after its asset
//! file, so the identity is orphaned and a later import mints a new one. That is a property of
//! sidecar identity, recorded on
//! [`import_assets_directory`](gizmo_renderer::asset::AssetManager::import_assets_directory), not
//! something this bridge can repair.
//!
//! **Nothing is registered until somebody scans.** `AssetManager::new` deliberately does not touch
//! the filesystem (it has dozens of call sites, and a constructor that walked a CWD-relative
//! `assets/` once stamped sidecars into whatever tree sat next to the working directory), so the
//! registry is empty until [`scan_assets_directory`](gizmo_renderer::asset::AssetManager::scan_assets_directory)
//! or the minting `import_assets_directory` is called. An empty registry answers `None` to both
//! lookups, which every caller treats as "leave the path alone" — so an application that never
//! scans behaves exactly as it did before any of this existed.

use gizmo_renderer::asset::AssetManager;
use gizmo_scene::AssetIdentity;

/// Answers the scene layer's identity questions from an [`AssetManager`]'s registry.
///
/// Borrowed rather than owning: the manager is a world resource, and the caller has it out for the
/// duration of a save or load anyway.
pub struct ManagerIdentity<'a>(pub &'a AssetManager);

impl AssetIdentity for ManagerIdentity<'_> {
    fn uuid_for_path(&self, path: &str) -> Option<String> {
        self.0.get_uuid(path).map(|id| id.to_string())
    }

    fn path_for_uuid(&self, uuid: &str) -> Option<String> {
        // A malformed UUID is not an error here: the scene may have been written by hand, and
        // "don't know" is the answer that leaves the path alone.
        let parsed = uuid.parse().ok()?;
        self.0.get_path(&parsed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A registry populated the way an application populates one — by importing a directory —
    /// answers both directions, and an empty one answers neither.
    ///
    /// Deliberately no hand-made UUID: asking `uuid_for_path` and feeding the answer back to
    /// `path_for_uuid` tests the pair as a round trip, needs no `uuid` dependency here, and proves
    /// the claim that matters — a scanned tree makes identity resolvable, an unscanned one leaves
    /// every path alone.
    #[test]
    fn a_scanned_tree_answers_both_directions_and_an_unscanned_one_answers_neither() {
        let dir = std::env::temp_dir().join(format!(
            "gizmo-identity-bridge-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let asset = dir.join("thing.png");
        std::fs::write(&asset, b"not really a png").expect("write asset");

        let mut manager = AssetManager::new();
        assert_eq!(
            ManagerIdentity(&manager).uuid_for_path(&asset.to_string_lossy()),
            None,
            "an AssetManager that has scanned nothing knows nothing — the state every application \
             that never calls a scan is in"
        );

        // Import is the minting half: the asset has no sidecar yet.
        manager.import_assets_directory(&dir);
        let bridge = ManagerIdentity(&manager);

        let uuid = bridge
            .uuid_for_path(&asset.to_string_lossy())
            .expect("importing gave the asset an identity");
        assert_eq!(
            bridge.path_for_uuid(&uuid).as_deref(),
            Some(asset.to_string_lossy().replace('\\', "/").as_str()),
            "the identity must lead back to the asset it was minted for"
        );

        // A path is not a UUID, and must not be treated as one.
        assert_eq!(bridge.path_for_uuid(&asset.to_string_lossy()), None);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
