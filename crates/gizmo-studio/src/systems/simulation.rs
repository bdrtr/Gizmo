use crate::state::StudioState;
use gizmo::editor::EditorState;
use gizmo::prelude::*;
pub fn handle_simulation(world: &mut World, editor_state: &mut EditorState, state: &mut StudioState, dt: f32, input: &Input) {
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
                        editor_state.log_warning(&format!(
                            "🔥 Shader Hot-Reload iskitleniyor: {}",
                            path_str
                        ));
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
            if let Some(mut engine) = world.remove_resource::<gizmo::scripting::ScriptEngine>() {
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
                if let Some(scripts) = world.borrow::<gizmo::scripting::Script>() {
                    let mut entity_calls = Vec::new();
                    for entity_id in scripts.dense.iter().map(|e| e.entity) {
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

                world.insert_resource(engine);
            }

            state.physics_accumulator += dt;
            let fixed_dt = 1.0 / 60.0;
            // Death spiral önleme
            state.physics_accumulator = state.physics_accumulator.min(fixed_dt * 16.0);

            let mut steps = 0;
            while state.physics_accumulator >= fixed_dt && steps < 16 {
                gizmo::physics::integration::physics_apply_forces_system(world, fixed_dt);
                gizmo::physics::vehicle::physics_vehicle_system(world, fixed_dt);
                gizmo::physics::system::physics_collision_system(world, fixed_dt);
                gizmo::physics::character::physics_character_system(world, fixed_dt);
                gizmo::physics::integration::physics_movement_system(world, fixed_dt);

                state.physics_accumulator -= fixed_dt;
                steps += 1;
            }
        } else {
            state.physics_accumulator = 0.0;
        }
}
