use crate::state::StudioState;
use gizmo::editor::EditorState;
use gizmo::prelude::*;
use gizmo::systems::PlayReport;

/// Route one frame's play-loop report into the editor console.
///
/// This is the whole of what the editor adds to a running game: the frame itself is
/// `gizmo::systems::PlayLoop`, the same one an exported game runs, so the only thing the two
/// callers may legitimately disagree about is where the messages land. The decisions behind these
/// messages — when a broken script is worth a line and when it is not — moved into that shared
/// step with the loop they belong to.
fn log_play_report(editor_state: &mut EditorState, report: PlayReport<'_>) {
    match report {
        PlayReport::ScriptError { error } => {
            editor_state.log_error(&format!("Script Error: {error}"))
        }
        PlayReport::EntityScriptError { path, error, .. } => {
            editor_state.log_warning(&format!("Entity script error ({path}): {error}"))
        }
        PlayReport::ScriptBroke { path, error } => editor_state.log_error(&format!(
            "❌ Script yüklenemedi: {path} — {error}. Entity'nin script'i ÇALIŞMIYOR."
        )),
        PlayReport::ScriptRecovered { path } => {
            editor_state.log_info(&format!("✅ Script yeniden yüklendi: {path}"))
        }
        PlayReport::ScriptLog { level, message } => match level {
            "error" => editor_state.log_error(&format!("[Lua] {message}")),
            "warn" => editor_state.log_warning(&format!("[Lua] {message}")),
            _ => editor_state.log_info(&format!("[Lua] {message}")),
        },
        PlayReport::Unanswered { call } => editor_state.log_warning(&unanswered_line(call)),
    }
}

/// The console line for a Lua call nothing answered.
fn unanswered_line(call: &str) -> String {
    match call {
        "scene.load" | "scene.save" => {
            format!("⚠ [Lua] `{call}` yok sayıldı: editör, oynayan bir betiğin sahneyi değiştirmesine ya da kaydetmesine izin vermez.")
        }
        _ => format!("⚠ [Lua] `{call}` yok sayıldı: motorda bunu karşılayan bir altsistem yok."),
    }
}

/// Advances play mode: steps the game while playing — a hooked game's own frame, else the engine's
/// `PlayLoop`. ▶ and ⏹ themselves are `crate::pie`'s, run first in the update hook.
pub fn handle_simulation(
    world: &mut World,
    editor_state: &mut EditorState,
    state: &mut StudioState,
    dt: f32,
    input: &Input,
) {
    hot_reload(world, editor_state, state);

    // --- OYUN / SİMÜLASYON DÖNGÜSÜ ---
    if editor_state.is_playing() {
        // The game is drawn into the Game panel, so for its step it sees the panel: its window is
        // the panel's render target, and its pointer the one over the panel, in that target's
        // pixels (`game_view_input`, `GameWindow`).
        let input = game_view_input(input, editor_state.game_view_pointer.map(|p| (p.x, p.y)));
        let window = GameWindow::enter(world);
        // A game hooked into the studio runs its own frame — the one its binary runs — once its
        // setup has run on the play world (`crate::pie::GameSetupPending`, the render hook's).
        let hooked = step_hooked_game(world, state, dt, &input);
        if !hooked {
            // The frame of a running game is the ENGINE's, not the editor's: `PlayLoop::step` is
            // the same call an exported game makes (`demo/src/bin/gizmo_runtime.rs`). That is what
            // lets Build/Export promise the shipped game behaves like the one you just pressed ▶
            // on — a promise that was prose here and a second implementation over there, until the
            // two were merged. What stays on this side is the console.
            //
            // The default `ActionMap` below is the one thing the editor adds and the runtime does
            // not. It was written for the fighter system deleted in `592bd6f`, which read these
            // eight names straight out of it and both filled the input buffer and picked the move.
            // The clock has since come back (`fighter_frame_system`, on `PlayLoop`'s fixed step)
            // but deliberately without those two halves: which button is a jab is the game's, and
            // so is which action names its combos are spelled in. So this stays scaffolding —
            // bindings a game actually needs belong in the scene, and then both paths get them.
            insert_default_action_map(world);

            // `editor_state` and `state` are separate bindings, so the console can be written
            // from inside the callback: the step borrows `state.play`, the reporter borrows the
            // editor, and neither is the other.
            state
                .play
                .step(world, dt, &input, &mut |r| log_play_report(editor_state, r));
        }
        window.leave(world);
        if hooked {
            return;
        }
    } else {
        // Time does not pass in a stopped game; carrying the debt into the next ▶ would spend it
        // all in one frame.
        state.play.reset();
    }

    // ⏸ has to reach the device too. `PlayLoop::step` is what drives audio, and a paused editor
    // does not call it — so the level's ambience kept playing over a frozen frame. Called every
    // frame rather than on the transition: the manager pushes only a CHANGE to the sinks, which
    // is also what keeps a game's own `pause(id)` from being undone every frame.
    if let Some(mut audio) = world.get_resource_mut::<gizmo::audio::AudioManager>() {
        audio.set_all_paused(editor_state.is_paused());
    }
    // …and the clock: see `hold_clock`.
    hold_clock(world, editor_state);
}

/// Reloads the scripts and shaders the asset watcher saw change.
fn hot_reload(world: &mut World, editor_state: &mut EditorState, state: &mut StudioState) {
    // --- HOT RELOAD POLLING SİSTEMİ ---
    if let Some(watcher) = &mut state.asset_watcher {
        let changes = watcher.poll_changes();
        if !changes.is_empty() {
            for changed_path in changes {
                let path_str = changed_path.to_string_lossy().to_string();
                let is_script = path_str.ends_with(".lua");

                if is_script {
                    editor_state.log_info(&format!("🔥 Script Hot-Reload: {}", path_str));
                    if let Some(mut engine) =
                        world.get_resource_mut::<gizmo::scripting::ScriptEngine>()
                    {
                        if let Err(e) = engine.load_script(&path_str) {
                            editor_state.log_error(&format!("❌ Script yüklenemedi: {}", e));
                        }
                    }
                } else if path_str.ends_with(".wgsl") {
                    // This used to push a `ShaderReloadEvent` and stop, under a comment reading
                    // "WGPU Shader hot reload events can be integrated here similarly". Nothing
                    // consumed that event — not one reader anywhere — so the editor logged that a
                    // reload was happening and the pipelines kept running the shaders they were
                    // built with. `Renderer::rebuild_shaders` already existed and already worked;
                    // the only demo that hot-reloads shaders (`rpg_demo`) calls it directly. So
                    // does this now, and the dead event type went with it.
                    //
                    // The rebuild itself is the render hook's: the renderer is not in the game
                    // world (the app keeps it for the draw), so asking the world for it found
                    // nothing, and from 2026-11-09 until this line this logged a rebuild that never
                    // happened. The update asks; the render hook, which holds the renderer, does.
                    request_shader_rebuild(world);
                    editor_state.log_info(&format!("🔥 Shader hot-reload: {} yeniden derleniyor", path_str));
                }
            }
        }
    }
}

/// A game hooked into the studio, one frame of its own; whether there is one. Its frame waits for
/// its setup, which the render hook runs.
fn step_hooked_game(world: &mut World, state: &mut StudioState, dt: f32, input: &Input) -> bool {
    let Some(game) = state.game.clone() else {
        return false;
    };
    if world.get_resource::<crate::pie::GameSetupPending>().is_none() {
        (game.update)(world, dt, input);
        // The Game view shows the game's own camera, whichever it is this frame.
        if let Some(camera) = crate::pie::game_camera(world, state.editor_camera) {
            state.game_camera = camera;
        }
    }
    true
}

/// The input a game sees in the Game panel: the window's, its pointer moved into the panel's frame
/// (`EditorState::game_view_pointer`, the panel's render-target pixels), or far outside it when the
/// pointer is not over the panel — so a game's UI is neither hovered nor clicked through another
/// panel.
///
/// The pointer used to be the window's: a game's UI, laid out to the window, was hit by a pointer
/// in window pixels while drawn into a panel of another size and place, so a button in the Game
/// panel answered a click somewhere else on screen, or nowhere.
fn game_view_input(input: &Input, pointer: Option<(f32, f32)>) -> Input {
    let mut game = input.clone();
    let (x, y) = pointer.unwrap_or((-1.0e6, -1.0e6));
    game.set_mouse_position(x, y);
    game
}

/// The play world's `WindowInfo` for the length of a game's step: the Game panel's render target —
/// the window the game is drawn into, which its UI is laid out to. [`GameWindow::leave`] puts the
/// window's own back: `WindowInfo` goes back to the edited world at ⏹, and the editor's views read
/// it.
struct GameWindow {
    window: Option<WindowInfo>,
}

impl GameWindow {
    fn enter(world: &mut World) -> Self {
        let window = world.get_resource::<WindowInfo>().map(|w| *w);
        if let Some((w, h)) = world.get_resource::<gizmo::renderer::components::GameRenderTarget>().map(|t| (t.0.width, t.0.height)) {
            world.insert_resource(WindowInfo::new(w as f32, h as f32));
        }
        Self { window }
    }

    fn leave(self, world: &mut World) {
        if let Some(window) = self.window {
            world.insert_resource(window);
        }
    }
}

/// The editor's default bindings, into a play world that has none: arrows and WASD, and J K L U.
fn insert_default_action_map(world: &mut World) {
    let has_am = world
        .try_get_resource::<gizmo::core::input::ActionMap>()
        .is_ok();
    if !has_am {
        let mut am = gizmo::core::input::ActionMap::new();
        use gizmo::prelude::KeyCode;
        // Yön tuşları (Ok tuşları)
        am.bind_key("Up", KeyCode::ArrowUp as u32);
        am.bind_key("Down", KeyCode::ArrowDown as u32);
        am.bind_key("Left", KeyCode::ArrowLeft as u32);
        am.bind_key("Right", KeyCode::ArrowRight as u32);
        // Alternatif yön: WASD
        am.bind_key("Up", KeyCode::KeyW as u32);
        am.bind_key("Down", KeyCode::KeyS as u32);
        am.bind_key("Left", KeyCode::KeyA as u32);
        am.bind_key("Right", KeyCode::KeyD as u32);
        // Saldırı tuşları: J=LightPunch, K=HeavyPunch, L=LightKick, U=HeavyKick
        am.bind_key("LightPunch", KeyCode::KeyJ as u32);
        am.bind_key("HeavyPunch", KeyCode::KeyK as u32);
        am.bind_key("LightKick", KeyCode::KeyL as u32);
        am.bind_key("HeavyKick", KeyCode::KeyU as u32);
        world.insert_resource(am);
    }
}

/// Should the **editor** step physics itself this frame?
///
/// Only outside a play session. While ▶ is down `gizmo::systems::PlayLoop` owns the frame — it is
/// the same step an exported game takes, which is the whole point of it existing — and a second
/// stepper on top of it is not a nuance, it is a different game:
///
/// - **▶ ran the simulation at roughly twice wall-clock.** `main.rs`'s update hook called
///   `cpu_physics_step_system(world, dt)` unconditionally, and then `handle_simulation` ran
///   `PlayLoop::step`, which spends its own 60 Hz accumulator. Two steps per rendered frame, one
///   of them invisible: a scene that behaved one way in the editor behaved another when exported,
///   which is exactly the drift `PlayLoop` was extracted (`9cbdddf`) to make impossible.
/// - **⏸ did not stop anything falling.** Pausing only stops `PlayLoop`; nothing sets
///   `PhysicsWorld::is_paused`, so the "⏸ DURAKLATILDI" overlay was painted over bodies that were
///   still moving under the editor's own step.
///
/// Edit mode keeps stepping — that is how a designer sees a stack settle — and is unaffected.
///
/// Answers `true` when the world carries no `EditorState` at all: a caller with no editor is not
/// in a play session, and the safe reading is "step it".
pub fn editor_owns_the_physics_step(world: &World) -> bool {
    world
        .try_get_resource::<EditorState>()
        .map(|editor| !editor.is_in_play_session())
        .unwrap_or(true)
}

/// ⏸ holds the engine's clock still: `Time::set_paused`, every frame, from the editor's mode.
///
/// Pausing stops `PlayLoop`, and the windowed loop updates `Time` every frame, paused or not — so
/// everything else that reads the clock (the frame's tweens, skeletal and sprite animation,
/// typewriters, camera shakes, particle emitters, all in the engine's `prepare_frame`) kept going
/// over a frozen game. The editor's render pipeline used to read a delta of its own that was zero
/// on pause (`animation_delta`); with the pause on the clock itself it prepares its frame with the
/// game's own `prepare_frame`, and a script's `set_time_scale` is still the game's, untouched by
/// the pause and back as it was on ▶.
pub(crate) fn hold_clock(world: &mut World, editor: &EditorState) {
    if let Some(mut time) = world.get_resource_mut::<gizmo::core::time::Time>() {
        if time.is_paused() != editor.is_paused() {
            time.set_paused(editor.is_paused());
        }
    }
}

/// The shaders changed on disk: rebuild the pipelines at the next draw — see
/// [`take_shader_rebuild`].
pub(crate) fn request_shader_rebuild(world: &mut World) {
    world.insert_resource(ShaderRebuild);
}

/// Whether a shader rebuild was asked for since the last call, cleared. The render hook calls it,
/// holding the renderer the update does not have.
pub(crate) fn take_shader_rebuild(world: &mut World) -> bool {
    world.remove_resource::<ShaderRebuild>().is_some()
}

/// A shader file changed; the pipelines are rebuilt at the next draw.
struct ShaderRebuild;

#[cfg(test)]
mod tests {
    use super::{game_view_input, GameWindow};

    use gizmo::math::Vec3;

    /// A game in the Game panel is hit by the pointer over the panel, in the panel's own frame, and
    /// by none when the pointer is elsewhere — so its UI is neither clicked from another panel nor
    /// missed because the window's pointer is in window pixels.
    #[test]
    fn the_game_sees_the_pointer_over_its_panel_and_only_that() {
        let mut input = Input::default();
        input.set_mouse_position(812.0, 433.0);
        input.on_mouse_button_pressed(0);
        let over = game_view_input(&input, Some((30.0, 20.0)));
        assert_eq!(over.mouse_position(), (30.0, 20.0));
        assert!(over.is_mouse_button_pressed(0), "the buttons are the window's");
        let elsewhere = game_view_input(&input, None);
        let (x, y) = elsewhere.mouse_position();
        assert!(x < -1.0e5 && y < -1.0e5, "far off any UI: {x}, {y}");
    }

    /// For its step the play world's window is the Game panel's render target — what its UI is laid
    /// out to — and the window's own comes back after, since ⏹ moves it to the edited world.
    #[test]
    fn the_game_steps_in_its_panel_window_and_the_window_comes_back() {
        let instance = gizmo::wgpu::Instance::new(gizmo::wgpu::InstanceDescriptor {
            backends: gizmo::wgpu::Backends::NOOP,
            backend_options: gizmo::wgpu::BackendOptions {
                noop: gizmo::wgpu::NoopBackendOptions { enable: true, ..Default::default() },
                ..Default::default()
            },
            ..gizmo::wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&gizmo::wgpu::RequestAdapterOptions::default())).expect("the noop adapter");
        let (device, _queue) = pollster::block_on(adapter.request_device(&gizmo::wgpu::DeviceDescriptor::default())).expect("a noop device");
        let texture = device.create_texture(&gizmo::wgpu::TextureDescriptor {
            label: None,
            size: gizmo::wgpu::Extent3d { width: 640, height: 360, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: gizmo::wgpu::TextureDimension::D2,
            format: gizmo::wgpu::TextureFormat::Rgba8Unorm,
            usage: gizmo::wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = std::sync::Arc::new(texture.create_view(&Default::default()));
        let mut world = World::new();
        world.insert_resource(WindowInfo::new(1920.0, 1080.0));
        world.insert_resource(gizmo::renderer::components::GameRenderTarget(gizmo::renderer::components::RenderTarget { view, width: 640, height: 360 }));
        let window = GameWindow::enter(&mut world);
        assert_eq!(world.get_resource::<WindowInfo>().map(|w| w.size()), Some((640.0, 360.0)), "the panel's target");
        window.leave(&mut world);
        assert_eq!(world.get_resource::<WindowInfo>().map(|w| w.size()), Some((1920.0, 1080.0)), "and the window's own back");
    }

    /// **The play frame is the engine's, and this file must not grow another one.**
    ///
    /// It used to be here: an accumulator, a step size, a cap and a script order — with the
    /// exported game's runtime carrying its own copy of all four. The tests that stood here were
    /// a *third* copy, a hand-written mirror of the pump they were guarding, which is a test that
    /// cannot fail when the thing it mirrors changes. The loop now lives in
    /// `gizmo::systems::PlayLoop`, is tested there against the real code, and both callers drive
    /// it. This guards the arrangement rather than restating the arithmetic.
    /// Source with its comments removed.
    ///
    /// The positive guards in this workspace cut comments because a comment satisfies a
    /// `contains`; the **negative** ones need it for the mirror-image reason, and this test is a
    /// negative one. Explaining in prose why the editor must not call `physics_step_system` used
    /// the words, and the guard read the explanation as the offence — it went red on a doc comment
    /// added right above the line it was protecting.
    ///
    /// `//` preceded by `:` is left alone so a `https://` inside a string is not mistaken for the
    /// start of a comment.
    fn code_only(src: &str) -> String {
        src.lines()
            .map(|line| {
                let bytes = line.as_bytes();
                let mut end = line.len();
                let mut i = 0;
                while i + 1 < bytes.len() {
                    if bytes[i] == b'/' && bytes[i + 1] == b'/' && (i == 0 || bytes[i - 1] != b':') {
                        end = i;
                        break;
                    }
                    i += 1;
                }
                &line[..end]
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn the_play_frame_is_the_shared_step_not_a_copy_of_it() {
        let src = include_str!("simulation.rs");
        let code = code_only(src.split("#[cfg(test)]").next().unwrap_or(""));
        let code = code.as_str();

        // Whitespace-stripped before matching. The first version of this looked for the call
        // spelled across three lines exactly as rustfmt had left it, which made it a test of the
        // formatter and of the checkout's line endings: it went red on Windows only, in CI, for a
        // file nobody had touched.
        let compact: String = code.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(
            compact.contains(".play.step("),
            "the editor must drive gizmo::systems::PlayLoop for its play frame"
        );
        for reimplemented in [
            "physics_step_system(",
            "physics_accumulator",
            "flush_commands(",
            "update_entity(",
        ] {
            assert!(
                !code.contains(reimplemented),
                "{reimplemented:?} is back in the editor's loop — the exported game and the \
                 editor are two implementations of one contract again"
            );
        }
    }

    // ── The fight HUD across ▶ / ⏸ / ⏹ ──────────────────────────────────────────────────────
    //
    // These drive the real `handle_simulation`, because what was wrong was its *condition*, not
    // any arithmetic: the HUD block asked `is_playing()`, which is false while paused, so ⏸ fell
    // into the reset branch.

    use super::handle_simulation;
    use crate::state::StudioState;
    use gizmo::editor::EditorState;
    use gizmo::prelude::*;

    fn studio_state() -> StudioState {
        StudioState {
            current_fps: 0.0,
            actual_dt: 0.016,
            editor_camera: Entity::new(0, 0),
            game_camera: Entity::new(0, 0),
            do_raycast: false,
            play: gizmo::systems::PlayLoop::new(),
            asset_watcher: None,
            gc_timer: 0.0,
            autosave_timer: 0.0,
            visible_entity_count: 0,
            draw_call_count: 0,
            session: None,
            game: None,
        }
    }

    /// How many fight-HUD health bars the world holds: the engine's HUD (`systems::fight_hud`),
    /// built by the play loop, is two `ProgressBar`s.
    fn hud_bars(world: &World) -> usize {
        world.query::<&gizmo::ui::ProgressBar>().iter().count()
    }

    /// While a fight is on, the engine's HUD follows the fighters in the world, and nothing moves
    /// the editor's camera: the fight camera this replaced framed the fighters through the Scene
    /// panel's camera, which the game is not drawn through, and held the panel while the fight
    /// lasted. A fighter that is gone takes the HUD down with it.
    #[test]
    fn a_fight_keeps_the_hud_and_leaves_the_editor_camera_alone() {
        use gizmo::physics::components::{FighterController, Transform};
        let mut world = World::new();
        world.insert_resource(gizmo::physics::world::PhysicsWorld::new());
        let camera = world.spawn();
        let at = gizmo::math::Vec3::new(0.0, 8.0, 18.0);
        world.add_component(camera, Transform::new(at));
        world.add_component(camera, gizmo::renderer::components::Camera::new(1.0, 0.1, 100.0, true));
        let mut fighters = Vec::new();
        for (id, x) in [(1u8, -2.0f32), (2, 2.0)] {
            let f = world.spawn();
            world.add_component(f, Transform::new(gizmo::math::Vec3::new(x, 0.0, 0.0)));
            world.add_component(f, FighterController::new(id));
            fighters.push(f);
        }
        let mut ed = EditorState::default();
        ed.mode = gizmo::editor::EditorMode::Play;
        let mut state = studio_state();
        state.editor_camera = camera;
        for _ in 0..30 {
            handle_simulation(&mut world, &mut ed, &mut state, 1.0 / 60.0, &Input::default());
        }
        assert_eq!(hud_bars(&world), 2, "two fighters make a fight, and its HUD");
        assert_eq!(world.query::<&Transform>().get(camera).unwrap().position, at, "the editor's camera was moved");

        world.despawn(fighters[1]);
        handle_simulation(&mut world, &mut ed, &mut state, 1.0 / 60.0, &Input::default());
        assert_eq!(hud_bars(&world), 0, "the HUD outlived its second fighter");
    }

    /// A world with a player-1 and a player-2 fighter — the pair the HUD needs before it draws.
    fn versus_world() -> World {
        let mut world = World::new();
        world.insert_resource(gizmo::physics::world::PhysicsWorld::new());
        for slot in [1u8, 2] {
            let e = world.spawn();
            world.add_component(e, Transform::new(Vec3::new(slot as f32, 0.0, 0.0)));
            world.add_component(
                e,
                gizmo::physics::components::fighter::FighterController::new(slot),
            );
        }
        world
    }

    /// **Pausing keeps the fight HUD and holds its clock.** ⏸ does not step the game, so the HUD
    /// stays as it was and the round clock does not run. (⏹ drops the play world, HUD and all —
    /// `crate::pie` — so there is nothing here to clear.)
    #[test]
    fn pausing_keeps_the_fight_hud_and_holds_its_clock() {
        let mut world = versus_world();
        let mut ed = EditorState::default();
        let mut st = studio_state();
        let input = Input::new();
        let clock = |world: &World| world.get_resource::<gizmo::systems::fight_hud::FightRound>().map(|r| r.seconds_left);

        ed.toggle_play();
        handle_simulation(&mut world, &mut ed, &mut st, 1.0 / 60.0, &input);
        assert_eq!(hud_bars(&world), 2, "two fighters, so the HUD must be up");
        let running = clock(&world).expect("a round");
        assert!(running < 99.0, "the round clock runs while playing");

        ed.toggle_pause();
        handle_simulation(&mut world, &mut ed, &mut st, 1.0 / 60.0, &input);
        assert_eq!(hud_bars(&world), 2, "the HUD survives ⏸ — the round is paused, not over");
        assert_eq!(clock(&world), Some(running), "and its clock does not tick while paused");
    }

    /// **⏸ holds the engine's clock, and only the pause is the editor's.**
    ///
    /// The engine's frame (`prepare_frame`, which the editor's render pipeline prepares with)
    /// advances skeletons, tweens, sprites and emitters by `Time::dt()`. The windowed loop updates
    /// `Time` every frame, paused or not, so unless ⏸ reaches the clock everything but physics kept
    /// moving under the pause overlay. And the pause is not the game's time scale: a script's
    /// `set_time_scale(0.5)` slows the viewport with the game, and comes back as it was on ▶.
    #[test]
    fn the_editors_pause_holds_the_clock_and_keeps_the_games_scale() {
        use gizmo::core::time::Time;

        let mut world = World::new();
        let mut time = Time::new();
        time.set_time_scale(0.5);
        world.insert_resource(time);
        let mut ed = EditorState::default();
        let mut st = studio_state();
        let input = Input::new();
        let tick = |world: &mut World| world.get_resource_mut::<Time>().expect("clock").update(1.0 / 60.0);
        let dt = |world: &World| world.get_resource::<Time>().expect("clock").dt();

        tick(&mut world);
        handle_simulation(&mut world, &mut ed, &mut st, 1.0 / 60.0, &input);
        assert!((dt(&world) - 1.0 / 120.0).abs() < 1e-6, "editing: the clock runs, at the game's scale");

        ed.toggle_play();
        tick(&mut world);
        handle_simulation(&mut world, &mut ed, &mut st, 1.0 / 60.0, &input);
        assert!((dt(&world) - 1.0 / 120.0).abs() < 1e-6, "playing: the clock runs");

        ed.toggle_pause();
        handle_simulation(&mut world, &mut ed, &mut st, 1.0 / 60.0, &input);
        assert_eq!(dt(&world), 0.0, "the frame ⏸ is pressed in still advances what reads the clock after it");
        tick(&mut world);
        handle_simulation(&mut world, &mut ed, &mut st, 1.0 / 60.0, &input);
        assert_eq!(dt(&world), 0.0, "⏸ does not hold the clock");

        ed.toggle_pause();
        handle_simulation(&mut world, &mut ed, &mut st, 1.0 / 60.0, &input);
        tick(&mut world);
        assert!((dt(&world) - 1.0 / 120.0).abs() < 1e-6, "▶ again: the game's own scale is back");
    }

    /// A shader change asks for a rebuild; the render hook — which holds the renderer the update
    /// does not — takes the request once.
    #[test]
    fn a_shader_change_is_rebuilt_by_the_render_hook_once() {
        let mut world = World::new();
        assert!(!super::take_shader_rebuild(&mut world));
        super::request_shader_rebuild(&mut world);
        super::request_shader_rebuild(&mut world);
        assert!(super::take_shader_rebuild(&mut world));
        assert!(!super::take_shader_rebuild(&mut world), "one rebuild for the changes since the last draw");
    }

    /// **The editor steps physics only when it owns the frame.**
    ///
    /// Both defects this guards were invisible from inside the editor: ▶ ran the simulation at
    /// roughly twice wall-clock (`main.rs`'s hook stepped, then `PlayLoop` stepped again), and ⏸
    /// stopped only `PlayLoop`, so the pause overlay sat over bodies that were still falling.
    #[test]
    fn the_editor_steps_physics_only_outside_a_play_session() {
        use super::editor_owns_the_physics_step;

        let mut world = World::new();
        assert!(
            editor_owns_the_physics_step(&world),
            "no editor in the world at all — nothing else is stepping, so this must"
        );

        world.insert_resource(EditorState::default());
        assert!(
            editor_owns_the_physics_step(&world),
            "edit mode: the editor's own step is how a designer watches a stack settle"
        );

        world
            .get_resource_mut::<EditorState>()
            .expect("editor")
            .toggle_play();
        assert!(
            !editor_owns_the_physics_step(&world),
            "▶: PlayLoop owns the frame, and a second stepper doubles the simulation rate"
        );

        world
            .get_resource_mut::<EditorState>()
            .expect("editor")
            .toggle_pause();
        assert!(
            !editor_owns_the_physics_step(&world),
            "⏸: paused means paused — this is the branch that let bodies keep falling"
        );

        {
            let mut ed = world.get_resource_mut::<EditorState>().expect("editor");
            ed.toggle_pause(); // Paused → Play
            ed.toggle_play(); // ⏹ → Edit
        }
        assert!(
            editor_owns_the_physics_step(&world),
            "⏹ hands the frame back to the editor"
        );
    }
}

#[cfg(test)]
mod script_reload_tests {
    // The reporting *decision* these used to cover moved into `gizmo::systems::play` with the
    // loop that makes it, and is tested there. What stays here is the engine behaviour that made
    // the decision necessary — it belongs wherever the engine is reachable, and it is the half a
    // shared helper cannot assert about itself.

    /// The defect this guards is real, and this is the engine saying so.
    ///
    /// Drives the actual `ScriptEngine` down the exact path the play loop takes for a `Script`
    /// component whose file does not exist: `reload_if_changed` fails, and `update_entity` — the
    /// only other call — reports success. Without the reporter, that pair is the whole story the
    /// user gets.
    #[test]
    fn the_engine_reports_success_for_a_script_it_never_loaded() {
        let mut engine = gizmo::scripting::ScriptEngine::new().expect("Lua VM");
        let missing = std::env::temp_dir()
            .join(format!("gizmo_absent_{}.lua", std::process::id()))
            .to_string_lossy()
            .to_string();
        let _ = std::fs::remove_file(&missing);

        assert!(
            engine.reload_if_changed(&missing).is_err(),
            "reading a file that is not there must fail; this is the one signal that exists"
        );
        assert!(
            engine
                .update_entity(1, &missing, 1.0 / 60.0, &Default::default())
                .is_ok(),
            "update_entity returning Ok for a script it never loaded is exactly why discarding \
             the reload error left the failure invisible"
        );
    }
}
