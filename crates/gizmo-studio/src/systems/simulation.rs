use crate::state::StudioState;
use gizmo::editor::EditorState;
use gizmo::prelude::*;

/// What the console should be told about one script's load attempt this frame.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ScriptReload {
    /// Nothing new: it loaded, or it is failing the same way it was already failing.
    Quiet,
    /// It has just started failing.
    Broke(String),
    /// It was failing and now loads.
    Recovered,
}

/// Decide what to say about a reload result, and remember the answer.
///
/// The studio ran `let _ = engine.reload_if_changed(&path);` — result discarded — and then called
/// `update_entity`, which for a script that never loaded takes its `else` branch, emits a `trace!`
/// and returns `Ok(())`. So a Script component pointing at a file that is not there did nothing,
/// forever, and said nothing: the editor's own `➕ Bileşen Ekle ▸ Script` stamps
/// `scripts/new_script.lua`, a path no part of the editor creates. Press ▶ and the console stays
/// empty while the script never runs.
///
/// Reporting it needs a memory, or it is 60 identical lines a second. `failed` holds the paths
/// that are currently broken, so the message is emitted on the way in and the recovery on the way
/// out — the two moments a person needs to know about.
pub(crate) fn script_reload_report(
    failed: &mut std::collections::BTreeSet<String>,
    path: &str,
    result: Result<bool, String>,
) -> ScriptReload {
    match result {
        Ok(_) => {
            if failed.remove(path) {
                ScriptReload::Recovered
            } else {
                ScriptReload::Quiet
            }
        }
        Err(e) => {
            if failed.insert(path.to_string()) {
                ScriptReload::Broke(e)
            } else {
                ScriptReload::Quiet
            }
        }
    }
}
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
        // SCRIPT ENGINE UPDATE: Sadece "Play" modundayken oyun mantığını işlet
        world.resource_scope(|world, engine: &mut gizmo::scripting::ScriptEngine| {
            if let Err(e) = engine.update(world, input, dt) {
                editor_state.log_error(&format!("Script Error: {}", e));
            }

            // Flush commands directly
            let unhandled_commands = engine.flush_commands(world, dt);
            for _cmd in unhandled_commands {
                // For now, audio/scene commands can be skipped or warned inside the editor
                // as the editor shouldn't suddenly switch scenes due to a script.
            }

            // Call per-entity updates
            let scripts = world.borrow::<gizmo::scripting::Script>();
            {
                // The entity's own property values ride along: scripts are loaded per path, so a
                // per-entity value cannot live in the shared Lua environment.
                let mut entity_calls: Vec<(u32, String, std::collections::BTreeMap<String, gizmo::scripting::ScriptValue>)> =
                    Vec::new();
                for (entity_id, _) in scripts.iter() {
                    if let Some(script) = scripts.get(entity_id) {
                        entity_calls.push((
                            entity_id,
                            script.file_path.clone(),
                            script.properties.clone(),
                        ));
                    }
                }
                drop(scripts);

                for (entity_id, path, properties) in entity_calls {
                    match script_reload_report(
                        &mut state.failed_scripts,
                        &path,
                        engine.reload_if_changed(&path),
                    ) {
                        ScriptReload::Broke(e) => editor_state.log_error(&format!(
                            "❌ Script yüklenemedi: {} — {}. Entity'nin script'i ÇALIŞMIYOR.",
                            path, e
                        )),
                        ScriptReload::Recovered => editor_state
                            .log_info(&format!("✅ Script yeniden yüklendi: {}", path)),
                        ScriptReload::Quiet => {}
                    }
                    if let Err(e) = engine.update_entity(entity_id, &path, dt, &properties) {
                        editor_state.log_warning(&format!("Entity script error: {}", e));
                    }
                }
            }

            // Pump logs to editor console
            if let Ok(mut logs) = engine.log_queue.lock() {
                for (level, msg) in logs.drain(..) {
                    match level.as_str() {
                        "error" => editor_state.log_error(&format!("[Lua] {}", msg)),
                        "warn" => editor_state.log_warning(&format!("[Lua] {}", msg)),
                        _ => editor_state.log_info(&format!("[Lua] {}", msg)),
                    }
                }
            }
        });

        state.physics_accumulator += dt;
        let fixed_dt = 1.0 / 60.0;
        // Death spiral önleme
        state.physics_accumulator = state.physics_accumulator.min(fixed_dt * 16.0);

        let mut steps = 0;
        while state.physics_accumulator >= fixed_dt && steps < 16 {
            gizmo::physics::system::physics_step_system(world, fixed_dt);
            
            // Fighter System: Dövüş mekanikleri (Input Buffer, Hitstop) her fizik karesinde güncellenir
            {
                let has_am = world.try_get_resource::<gizmo::core::input::ActionMap>().is_ok();
                if !has_am {
                    let mut am = gizmo::core::input::ActionMap::new();
                    use gizmo::prelude::KeyCode;
                    // Yön tuşları (Ok tuşları)
                    am.bind_key("Up",    KeyCode::ArrowUp as u32);
                    am.bind_key("Down",  KeyCode::ArrowDown as u32);
                    am.bind_key("Left",  KeyCode::ArrowLeft as u32);
                    am.bind_key("Right", KeyCode::ArrowRight as u32);
                    // Alternatif yön: WASD
                    am.bind_key("Up",    KeyCode::KeyW as u32);
                    am.bind_key("Down",  KeyCode::KeyS as u32);
                    am.bind_key("Left",  KeyCode::KeyA as u32);
                    am.bind_key("Right", KeyCode::KeyD as u32);
                    // Saldırı tuşları: J=LightPunch, K=HeavyPunch, L=LightKick, U=HeavyKick
                    am.bind_key("LightPunch", KeyCode::KeyJ as u32);
                    am.bind_key("HeavyPunch", KeyCode::KeyK as u32);
                    am.bind_key("LightKick",  KeyCode::KeyL as u32);
                    am.bind_key("HeavyKick",  KeyCode::KeyU as u32);
                    world.insert_resource(am);
                }
                
                if let Ok(_am) = world.try_get_resource::<gizmo::core::input::ActionMap>() {
                    // gizmo::physics::system::physics_fighter_system(world, input, &am);
                }
                
                // Hit Detection: Hitbox ↔ Hurtbox çarpışma algılama
                // let hit_events = gizmo::physics::system::hit_detection_system(world);
                /*
                for event in &hit_events {
                    editor_state.log_info(&format!(
                        "💥 HIT! Saldırgan:{} → Kurban:{} | Hasar: {:.1} | Pozisyon: ({:.1}, {:.1}, {:.1})",
                        event.attacker_id, event.victim_id, event.damage,
                        event.hit_point.x, event.hit_point.y, event.hit_point.z
                    ));
                }
                */
            }
            
            state.physics_accumulator -= fixed_dt;
            steps += 1;
        }
    } else {
        state.physics_accumulator = 0.0;
    }

    // --- FIGHT HUD SYNC: FighterController → EditorState.fight_hud ---
    if editor_state.is_playing() {
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

        // Timer countdown
        if editor_state.fight_hud.active && editor_state.fight_hud.timer_seconds > 0.0 {
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
        // Play modundan çıkınca HUD'u sıfırla
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

#[cfg(test)]
mod tests {
    use gizmo::math::Vec3;

    // Mirror of the fixed-timestep pump in `handle_simulation` (the play-mode physics
    // loop). Same accumulator, same fixed_dt, same death-spiral clamp + 16-step cap.
    // Returns (leftover_accumulator, steps_taken) so the invariants are observable
    // without a live World / ScriptEngine / PhysicsWorld.
    fn pump(mut accumulator: f32, dt: f32) -> (f32, u32) {
        accumulator += dt;
        let fixed_dt = 1.0 / 60.0;
        // Death spiral önleme
        accumulator = accumulator.min(fixed_dt * 16.0);

        let mut steps = 0;
        while accumulator >= fixed_dt && steps < 16 {
            accumulator -= fixed_dt;
            steps += 1;
        }
        (accumulator, steps)
    }

    /// One real-time frame at exactly the fixed rate advances the sim exactly once
    /// and leaves (essentially) no leftover.
    #[test]
    fn pump_single_frame_is_one_step() {
        let (leftover, steps) = pump(0.0, 1.0 / 60.0);
        assert_eq!(steps, 1);
        assert!(leftover.abs() < 1e-4, "leftover accumulator: {leftover}");
    }

    /// A sub-frame dt performs no step but banks time; two half-frames then trigger
    /// exactly one step (accumulator carry-over invariant).
    #[test]
    fn pump_sub_frame_banks_then_steps() {
        let (acc1, steps1) = pump(0.0, 1.0 / 120.0);
        assert_eq!(steps1, 0, "half a frame must not step yet");
        assert!(acc1 > 0.0);

        let (acc2, steps2) = pump(acc1, 1.0 / 120.0);
        assert_eq!(steps2, 1, "two half-frames = one step");
        assert!(acc2.abs() < 1e-4, "leftover after the step: {acc2}");
    }

    /// A catastrophic hitch (1 full second) must NOT spiral: the accumulator is
    /// clamped to 16*fixed_dt and the loop is hard-capped at 16 steps, so the sim
    /// never tries to simulate a second of physics in one frame.
    #[test]
    fn pump_huge_dt_is_capped_at_16_steps() {
        let (leftover, steps) = pump(0.0, 1.0);
        assert_eq!(steps, 16, "step count must be capped");
        // Clamp = 16*fixed_dt, exactly drained by 16 steps → ~0 leftover, and never
        // the ~0.78s of un-simulated time a naive loop would carry.
        assert!(leftover < 1.0 / 60.0, "leftover must be below one step: {leftover}");
    }

    /// Even with pre-existing banked time plus a big dt, the clamp holds the step
    /// count at the 16 ceiling (idempotent under repeated overload).
    #[test]
    fn pump_overload_stays_capped_with_prior_accumulator() {
        let (_, steps) = pump(0.5, 0.5);
        assert_eq!(steps, 16);
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
}

#[cfg(test)]
mod script_reload_tests {
    use super::*;

    /// A script that cannot load must be reported — once, and again only when it changes state.
    ///
    /// The studio discarded this result entirely (`let _ = engine.reload_if_changed(&path);`), and
    /// the call after it, `update_entity`, returns `Ok(())` for a script it never loaded. So the
    /// two places that knew both stayed quiet and the entity's script simply never ran. Reporting
    /// it naively is no better: this runs per entity per frame, so an unconditional log is 60
    /// identical lines a second on top of whatever the user is actually reading.
    #[test]
    fn a_broken_script_is_reported_once_and_its_recovery_too() {
        let mut failed = std::collections::BTreeSet::new();
        let path = "scripts/new_script.lua";

        // Frame 1 — the file is not there. The editor's own `Script` component points here.
        assert_eq!(
            script_reload_report(&mut failed, path, Err("Script okunamadı".into())),
            ScriptReload::Broke("Script okunamadı".into()),
            "the first failure has to reach the console; nothing else in the chain says a word"
        );

        // Frames 2..n — still broken. Silence, or the console is unusable.
        for _ in 0..120 {
            assert_eq!(
                script_reload_report(&mut failed, path, Err("Script okunamadı".into())),
                ScriptReload::Quiet,
                "a failure that is already on screen must not be printed again every frame"
            );
        }

        // The user creates the file.
        assert_eq!(
            script_reload_report(&mut failed, path, Ok(true)),
            ScriptReload::Recovered,
            "coming back has to be reported too — otherwise the last word on screen is an error \
             about a script that is now running fine"
        );
        assert_eq!(
            script_reload_report(&mut failed, path, Ok(false)),
            ScriptReload::Quiet,
            "and then it goes quiet again"
        );
        assert!(failed.is_empty(), "a recovered path must not stay in the failed set");
    }

    /// Two broken scripts are two reports, not one: the set is keyed by path.
    #[test]
    fn each_script_is_tracked_on_its_own() {
        let mut failed = std::collections::BTreeSet::new();
        assert!(matches!(
            script_reload_report(&mut failed, "a.lua", Err("yok".into())),
            ScriptReload::Broke(_)
        ));
        assert!(
            matches!(
                script_reload_report(&mut failed, "b.lua", Err("yok".into())),
                ScriptReload::Broke(_)
            ),
            "a second broken script was swallowed because the first one had already reported"
        );
        assert_eq!(
            script_reload_report(&mut failed, "a.lua", Ok(true)),
            ScriptReload::Recovered
        );
        assert_eq!(failed.iter().collect::<Vec<_>>(), ["b.lua"]);
    }

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
