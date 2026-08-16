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
//! different engine. So it is answered by reference instead: per frame this runs what
//! `gizmo-studio`'s `handle_simulation` runs behind `is_playing()`, and nothing else.
//!
//! 1. `ScriptEngine::update` → `flush_commands` → per-entity `update_entity`, with a script whose
//!    file cannot be read reported once on the way in and once on the way out — a per-frame log
//!    line is sixty identical lines a second.
//! 2. A fixed-step physics accumulator: 1/60 s, at most 16 steps in a frame, and the debt itself
//!    clamped to 16 steps so a slow frame cannot spiral.
//! 3. `default_render_pass` — the same pass, with SSR/SSGI/volumetric/TAA left on.
//!
//! Picking Play mode as the definition is what makes the two paths comparable: "the exported game
//! does what the editor showed you" stops being a promise and becomes a measurement. Where they
//! differ, one of them is wrong — which is a bug report, not a design argument.
//!
//! Two knowing differences from `handle_simulation`, both because the missing part is the editor,
//! not the game: there is no asset watcher (hot-reload is an authoring tool — a shipped game has
//! no source to watch), and script logs go to stdout instead of the editor console.
//!
//! ## Where the data comes from
//!
//! An exported build is a self-contained directory — the binary with `scenes/`, `scripts/` and
//! `assets/` beside it — so the runtime makes *that* directory the working directory and every
//! relative path the editor wrote keeps meaning what it meant. A dev checkout is not laid out
//! that way (the binary sits in `target/release/`), so there the caller's cwd is left alone:
//!
//! ```sh
//! cargo run --release --bin gizmo_runtime -- demo/assets/perfect_car.scene
//! ```
//!
//! With no argument it opens `scenes/main.scene`, which is where the export writes the scene.

use gizmo::prelude::*;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// The fixed simulation step. Same value as the editor's Play loop — see the module docs on why
/// that is a contract and not a coincidence.
const FIXED_DT: f32 = 1.0 / 60.0;
/// Ceiling on the steps one frame may take, and on the debt the accumulator may carry.
const MAX_STEPS: u32 = 16;

/// The default scene, and the name the export writes.
const DEFAULT_SCENE: &str = "scenes/main.scene";

/// What the runtime carries between frames: the physics debt, and which scripts are currently
/// failing to load (so the failure is reported on its edges rather than every frame).
struct RuntimeState {
    physics_accumulator: f32,
    failed_scripts: BTreeSet<String>,
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

/// What to say about one script's reload attempt. Reporting needs a memory or it is sixty
/// identical lines a second; `failed` is that memory, so a break is announced once and so is the
/// recovery. (The editor keeps its own copy of this decision in `gizmo-studio`; the two are the
/// same rule for the same reason, and they are the first thing to merge if a third caller
/// appears.)
fn report_reload(failed: &mut BTreeSet<String>, path: &str, result: Result<bool, String>) {
    match result {
        Ok(_) => {
            if failed.remove(path) {
                println!("✅ Script yeniden yüklendi: {path}");
            }
        }
        Err(e) => {
            if failed.insert(path.to_string()) {
                eprintln!("❌ Script yüklenemedi: {path} — {e}. Entity'nin script'i ÇALIŞMIYOR.");
            }
        }
    }
}

/// Resources the scene loader and the Play loop need in the world before either runs.
///
/// `AssetManager` is not optional decoration: `App::load_scene` takes it out of the world to load
/// into, and logs "AssetManager bulunamadı, sahne yüklenemiyor" and skips the scene entirely if it
/// is not there.
fn setup(world: &mut World, _renderer: &Renderer) -> RuntimeState {
    world.insert_resource(gizmo::physics::world::PhysicsWorld::new());
    world.insert_resource(AssetManager::new());
    world.insert_resource(gizmo::core::asset::Assets::<gizmo::renderer::components::Mesh>::default());

    match gizmo::scripting::ScriptEngine::new() {
        Ok(engine) => world.insert_resource(engine),
        // A game whose scripts cannot run is still a game that should draw its scene, so this is
        // reported and survived rather than fatal.
        Err(e) => eprintln!("❌ Script motoru başlatılamadı: {e}. Script'ler çalışmayacak."),
    }

    RuntimeState {
        physics_accumulator: 0.0,
        failed_scripts: BTreeSet::new(),
    }
}

/// One frame of the game: scripts, then physics. Mirrors `handle_simulation`'s `is_playing()`
/// branch step for step.
fn update(world: &mut World, state: &mut RuntimeState, dt: f32, input: &Input) {
    if world
        .try_get_resource::<gizmo::scripting::ScriptEngine>()
        .is_ok()
    {
        world.resource_scope(|world, engine: &mut gizmo::scripting::ScriptEngine| {
            if let Err(e) = engine.update(world, input, dt) {
                eprintln!("Script Error: {e}");
            }

            // Audio and scene commands a script asked for that nothing here consumes. The editor
            // drops them on purpose (it must not switch scenes under the author); a shipped game
            // has no such reason, so this is where scene switching would be implemented.
            let _unhandled = engine.flush_commands(world, dt);

            // Per-entity `on_update`. The entity's own property overrides ride along: scripts are
            // cached per path, so a per-entity value cannot live in the shared Lua environment.
            let mut entity_calls = Vec::new();
            {
                let scripts = world.borrow::<gizmo::scripting::Script>();
                for (entity_id, script) in scripts.iter() {
                    entity_calls.push((
                        entity_id,
                        script.file_path.clone(),
                        script.properties.clone(),
                    ));
                }
            }
            for (entity_id, path, properties) in entity_calls {
                report_reload(&mut state.failed_scripts, &path, engine.reload_if_changed(&path));
                if let Err(e) = engine.update_entity(entity_id, &path, dt, &properties) {
                    eprintln!("Entity script error: {e}");
                }
            }

            if let Ok(mut logs) = engine.log_queue.lock() {
                for (level, msg) in logs.drain(..) {
                    match level.as_str() {
                        "error" => eprintln!("[Lua] {msg}"),
                        "warn" => eprintln!("[Lua] {msg}"),
                        _ => println!("[Lua] {msg}"),
                    }
                }
            }
        });
    }

    state.physics_accumulator =
        (state.physics_accumulator + dt).min(FIXED_DT * MAX_STEPS as f32);
    let mut steps = 0;
    while state.physics_accumulator >= FIXED_DT && steps < MAX_STEPS {
        gizmo::physics::system::physics_step_system(world, FIXED_DT);
        state.physics_accumulator -= FIXED_DT;
        steps += 1;
    }
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

    #[test]
    fn a_broken_script_is_announced_once_and_so_is_its_recovery() {
        let mut failed = BTreeSet::new();

        report_reload(&mut failed, "scripts/a.lua", Err("yok".into()));
        assert!(failed.contains("scripts/a.lua"));

        // Still broken, same way: already remembered, so nothing new to say.
        report_reload(&mut failed, "scripts/a.lua", Err("yok".into()));
        assert_eq!(failed.len(), 1);

        report_reload(&mut failed, "scripts/a.lua", Ok(true));
        assert!(failed.is_empty(), "recovery forgets it, so the next break is news again");
    }
}
