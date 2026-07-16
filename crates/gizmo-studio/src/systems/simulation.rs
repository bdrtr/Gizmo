use crate::state::StudioState;
use gizmo::editor::EditorState;
use gizmo::prelude::*;
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
                    editor_state
                        .log_warning(&format!("🔥 Shader Hot-Reload iskitleniyor: {}", path_str));
                    // WGPU Shader hot reload events can be integrated here similarly
                    let has_events = world.get_resource::<gizmo::core::event::Events<crate::state::ShaderReloadEvent>>().is_some();
                    if !has_events {
                        world.insert_resource(gizmo::core::event::Events::<
                            crate::state::ShaderReloadEvent,
                        >::new());
                    }
                    if let Some(mut events) = world.get_resource_mut::<gizmo::core::event::Events<crate::state::ShaderReloadEvent>>() {
                            events.push(crate::state::ShaderReloadEvent);
                        }
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
                let mut entity_calls: Vec<(u32, String)> = Vec::new();
                for (entity_id, _) in scripts.iter() {
                    if let Some(script) = scripts.get(entity_id) {
                        entity_calls.push((entity_id, script.file_path.clone()));
                    }
                }
                drop(scripts);

                for (entity_id, path) in entity_calls {
                    let _ = engine.reload_if_changed(&path);
                    if let Err(e) = engine.update_entity(entity_id, &path, dt) {
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
                                let dir = (look_target - t.position).normalize();
                                // Invert Camera::get_front(): fx = cos(yaw)cos(pitch),
                                // fy = sin(pitch), fz = sin(yaw)cos(pitch)
                                cam.yaw = dir.z.atan2(dir.x);
                                cam.pitch = dir.y.asin();
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
