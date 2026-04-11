use crate::state::GameState;
use gizmo::math::Vec3;
use gizmo::prelude::*;
use gizmo::winit::keyboard::KeyCode;

pub fn run_scripts(
    world: &mut World,
    _state: &mut GameState,
    dt: f32,
    input: &Input,
) -> Vec<gizmo::scripting::commands::ScriptCommand> {
    let mut unhandled = Vec::new();
    let mut engine_opt = world.remove_resource::<gizmo::scripting::ScriptEngine>();
    if engine_opt.is_none() {
        return unhandled;
    }

    if let (Some(mut transforms), Some(mut vels), Some(scripts)) = (
        world.borrow_mut::<gizmo::physics::components::Transform>(),
        world.borrow_mut::<gizmo::physics::components::Velocity>(),
        world.borrow::<gizmo::scripting::Script>(),
    ) {
        let entity_ids: Vec<u32> = scripts.dense.iter().map(|e| e.entity).collect::<Vec<_>>();
        for e in entity_ids {
            let script = match scripts.get(e) {
                Some(s) => s,
                None => continue,
            };
            let t = match transforms.get_mut(e) {
                Some(t) => t,
                None => continue,
            };
            let v = match vels.get_mut(e) {
                Some(v) => v,
                None => continue,
            };
            let ctx = gizmo::scripting::engine::ScriptContext {
                entity_id: e,
                dt,
                position: [t.position.x, t.position.y, t.position.z],
                velocity: [v.linear.x, v.linear.y, v.linear.z],
                key_w: input.is_key_pressed(KeyCode::KeyW as u32),
                key_a: input.is_key_pressed(KeyCode::KeyA as u32),
                key_s: input.is_key_pressed(KeyCode::KeyS as u32),
                key_d: input.is_key_pressed(KeyCode::KeyD as u32),
                key_space: input.is_key_pressed(KeyCode::Space as u32),
                key_up: input.is_key_pressed(KeyCode::ArrowUp as u32),
                key_down: input.is_key_pressed(KeyCode::ArrowDown as u32),
                key_left: input.is_key_pressed(KeyCode::ArrowLeft as u32),
                key_right: input.is_key_pressed(KeyCode::ArrowRight as u32),
            };
            if let Some(engine) = engine_opt.as_mut() {
                let func_name = {
                    let stem = std::path::Path::new(&script.file_path)
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("on");
                    let auto_name = format!("{}_update", stem);
                    if engine.has_function(&script.file_path, &auto_name) {
                        auto_name
                    } else {
                        continue;
                    }
                };

                match engine.run_entity_update(&script.file_path, &func_name, &ctx) {
                    Ok(res) => {
                        if let Some(pos) = res.new_position {
                            t.position = Vec3::new(pos[0], pos[1], pos[2]);
                        }
                        if let Some(vel) = res.new_velocity {
                            v.linear = Vec3::new(vel[0], vel[1], vel[2]);
                        }
                    }
                    Err(err) => {
                        println!("[Lua Runtime Error] {}: {}", func_name, err);
                    }
                }
            }
        }
    }
    if let Some(engine) = engine_opt {
        unhandled = engine.flush_commands(world, dt);
        world.insert_resource(engine);
    }
    unhandled
}

pub fn process_game_commands(
    world: &mut World,
    state: &mut GameState,
    dt: f32,
    commands: Vec<gizmo::scripting::commands::ScriptCommand>,
) {
    use gizmo::scripting::commands::ScriptCommand;

    // Diyalog timer'ını güncelle
    if let Some(ref mut dlg) = state.active_dialogue {
        if dlg.timer > 0.0 {
            dlg.timer -= dt;
            if dlg.timer <= 0.0 {
                state.active_dialogue = None;
            }
        }
    }

    // Yarış timer'ı
    if state.race_status == crate::state::RaceStatus::Running {
        state.race_timer += dt;
    }

    // Kamera takip sistemi
    if let Some(target_id) = state.camera_follow_target {
        let mut target_pos = None;
        if let Some(transforms) = world.borrow::<gizmo::physics::components::Transform>() {
            if let Some(t) = transforms.get(target_id) {
                target_pos = Some(t.position);
            }
        }
        if let Some(tpos) = target_pos {
            if let Some(mut transforms) =
                world.borrow_mut::<gizmo::physics::components::Transform>()
            {
                if let Some(cam_t) = transforms.get_mut(state.player_id) {
                    let offset = Vec3::new(0.0, 4.0, 10.0);
                    cam_t.position = cam_t.position.lerp(tpos + offset, dt * 5.0);
                }
            }
        }
    }

    // Checkpoint temas kontrolü
    {
        let mut player_pos = None;
        if let Some(transforms) = world.borrow::<gizmo::physics::components::Transform>() {
            if let Some(t) = transforms.get(state.player_id) {
                player_pos = Some(t.position);
            }
        }
        if let Some(ppos) = player_pos {
            for cp in &mut state.checkpoints {
                if !cp.activated && ppos.distance(cp.position) < cp.radius {
                    cp.activated = true;
                    println!("[Race] Checkpoint {} geçildi!", cp.id);
                }
            }
            if !state.checkpoints.is_empty()
                && state.checkpoints.iter().all(|c| c.activated)
                && state.race_status == crate::state::RaceStatus::Running
            {
                state.race_status = crate::state::RaceStatus::Finished;
                println!("[Race] Yarış tamamlandı! Süre: {:.2}s", state.race_timer);
            }
        }
    }

    for cmd in commands {
        match cmd {
            ScriptCommand::ShowDialogue {
                speaker,
                text,
                duration,
            } => {
                state.active_dialogue = Some(crate::state::DialogueEntry {
                    speaker,
                    text,
                    timer: duration,
                });
            }
            ScriptCommand::HideDialogue => {
                state.active_dialogue = None;
            }
            ScriptCommand::TriggerCutscene(name) => {
                state.active_cutscene = Some(name.clone());
                state.free_cam = false;
                println!("[Cutscene] Başladı: {}", name);
            }
            ScriptCommand::EndCutscene => {
                state.active_cutscene = None;
                state.free_cam = true;
                println!("[Cutscene] Bitti.");
            }
            ScriptCommand::AddCheckpoint {
                id,
                position,
                radius,
            } => {
                state.checkpoints.push(crate::state::Checkpoint {
                    id,
                    position,
                    radius,
                    activated: false,
                });
            }
            ScriptCommand::ActivateCheckpoint(id) => {
                if let Some(cp) = state.checkpoints.iter_mut().find(|c| c.id == id) {
                    cp.activated = true;
                }
            }
            ScriptCommand::StartRace => {
                state.race_status = crate::state::RaceStatus::Running;
            }
            ScriptCommand::FinishRace { winner_name: _ } => {
                state.race_status = crate::state::RaceStatus::Finished;
            }
            ScriptCommand::ResetRace => {
                for cp in &mut state.checkpoints {
                    cp.activated = false;
                }
                state.race_timer = 0.0;
                state.race_status = crate::state::RaceStatus::Idle;
            }
            ScriptCommand::SetCameraTarget(entity_id) => {
                state.camera_follow_target = Some(entity_id);
                state.free_cam = false;
            }
            ScriptCommand::SetCameraFov(fov) => {
                if let Some(mut cameras) = world.borrow_mut::<gizmo::renderer::components::Camera>()
                {
                    if let Some(cam) = cameras.get_mut(state.player_id) {
                        cam.fov = fov.to_radians();
                    }
                }
            }
            ScriptCommand::LoadScene(path) => {
                println!("[ScriptSys] Sahne yükleme isteği eklendi: {}", path);
                world.insert_resource(crate::state::SceneLoadRequest(path));
            }
            _ => {}
        }
    }
}
