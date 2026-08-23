//! Shared helpers for the demo binaries.
//!
//! The demos live in `src/bin/` and each one is a standalone showcase, but a few
//! concerns are common enough that duplicating them across the binaries is worse than
//! a small shared surface: asset resolution, and re-running the simple scene's own
//! per-frame work when a demo needs an exclusive `&mut World` hook of its own.

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

/// Re-running what [`SimpleAppExt::with_simple_scene`] does every frame.
///
/// **The trap this exists for.** `App::set_update` *replaces* the update hook — the builder
/// stores `self.update_fn = Some(f)`, it does not chain. And `with_simple_scene` installs its
/// own `set_update`, which does four things:
///
///   1. `state.fly_step(input, dt)` — turns mouse/WASD into a camera pose;
///   2. stamps that pose onto every `(Transform, Camera)` pair in the world;
///   3. steps the CPU physics in ≤16 ms slices;
///   4. runs `TransformSyncSystem` and `TransformPropagateSystem`.
///
/// So a demo that wants its own exclusive hook — anything needing `&mut World`, which a
/// scheduled system can never have — and writes `.with_simple_scene(..).set_update(..)` silently
/// throws all four away. Measured 2026-08-23: the camera stops responding to input and
/// `GlobalTransform` stops being propagated, with nothing logged and nothing failing to compile.
///
/// Call this at the top of such a hook and the demo gets both.
///
/// ```no_run
/// # use gizmo::prelude::*;
/// # use gizmo::simple::{SimpleAppExt, SimpleSceneState};
/// # fn my_work(_: &mut gizmo::core::World) {}
/// # let app = App::<SimpleSceneState>::new("x", 1, 1).with_simple_scene(|_, _| {});
/// app.set_update(|world, state, dt, input| {
///     demo::simple_scene_update(world, state, dt, input);
///     my_work(world);
/// })
/// # ;
/// ```
///
/// ## One thing it deliberately reproduces, warts and all
///
/// Step 2 stamps **every** camera, with no `primary` check. That is the engine's behaviour, not
/// a simplification here: under the simple scene a second camera cannot hold its own pose, which
/// is why split-screen and multi-pass demos have nothing to work with even though
/// `default_render_pass` accepts any target view.
pub fn simple_scene_update(
    world: &mut gizmo::core::World,
    state: &mut gizmo::simple::SimpleSceneState,
    dt: f32,
    input: &gizmo::core::input::Input,
) {
    use gizmo::core::system::System;
    use gizmo::prelude::{Quat, Transform};
    use std::f32::consts::FRAC_PI_2;

    state.fly_step(input, dt);

    if let Some(mut q) = world.query_mut::<(
        gizmo::core::query::Mut<Transform>,
        gizmo::core::query::Mut<gizmo::renderer::components::Camera>,
    )>() {
        let rot = Quat::from_rotation_y(-state.camera_yaw + FRAC_PI_2)
            * Quat::from_rotation_x(state.camera_pitch);
        for (_, (mut transform, mut camera)) in q.iter_mut() {
            transform.position = state.camera_pos;
            transform.rotation = rot;
            camera.yaw = state.camera_yaw;
            camera.pitch = state.camera_pitch;
        }
    }

    let mut physics_dt = dt.min(0.1);
    while physics_dt > 0.0 {
        let step = physics_dt.min(0.016);
        gizmo::systems::cpu_physics_step_system(world, step);
        physics_dt -= step;
    }

    gizmo::systems::transform::TransformSyncSystem.run(world, dt);
    gizmo::systems::transform::TransformPropagateSystem.run(world, dt);
}
