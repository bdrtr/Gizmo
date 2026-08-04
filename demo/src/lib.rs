//! Shared helpers for the demo binaries.
//!
//! The demos live in `src/bin/` and each one is a standalone showcase, but a few
//! concerns are common enough that duplicating them across 39 files is worse than
//! a small shared surface. Right now that is asset resolution.

/// Locating optional model/texture assets that are **not** committed to the repository.
///
/// `.gitignore` excludes `*.glb`, so every demo that shows off a real vehicle or a
/// textured glTF scene depends on a file a fresh clone does not have. Those demos used
/// to hardcode an absolute path into the original author's home directory and
/// `unwrap()` on the load — so `cargo run --bin car_demo`, a command the README itself
/// tells people to run, panicked for everyone else (and, once the repo was renamed,
/// for the author too).
///
/// The rule here: a missing optional asset is a *degraded demo*, never a crash.
pub mod assets {
    use std::path::{Path, PathBuf};

    /// Resolve an optional asset by file name, returning `None` when it is unavailable.
    ///
    /// Search order:
    /// 1. `$GIZMO_ASSETS/<name>` — for keeping a personal asset library outside the repo.
    /// 2. `<repo>/assets/<name>` — relative to `CARGO_MANIFEST_DIR`, so it works no
    ///    matter which directory `cargo run` was invoked from.
    /// 3. `assets/<name>` relative to the current working directory.
    ///
    /// Callers are expected to fall back to procedural geometry, not to panic:
    ///
    /// ```
    /// # use demo::assets;
    /// match assets::find("definitely_not_here.glb") {
    ///     Some(path) => { /* load the real model */ }
    ///     None => { /* build a stand-in out of primitives */ }
    /// }
    /// ```
    pub fn find(name: &str) -> Option<PathBuf> {
        let mut candidates: Vec<PathBuf> = Vec::new();

        if let Ok(dir) = std::env::var("GIZMO_ASSETS") {
            candidates.push(Path::new(&dir).join(name));
        }
        // `CARGO_MANIFEST_DIR` is `<repo>/demo`, so the repo's `assets/` is one level up.
        candidates.push(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../assets")
                .join(name),
        );
        candidates.push(Path::new("assets").join(name));

        candidates.into_iter().find(|p| p.is_file())
    }

    /// [`find`], plus a one-line explanation on stderr when the asset is missing.
    ///
    /// Use this at the call site that would otherwise silently render a placeholder, so a
    /// user who *does* have the model knows why they are looking at a grey box.
    pub fn find_or_warn(name: &str, what_youll_get_instead: &str) -> Option<PathBuf> {
        let found = find(name);
        if found.is_none() {
            eprintln!(
                "[assets] '{name}' not found — falling back to {what_youll_get_instead}.\n\
                 [assets] Large models are gitignored and not shipped with the repo. To use the \
                 real one, drop it in <repo>/assets/ or point $GIZMO_ASSETS at a directory \
                 containing it."
            );
        }
        found
    }
}

#[cfg(test)]
mod tests {
    use super::assets;

    #[test]
    fn missing_assets_resolve_to_none_instead_of_panicking() {
        assert!(assets::find("this-file-does-not-exist-9d3f.glb").is_none());
    }

    /// The repo does ship a few small assets; `suzanne.obj` is committed, so resolution
    /// relative to `CARGO_MANIFEST_DIR` must find it regardless of the working directory.
    #[test]
    fn committed_assets_are_found_relative_to_the_manifest() {
        assert!(
            assets::find("suzanne.obj").is_some(),
            "assets/suzanne.obj is committed and must be discoverable"
        );
    }

    #[test]
    fn env_override_takes_precedence_when_it_contains_the_file() {
        // `GIZMO_ASSETS` pointing somewhere without the file must not shadow the repo copy.
        let tmp = std::env::temp_dir();
        // SAFETY-ish: single-threaded test, restored immediately.
        unsafe { std::env::set_var("GIZMO_ASSETS", &tmp) };
        let found = assets::find("suzanne.obj");
        unsafe { std::env::remove_var("GIZMO_ASSETS") };
        assert!(found.is_some(), "must fall through to the repo's assets/ dir");
    }
}
