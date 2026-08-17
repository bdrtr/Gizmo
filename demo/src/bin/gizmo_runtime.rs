//! # `gizmo_runtime` — the binary an exported game ships
//!
//! Studio's Build/Export packaged `demo`'s default binary, `bevy_3d_scene`: a fixed floor, cube,
//! light and camera that reads no scene file and runs no script. The export copied the user's
//! `scenes/` and `scripts/` next to it, and nothing on the other side ever opened them. This is
//! the other side.
//!
//! ## The contract is the editor's Play mode, deliberately
//!
//! What a default runtime should open — physics? scripting? networking? — is the question that
//! kept this unwritten, and answering it by taste would have produced a second, slightly
//! different engine. So it is not answered here at all: the frame is
//! [`gizmo::systems::PlayLoop`], the same function the editor's ▶ drives. Scripts (shared pass →
//! queued commands → per-entity update), then a 1/60 s fixed-step physics accumulator, then
//! `default_render_pass` with SSR/SSGI/volumetric/TAA left on.
//!
//! That is what makes "the exported game does what the editor showed you" a fact instead of a
//! promise: there is nothing here to drift *from*. Only the reporting differs — the editor writes
//! to its console, this writes to stdout — which is why `PlayLoop::step` takes the reporter as an
//! argument.
//!
//! Two knowing differences from the editor, both because the missing part is the editor and not
//! the game: no asset watcher (hot-reload is an authoring tool — a shipped game has no source to
//! watch), and no default `ActionMap` scaffolding.
//!
//! ## Where the data comes from
//!
//! An exported build is a self-contained directory — the binary with `scenes/`, `scripts/` and
//! `assets/` beside it — so the runtime makes *that* directory the working directory and every
//! relative path the editor wrote keeps meaning what it meant. A dev checkout is not laid out
//! that way (the binary sits in `target/release/`), so there the caller's cwd is left alone:
//!
//! ```sh
//! cargo run --release --bin gizmo_runtime -- demo/assets/sample.scene
//! ```
//!
//! With no argument it opens `scenes/main.scene`, which is where the export writes the scene.

use gizmo::prelude::*;
use gizmo::systems::{PlayLoop, PlayReport};
use std::path::{Path, PathBuf};

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// The default scene, and the name the export writes.
const DEFAULT_SCENE: &str = "scenes/main.scene";

/// What the runtime carries between frames — all of it inside the shared play loop.
struct RuntimeState {
    play: PlayLoop,
}

/// The scene to open: the first argument, or the exported default.
fn scene_argument(mut args: impl Iterator<Item = String>) -> String {
    args.nth(1).unwrap_or_else(|| DEFAULT_SCENE.to_string())
}

/// Whether the binary is sitting in an exported game directory, in which case that directory is
/// the project root and becomes the working directory.
///
/// Split from the `main` that uses it so the rule is testable without an export on disk: it is
/// "does a `scenes/` directory sit beside the binary", and a dev build (in `target/release/`)
/// answers no, which is exactly when the caller's own cwd is the right one to keep.
fn exported_layout_root<'a>(exe_dir: &'a Path, is_dir: &dyn Fn(&Path) -> bool) -> Option<&'a Path> {
    if is_dir(&exe_dir.join("scenes")) {
        Some(exe_dir)
    } else {
        None
    }
}

/// Where a shipped game's messages go: the console it was launched from. The editor's copy of
/// this match writes the same events to its own console — that difference is the only one the
/// two paths have, which is why it is the part that is injected.
fn print_report(report: PlayReport<'_>) {
    match report {
        PlayReport::ScriptError { error } => eprintln!("Script hatası: {error}"),
        PlayReport::EntityScriptError {
            entity,
            path,
            error,
        } => eprintln!("Entity {entity} script hatası ({path}): {error}"),
        PlayReport::ScriptBroke { path, error } => {
            eprintln!("❌ Script yüklenemedi: {path} — {error}. Entity'nin script'i ÇALIŞMIYOR.")
        }
        PlayReport::ScriptRecovered { path } => println!("✅ Script yeniden yüklendi: {path}"),
        PlayReport::ScriptLog { level, message } => match level {
            "error" | "warn" => eprintln!("[Lua] {message}"),
            _ => println!("[Lua] {message}"),
        },
    }
}

/// Resources the scene loader and the Play loop need in the world before either runs.
///
/// `AssetManager` is not optional decoration: `App::load_scene` takes it out of the world to load
/// into, and logs "AssetManager bulunamadı, sahne yüklenemiyor" and skips the scene entirely if it
/// is not there.
fn setup(world: &mut World, _renderer: &Renderer) -> RuntimeState {
    world.insert_resource(gizmo::physics::world::PhysicsWorld::new());

    // Register asset identities before the scene loads, so a reference whose path has gone stale
    // can be repointed at the file it names (`gizmo::asset_identity`). Read-only: a shipped game
    // must not stamp new sidecars into its own install directory — an asset with no identity keeps
    // being addressed by path, exactly as before.
    //
    // `assets/` and `demo/assets/` because those are the two layouts `exported_layout_root` picks
    // between: an export puts `assets/` beside the binary, and a dev run from the workspace has
    // `demo/assets/`. Scanning a directory that does not exist is a no-op.
    let mut assets = AssetManager::new();
    for root in ["assets", "demo/assets"] {
        assets.scan_assets_directory(Path::new(root));
    }
    if !assets.path_to_uuid.is_empty() {
        tracing::debug!(registered = assets.path_to_uuid.len(), "varlık kimlikleri kaydedildi");
    }
    world.insert_resource(assets);
    world.insert_resource(gizmo::core::asset::Assets::<gizmo::renderer::components::Mesh>::default());

    match gizmo::scripting::ScriptEngine::new() {
        Ok(engine) => world.insert_resource(engine),
        // A game whose scripts cannot run is still a game that should draw its scene, so this is
        // reported and survived rather than fatal.
        Err(e) => eprintln!("❌ Script motoru başlatılamadı: {e}. Script'ler çalışmayacak."),
    }

    RuntimeState {
        play: PlayLoop::new(),
    }
}

/// One frame of the game. The frame itself is the engine's; only where the messages land is this
/// binary's business.
fn update(world: &mut World, state: &mut RuntimeState, dt: f32, input: &Input) {
    state.play.step(world, dt, input, &mut print_report);
}

fn render(
    world: &mut World,
    _state: &RuntimeState,
    encoder: &mut gizmo::wgpu::CommandEncoder,
    view: &gizmo::wgpu::TextureView,
    renderer: &mut Renderer,
    _light_time: f32,
) {
    gizmo::systems::render::default_render_pass(world, encoder, view, renderer);
}

fn main() {
    // Not decoration: the engine's panic hook reports through `tracing::error!`, so a runtime
    // without a subscriber dies with exit code 101 and an empty terminal. Measured — the first
    // scene this binary was pointed at crashed in the loader and said nothing at all.
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .try_init();

    let scene = scene_argument(std::env::args());

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let is_dir = |p: &Path| p.is_dir();
            if let Some(root) = exported_layout_root(dir, &is_dir) {
                let _ = std::env::set_current_dir(root);
            }
        }
    }

    // The window carries the scene's own name — an exported game should not be called "demo".
    let title = Path::new(&scene)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "Gizmo".to_string());

    if !PathBuf::from(&scene).exists() {
        eprintln!(
            "⚠ Sahne bulunamadı: {scene} (çalışma dizini: {}). Pencere açılacak ama boş olacak.",
            std::env::current_dir()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| "?".into())
        );
    }

    App::<RuntimeState>::new(&title, 1280, 720)
        .set_setup(setup)
        .set_update(update)
        .set_render(render)
        .load_scene(&scene)
        .run()
        .expect("çalışma zamanı başlatılamadı");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scene_argument_defaults_to_the_exported_name() {
        let none = scene_argument(["gizmo_runtime".to_string()].into_iter());
        assert_eq!(none, DEFAULT_SCENE);

        let given = scene_argument(
            ["gizmo_runtime".to_string(), "levels/two.scene".to_string()].into_iter(),
        );
        assert_eq!(given, "levels/two.scene");
    }

    /// The rule that decides whether the binary is standing in an exported game or in a dev
    /// checkout. Getting it backwards would either break `cargo run` (chdir into `target/release`,
    /// where no asset resolves) or break the exported game (leave cwd wherever the user
    /// double-clicked from).
    #[test]
    fn only_an_exported_layout_takes_over_the_working_directory() {
        let exported = Path::new("/games/my_game");
        let has_scenes = |p: &Path| p == Path::new("/games/my_game/scenes");
        assert_eq!(
            exported_layout_root(exported, &has_scenes),
            Some(exported),
            "a binary with scenes/ beside it IS the project root"
        );

        let dev = Path::new("/repo/target/release");
        assert_eq!(
            exported_layout_root(dev, &has_scenes),
            None,
            "a dev build must keep the cwd it was launched from"
        );
    }

    /// The frame belongs to the engine. If this binary ever grows its own accumulator or its own
    /// script order again, the export's promise ("it does what Play mode did") quietly becomes
    /// two implementations of one contract — which is exactly what it was before.
    #[test]
    fn the_frame_is_the_shared_play_step_not_a_copy_of_it() {
        let src = include_str!("gizmo_runtime.rs");
        let code = src.split("#[cfg(test)]").next().unwrap_or("");

        assert!(
            code.contains("play.step("),
            "the runtime must drive gizmo::systems::PlayLoop"
        );
        for reimplemented in ["physics_step_system(", "flush_commands(", "update_entity("] {
            assert!(
                !code.contains(reimplemented),
                "the runtime re-implements {reimplemented:?} instead of sharing the play step"
            );
        }
    }
}
