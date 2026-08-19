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
    }
}

/// Advances play mode: steps the game loop while playing, and takes or restores the scene
/// snapshot on the play/stop transitions.
pub fn handle_simulation(
    world: &mut World,
    editor_state: &mut EditorState,
    state: &mut StudioState,
    dt: f32,
    input: &Input,
) {
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
                    // The renderer IS a live resource here: the app loop removes it just before
                    // drawing and puts it back after, and the update hook runs before that.
                    world.resource_scope(|_world, renderer: &mut gizmo::renderer::Renderer| {
                        renderer.rebuild_shaders();
                    });
                    editor_state.log_info(&format!("🔥 Shader hot-reload: {} yeniden derlendi", path_str));
                }
            }
        }
    }

    // --- OYUN / SİMÜLASYON DÖNGÜSÜ ---
    if editor_state.is_playing() {
        // The frame of a running game is the ENGINE's, not the editor's: `PlayLoop::step` is the
        // same call an exported game makes (`demo/src/bin/gizmo_runtime.rs`). That is what lets
        // Build/Export promise the shipped game behaves like the one you just pressed ▶ on — a
        // promise that was prose here and a second implementation over there, until the two were
        // merged. What stays on this side is the console.
        //
        // The default `ActionMap` below is the one thing the editor adds and the runtime does
        // not. It was written for the fighter system deleted in `592bd6f`, which read these
        // eight names straight out of it and both filled the input buffer and picked the move.
        // The clock has since come back (`fighter_frame_system`, on `PlayLoop`'s fixed step) but
        // deliberately without those two halves: which button is a jab is the game's, and so is
        // which action names its combos are spelled in. So this stays scaffolding — bindings a
        // game actually needs belong in the scene, and then both paths get them.
        {
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

        // `editor_state` and `state` are separate bindings, so the console can be written
        // from inside the callback: the step borrows `state.play`, the reporter borrows the
        // editor, and neither is the other.
        state
            .play
            .step(world, dt, input, &mut |r| log_play_report(editor_state, r));
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

    // --- FIGHT HUD SYNC: FighterController → EditorState.fight_hud ---
    //
    // The condition is `is_in_play_session`, not `is_playing`: ⏸ leaves the mode at `Paused`, so
    // the old `is_playing()` sent every paused frame into the `else` below and **reset the HUD** —
    // the health bars vanished and the round timer jumped back to 99 the moment you paused, while
    // the pause overlay was still being drawn over the top of them. A paused fight is still a
    // fight; only the *countdown* below stops, because that is the part that spends time.
    if editor_state.is_in_play_session() {
        let fighters = world.borrow::<gizmo::physics::components::FighterController>();
        let names = world.borrow::<gizmo::core::component::EntityName>();
        let mut found_any = false;

        for (id, fighter) in fighters.iter() {
            found_any = true;
            if fighter.player_id == 1 {
                editor_state.fight_hud.p1_entity = Some(id);
                editor_state.fight_hud.p1_health = fighter.health;
                editor_state.fight_hud.p1_max_health = fighter.max_health;
                if let Some(name) = names.get(id) {
                    editor_state.fight_hud.p1_name = name.0.clone();
                }
            } else if fighter.player_id == 2 {
                editor_state.fight_hud.p2_entity = Some(id);
                editor_state.fight_hud.p2_health = fighter.health;
                editor_state.fight_hud.p2_max_health = fighter.max_health;
                if let Some(name) = names.get(id) {
                    editor_state.fight_hud.p2_name = name.0.clone();
                }
            }
        }

        editor_state.fight_hud.active = found_any && editor_state.fight_hud.p1_entity.is_some() && editor_state.fight_hud.p2_entity.is_some();

        // Timer countdown — the one part of the HUD that must not run while paused.
        if editor_state.is_playing()
            && editor_state.fight_hud.active
            && editor_state.fight_hud.timer_seconds > 0.0
        {
            editor_state.fight_hud.timer_seconds = (editor_state.fight_hud.timer_seconds - dt).max(0.0);
        }

        // --- MISSING-3: DÖVÜŞ KAMERASI ---
        // İki dövüşçü varsa kamerayı otomatik olarak aralarına konumlandır
        if editor_state.fight_hud.active {
            if let (Some(p1_id), Some(p2_id)) = (editor_state.fight_hud.p1_entity, editor_state.fight_hud.p2_entity) {
                let p1_pos;
                let p2_pos;
                {
                    let transforms = world.borrow::<gizmo::prelude::Transform>();
                    p1_pos = transforms.get(p1_id).map(|t| t.position);
                    p2_pos = transforms.get(p2_id).map(|t| t.position);
                }

                if let (Some(p1), Some(p2)) = (p1_pos, p2_pos) {
                    let midpoint = (p1 + p2) * 0.5;
                    let separation = (p2 - p1).length().max(2.0);

                    let camera_height = 1.8_f32;
                    let min_dist = 4.0_f32;
                    let camera_distance = (separation * 1.2).max(min_dist);

                    let target_pos = gizmo::math::Vec3::new(
                        midpoint.x,
                        midpoint.y + camera_height,
                        midpoint.z + camera_distance,
                    );

                    let look_target = gizmo::math::Vec3::new(
                        midpoint.x,
                        midpoint.y + camera_height * 0.5,
                        midpoint.z,
                    );

                    // Editör kamera entity'sinin Transform ve Camera bileşenlerini güncelle
                    let cam_entity_id = state.editor_camera;
                    {
                        // SAFETY: exclusive `&mut World`; Transform and Camera are distinct component types.
                        let mut transforms = unsafe { world.borrow_mut_unchecked::<gizmo::prelude::Transform>() };
                        // SAFETY: as above — Camera is a distinct component type from Transform.
                        let mut cameras = unsafe { world.borrow_mut_unchecked::<gizmo::renderer::components::Camera>() };

                        if let Some(mut t) = transforms.get_mut(cam_entity_id) {
                            // Yumuşak geçiş (lerp)
                            let lerp_speed = (5.0 * dt).min(1.0);
                            t.position = gizmo::math::Vec3::new(
                                t.position.x + (target_pos.x - t.position.x) * lerp_speed,
                                t.position.y + (target_pos.y - t.position.y) * lerp_speed,
                                t.position.z + (target_pos.z - t.position.z) * lerp_speed,
                            );

                            // Look-at: Yaw/Pitch hesapla
                            if let Some(mut cam) = cameras.get_mut(cam_entity_id) {
                                // Kamera hedefin tam üstündeyse/üstündeyken yön dikey, kamera
                                // hedefin ÜSTÜNDEYSE de sıfır olur; ikisinde de eski kod kameraya
                                // 0 ya da NaN yaw yazıyordu. Şimdi mevcut açı korunuyor.
                                let dir = look_target - t.position;
                                if let Some((yaw, pitch)) =
                                    gizmo::renderer::components::Camera::yaw_pitch_from_forward(
                                        dir, cam.yaw,
                                    )
                                {
                                    cam.yaw = yaw;
                                    cam.pitch = pitch;
                                }
                            }

                            t.update_local_matrix();
                        }
                    }
                }
            }
        }
    } else {
        // The play SESSION is over (⏹, not ⏸) — the snapshot is gone and so is the round.
        editor_state.fight_hud = gizmo::editor::editor_state::FightHudState::default();
    }

    // --- NAVMESH DEBUG GIZMOS ---
    if editor_state.open {
        if let Some(mut gizmos) = world.get_resource_mut::<gizmo::renderer::Gizmos>() {
            // Draw Navmesh Obstacles
            if let Some(grid) = world.get_resource::<gizmo::ai::pathfinding::NavGrid>() {
                for &cell in &grid.obstacles {
                    let center = grid.grid_to_world(cell);
                    let half_size = gizmo::math::Vec3::new(
                        grid.cell_size * 0.5,
                        grid.cell_size * 0.5,
                        grid.cell_size * 0.5,
                    );
                    let min = center - half_size;
                    let max = center + half_size;
                    gizmos.draw_box(min, max, [1.0, 0.0, 0.0, 0.5]); // Red boxes for obstacles
                }
            }


        }
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

/// How much time skeletal animation should advance by in the **editor's** picture this frame.
///
/// The engine's draw path takes this from [`Time::dt`](gizmo::core::time::Time::dt) — the frame
/// delta scaled by `time_scale` and clamped — and skips the update entirely at zero. The studio
/// passed its own raw frame delta instead, and the two disagreed in exactly the cases a clock
/// exists for: a script calling `set_time_scale(0.5)` in Play mode halved the game's animation and
/// left the viewport's running at full speed, and `set_time_scale(0.0)` — pause — stopped physics
/// while the skeletons kept walking. `current_time += dt * speed` is the whole of the animation
/// clock, so the delta *is* the playback rate.
///
/// Three cases, and the middle one is the reason this is a function rather than one expression:
/// **⏸ returns zero.** Pausing stops `PlayLoop`, and `Time` keeps advancing regardless (the
/// windowed loop updates it every frame, paused or not), so reading `Time::dt` alone would leave a
/// paused editor animating. Editing returns the clock too — a designer previewing a clip wants it
/// to play, and `Time::dt` is that, only clamped, so a two-second hitch no longer jumps the pose
/// two seconds forward.
pub fn animation_delta(world: &World) -> f32 {
    let paused = world
        .try_get_resource::<EditorState>()
        .map(|editor| editor.is_paused())
        .unwrap_or(false);
    if paused {
        return 0.0;
    }
    world
        .get_resource::<gizmo::core::time::Time>()
        .map(|time| time.dt())
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use gizmo::math::Vec3;

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

    // Mirror of the auto-fight-camera framing math in `handle_simulation`:
    // separation is floored at 2.0, and the camera pull-back distance is floored at
    // 4.0 (min_dist) after a 1.2x zoom-out. Guards the two boundary clamps.
    fn fight_camera_distance(p1: Vec3, p2: Vec3) -> f32 {
        let separation = (p2 - p1).length().max(2.0);
        let min_dist = 4.0_f32;
        (separation * 1.2).max(min_dist)
    }

    #[test]
    fn fight_camera_distance_respects_min_floor() {
        // Fighters almost on top of each other → separation floored to 2.0, then
        // 2.0*1.2 = 2.4 < 4.0 → distance floored to the 4.0 minimum.
        let d = fight_camera_distance(Vec3::ZERO, Vec3::new(0.2, 0.0, 0.0));
        assert!((d - 4.0).abs() < 1e-4, "close fighters must clamp to min_dist: {d}");
    }

    #[test]
    fn fight_camera_distance_scales_when_far_apart() {
        // Ten units apart → 10*1.2 = 12 wins over the 4.0 floor.
        let d = fight_camera_distance(Vec3::new(-5.0, 0.0, 0.0), Vec3::new(5.0, 0.0, 0.0));
        assert!((d - 12.0).abs() < 1e-4, "far fighters should zoom out: {d}");
    }

    #[test]
    fn fight_camera_midpoint_is_average() {
        let p1 = Vec3::new(-3.0, 1.0, 2.0);
        let p2 = Vec3::new(7.0, 3.0, -4.0);
        let midpoint = (p1 + p2) * 0.5;
        assert!((midpoint - Vec3::new(2.0, 2.0, -1.0)).length() < 1e-5);
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
            editor_camera: 0,
            game_camera: 0,
            do_raycast: false,
            play: gizmo::systems::PlayLoop::new(),
            asset_watcher: None,
            gc_timer: 0.0,
            autosave_timer: 0.0,
            visible_entity_count: 0,
            draw_call_count: 0,
        }
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

    /// **Pausing must not wipe the fight HUD.** ⏸ leaves the editor in `Paused`, which
    /// `is_playing()` reports as false — so every paused frame reset `fight_hud` to its default:
    /// the health bars disappeared and the round timer snapped back to 99, underneath a pause
    /// overlay that was still being drawn. Stopping (⏹) is the transition that legitimately ends
    /// the round, and it still does.
    #[test]
    fn pausing_keeps_the_fight_hud_and_stopping_clears_it() {
        let mut world = versus_world();
        let mut ed = EditorState::default();
        let mut st = studio_state();
        let input = Input::new();

        ed.toggle_play();
        handle_simulation(&mut world, &mut ed, &mut st, 1.0 / 60.0, &input);
        assert!(ed.fight_hud.active, "two fighters, so the HUD must be up");
        let timer_running = ed.fight_hud.timer_seconds;
        assert!(timer_running < 99.0, "the round clock runs while playing");

        ed.toggle_pause();
        handle_simulation(&mut world, &mut ed, &mut st, 1.0 / 60.0, &input);
        assert!(
            ed.fight_hud.active,
            "the HUD must survive ⏸ — the round is paused, not over"
        );
        assert_eq!(
            ed.fight_hud.timer_seconds, timer_running,
            "but the round clock must not tick while paused"
        );

        ed.toggle_play(); // Paused → Edit (⏹)
        handle_simulation(&mut world, &mut ed, &mut st, 1.0 / 60.0, &input);
        assert!(!ed.fight_hud.active, "stopping ends the round and clears the HUD");
    }

    /// **The editor's animation runs on the clock, and ⏸ stops it.**
    ///
    /// The engine's draw path advances skeletons by `Time::dt()`; the studio used its own raw
    /// frame delta, so the two disagreed exactly where a clock matters — `set_time_scale(0.5)`
    /// halved a game's animation and left the viewport at full speed, and pause stopped physics
    /// while the skeletons kept walking.
    #[test]
    fn the_editors_animation_delta_follows_the_clock_and_stops_on_pause() {
        use super::animation_delta;
        use gizmo::core::time::Time;

        let mut world = World::new();
        assert_eq!(
            animation_delta(&world),
            0.0,
            "no clock in the world means no animation, the same answer the game path gives"
        );

        let mut time = Time::new();
        time.update(1.0 / 60.0);
        world.insert_resource(time);
        world.insert_resource(EditorState::default());
        assert!(
            (animation_delta(&world) - 1.0 / 60.0).abs() < 1e-6,
            "editing: the clock's delta, so a designer can preview a clip"
        );

        // A script slowing time down must slow the viewport with it.
        {
            let mut time = world.get_resource_mut::<Time>().expect("clock");
            time.set_time_scale(0.5);
            time.update(1.0 / 60.0);
        }
        assert!(
            (animation_delta(&world) - 1.0 / 120.0).abs() < 1e-6,
            "half speed halves the animation too — this is the number that used to be full speed"
        );

        world
            .get_resource_mut::<EditorState>()
            .expect("editor")
            .toggle_play();
        assert!(
            (animation_delta(&world) - 1.0 / 120.0).abs() < 1e-6,
            "playing: still the clock"
        );

        world
            .get_resource_mut::<EditorState>()
            .expect("editor")
            .toggle_pause();
        assert_eq!(
            animation_delta(&world),
            0.0,
            "⏸ freezes the picture — `Time` keeps advancing while paused, so reading it alone \
             would leave the skeletons walking under the pause overlay"
        );
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
