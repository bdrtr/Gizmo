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
            state.physics_accumulator -= fixed_dt;
            steps += 1;
        }
    } else {
        state.physics_accumulator = 0.0;
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
            
            // Draw Editor Selection Outlines (Blender Style Orange)
            let transforms = world.borrow::<gizmo::physics::components::Transform>();
            let global_transforms = world.borrow::<gizmo::physics::components::GlobalTransform>();
            let colliders = world.borrow::<gizmo::physics::Collider>();
            let meshes = world.borrow::<gizmo::renderer::components::Mesh>();
            for &entity in &editor_state.selection.entities {
                if let Some(t) = transforms.get(entity.id()) {
                    let mut min = gizmo::math::Vec3::new(-1.0, -1.0, -1.0);
                    let mut max = gizmo::math::Vec3::new(1.0, 1.0, 1.0);

                    if let Some(m) = meshes.get(entity.id()) {
                        let center = m.bounds.center();
                        let half = m.bounds.half_extents();
                        min = (center - half).into();
                        max = (center + half).into();
                    } else if let Some(c) = colliders.get(entity.id()) {
                        let extents: gizmo::math::Vec3 = c.compute_aabb(gizmo::math::Vec3::ZERO, gizmo::math::Quat::IDENTITY).half_extents().into();
                        min = -extents;
                        max = extents;
                    }

                    // Eşik payı ver (çizgiler objenin içine girmesin diye)
                    min = min * 1.02;
                    max = max * 1.02;

                    let model = if let Some(gt) = global_transforms.get(entity.id()) {
                        gt.matrix
                    } else {
                        t.local_matrix
                    };

                    let p0 = model.transform_point3(gizmo::math::Vec3::new(min.x, min.y, min.z));
                    let p1 = model.transform_point3(gizmo::math::Vec3::new(max.x, min.y, min.z));
                    let p2 = model.transform_point3(gizmo::math::Vec3::new(max.x, max.y, min.z));
                    let p3 = model.transform_point3(gizmo::math::Vec3::new(min.x, max.y, min.z));
                    let p4 = model.transform_point3(gizmo::math::Vec3::new(min.x, min.y, max.z));
                    let p5 = model.transform_point3(gizmo::math::Vec3::new(max.x, min.y, max.z));
                    let p6 = model.transform_point3(gizmo::math::Vec3::new(max.x, max.y, max.z));
                    let p7 = model.transform_point3(gizmo::math::Vec3::new(min.x, max.y, max.z));

                    // Draw clean orange lines around the selected object
                    let color = [1.0, 0.5, 0.0, 1.0];
                    gizmos.draw_line(p0, p1, color);
                    gizmos.draw_line(p1, p2, color);
                    gizmos.draw_line(p2, p3, color);
                    gizmos.draw_line(p3, p0, color);
                    gizmos.draw_line(p4, p5, color);
                    gizmos.draw_line(p5, p6, color);
                    gizmos.draw_line(p6, p7, color);
                    gizmos.draw_line(p7, p4, color);
                    gizmos.draw_line(p0, p4, color);
                    gizmos.draw_line(p1, p5, color);
                    gizmos.draw_line(p2, p6, color);
                    gizmos.draw_line(p3, p7, color);
                }
            }
        }
    }
}
