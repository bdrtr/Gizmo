use gizmo::prelude::*;

pub mod scene;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct EntityName(pub String);

pub mod state;       pub use state::*;
pub mod scene_setup; pub use scene_setup::*;
pub mod demo_car;    pub use demo_car::*;
pub mod ui;          pub use ui::*;
pub mod render_pipeline; pub use render_pipeline::*;
pub mod systems;     pub use systems::*;
pub mod gizmo_input; pub use gizmo_input::*;
pub mod camera;      pub use camera::*;
pub mod hot_reload_sys; pub use hot_reload_sys::*;
pub mod components;

fn main() {
    let mut app = App::new("Gizmo Engine — Rust 3D Motor", 1280, 720);

    // ── SETUP ──────────────────────────────────────────────────────────────
    app = app.set_setup(|world, renderer| {
        scene_setup::setup_default_scene(world, renderer) // Galton Kutusu (Pachinko) Deneysel Sahnesi
    });

    // ── UPDATE ─────────────────────────────────────────────────────────────
    app = app.set_update(|world, state, dt, input| {
        state.current_fps = 1.0 / dt;

        // Hot-reload texture dosya takibi
        poll_hot_reload(world, state);

        // Seçim isteği uygula
        if let Some(mut events) = world.get_resource_mut::<gizmo::core::event::Events<crate::state::SelectionEvent>>() {
            for ev in events.drain() {
                state.inspector_selected_entity = Some(ev.entity_id);
            }
        }

        // Mouse tıklaması → raycast bayrağı
        if input.is_mouse_button_just_pressed(mouse::LEFT) { state.do_raycast = true; }
        if input.is_mouse_button_just_released(mouse::LEFT) { state.dragging_axis = None; }

        let is_in_game = world.get_resource::<crate::state::AppMode>().map(|m| *m) == Some(crate::state::AppMode::InGame);

        // Kamera fare ile döndür (Serbest kamera modunda)
        if is_in_game && state.free_cam && input.is_mouse_button_pressed(mouse::RIGHT) {
            if let Some(mut cameras) = world.borrow_mut::<Camera>() {
                if let Some(cam) = cameras.get_mut(state.player_id) {
                    let delta = input.mouse_delta();
                    cam.yaw   += delta.0 * 0.002;
                    cam.pitch -= delta.1 * 0.002;
                    cam.pitch  = cam.pitch.clamp(-1.5, 1.5);
                }
            }
        }

        // Serbest kamera WASD hareketi
        if is_in_game && state.free_cam {
            let speed = 10.0 * dt;
            let mut f = Vec3::ZERO;
            let mut r = Vec3::ZERO;
            if let Some(cameras) = world.borrow::<Camera>() {
                if let Some(cam) = cameras.get(state.player_id) {
                    f = cam.get_front();
                    r = cam.get_right();
                }
            }
            let mut move_delta = Vec3::ZERO;
            if input.is_key_pressed(KeyCode::KeyW as u32) { move_delta += f * speed; }
            if input.is_key_pressed(KeyCode::KeyS as u32) { move_delta -= f * speed; }
            if input.is_key_pressed(KeyCode::KeyA as u32) { move_delta -= r * speed; }
            if input.is_key_pressed(KeyCode::KeyD as u32) { move_delta += r * speed; }
            if move_delta.length_squared() > 0.0 {
                if let Some(mut trans) = world.borrow_mut::<Transform>() {
                    if let Some(t) = trans.get_mut(state.player_id) {
                        t.position += move_delta;
                    }
                }
            }
        }

        // Ray hesapla
        let (mx, my) = input.mouse_position();
        let (ww, wh) = input.window_size();
        let ndc_x = (2.0 * mx) / ww - 1.0;
        let ndc_y = 1.0 - (2.0 * my) / wh;
        let current_ray = build_ray(world, state.player_id, ndc_x, ndc_y, ww, wh);

        // Gizmo Input (raycast + drag)
        if let Some(ray) = current_ray {
            let do_rc = state.do_raycast && !state.egui_wants_pointer;
            if do_rc { state.do_raycast = false; }
            handle_gizmo_input(world, state, ray, do_rc);
        }

        // Gizmo görsel senkron
        sync_gizmos(world, state);

        // Zaman kaynağı
        state.total_elapsed += dt as f64;
        world.insert_resource(Time { dt, elapsed_seconds: state.total_elapsed });

        // Fizik (sabit adım)
        state.physics_accumulator += dt;
        let fixed_dt = 1.0 / state.target_physics_fps;
        // Death spiral önleme: accumulator'ı max 16 adımla sınırla
        state.physics_accumulator = state.physics_accumulator.min(fixed_dt * 16.0);
        let mut steps = 0;
        while state.physics_accumulator >= fixed_dt && steps < 16 {
            gizmo::physics::system::physics_collision_system(world, fixed_dt);
            gizmo::physics::character::physics_character_system(world, fixed_dt);
            if let Some(jw) = world.get_resource::<gizmo::physics::JointWorld>() {
                gizmo::physics::solve_constraints(&*jw, world, fixed_dt);
            }
            gizmo::physics::vehicle::physics_vehicle_system(world, fixed_dt);
            
            // AI Navigasyon sistemi
            gizmo_ai::ai_navigation_system(world, fixed_dt);
            
            gizmo::physics::integration::physics_movement_system(world, fixed_dt);
            state.physics_accumulator -= fixed_dt;
            steps += 1;
        }

        // Vehicle Controller (Input to Engine/Steering)
        if is_in_game {
            if let Some(mut vehicles) = world.borrow_mut::<gizmo::physics::vehicle::VehicleController>() {
                let engine_power = 2000.0;
                let mut current_engine = 0.0;
                let mut current_steer = 0.0;
                let mut current_brake = 0.0;
                
                if input.is_key_pressed(KeyCode::ArrowUp as u32) { current_engine = engine_power; }
                if input.is_key_pressed(KeyCode::ArrowDown as u32) { current_engine = -engine_power * 0.5; }
                if input.is_key_pressed(KeyCode::ArrowLeft as u32) { current_steer = 0.5; } // Rad
                if input.is_key_pressed(KeyCode::ArrowRight as u32) { current_steer = -0.5; }
                if input.is_key_pressed(KeyCode::Space as u32) { current_brake = 5000.0; }

                for entity in vehicles.entity_dense.clone() {
                    if let Some(v) = vehicles.get_mut(entity) {
                        v.engine_force = current_engine;
                        v.steering_angle = current_steer;
                        v.brake_force = current_brake;
                    }
                }
            }
        }

        transform_hierarchy_system(world);
        
        // AI Hedef Güncelleme (Oyuncuyu Takip Et)
        if let Some(mut agents) = world.borrow_mut::<gizmo_ai::NavAgent>() {
            let player_pos = if let Some(transforms) = world.borrow::<Transform>() {
                transforms.get(state.player_id).map(|t| t.position)
            } else { None };
            
            if let Some(ppos) = player_pos {
                let keys = agents.entity_dense.clone();
                for e in keys {
                    if let Some(a) = agents.get_mut(e) {
                         a.target = Some(ppos);
                    }
                }
            }
        }

        // Lua Script motoru güncellemeleri (Input, Time, Scene durumlarını aktarır + global on_update çağırır)
        let mut engine_opt = world.remove_resource::<gizmo::scripting::ScriptEngine>();
        if let Some(mut engine) = engine_opt.take() {
            let _ = engine.update(world, input, dt);
            world.insert_resource(engine);
        }

        // Script bileşeni olan entity'ler için per-entity script çalıştır (eski run_scripts)
        // NOT: run_scripts her entity için "on_update" çağırdığından, global "on_update" fonksiyonu
        // race_map1.lua ve car_controller.lua arasında ezilebilir. Bunun çözümü her component'in scriptini bağlamalı yapmak.
        // Şimdilik sadece input entegrasyonu için engine.update() eklendi.
        let cmds = run_scripts(world, state, dt, input);

        // Lua'dan gelen oyun komutlarını işle
        process_game_commands(world, state, dt, cmds);
    });

    // ── UI ─────────────────────────────────────────────────────────────────
    app = app.set_ui(|world, state, ctx| {
        state.egui_wants_pointer = ctx.is_pointer_over_area();
        render_ui(ctx, state, world);
    });

    app = app.set_render(|world, state, encoder, view, renderer: &mut gizmo::renderer::Renderer, light_time| {
        // Post-process ayarlarını uygula
        {
            if let Some(pp) = world.get_resource::<gizmo::renderer::renderer::PostProcessUniforms>() {
                renderer.update_post_process(&renderer.queue, *pp);
            }
        }
        
        // Shader reload isteği
        if let Some(mut events) = world.get_resource_mut::<gizmo::core::event::Events<crate::state::ShaderReloadEvent>>() {
            if !events.is_empty() {
                renderer.rebuild_shaders();
                events.clear();
            }
        }

        render_pipeline::execute_render_pipeline(world, state, encoder, view, renderer, light_time);
    });

    app.run();
}

// ── Yardımcı Fonksiyonlar ──────────────────────────────────────────────────

fn build_ray(world: &World, player_id: u32, ndc_x: f32, ndc_y: f32, ww: f32, wh: f32) -> Option<gizmo::math::Ray> {
    if let (Some(cameras), Some(transforms)) = (world.borrow::<Camera>(), world.borrow::<Transform>()) {
        if let (Some(cam), Some(cam_t)) = (cameras.get(player_id), transforms.get(player_id)) {
            let proj    = Mat4::perspective_rh(cam.fov, ww / wh, cam.near, cam.far);
            let view    = cam.get_view(cam_t.position);
            let inv_vp  = (proj * view).inverse();
            let far_pt  = inv_vp * Vec4::new(ndc_x, ndc_y, 1.0, 1.0);
            let near_pt = inv_vp * Vec4::new(ndc_x, ndc_y, 0.0, 1.0);
            let world_near = Vec3::new(near_pt.x / near_pt.w, near_pt.y / near_pt.w, near_pt.z / near_pt.w);
            let world_far  = Vec3::new(far_pt.x / far_pt.w, far_pt.y / far_pt.w, far_pt.z / far_pt.w);
            return Some(gizmo::math::Ray::new(world_near, (world_far - world_near).normalize()));
        }
    }
    None
}

fn run_scripts(world: &mut World, _state: &mut GameState, dt: f32, input: &Input) -> Vec<gizmo::scripting::commands::ScriptCommand> {
    let mut unhandled = Vec::new();
    let mut engine_opt = world.remove_resource::<gizmo::scripting::ScriptEngine>();
    if engine_opt.is_none() { return unhandled; }

    if let (Some(mut transforms), Some(mut vels), Some(scripts)) = (
        world.borrow_mut::<Transform>(),
        world.borrow_mut::<Velocity>(),
        world.borrow::<gizmo::scripting::Script>(),
    ) {
        let entity_ids: Vec<u32> = scripts.entity_dense.clone();
        for e in entity_ids {
            let script = match scripts.get(e) { Some(s) => s, None => continue };
            let t = match transforms.get_mut(e) { Some(t) => t, None => continue };
            let v = match vels.get_mut(e)        { Some(v) => v, None => continue };
            let ctx = gizmo::scripting::engine::ScriptContext {
                entity_id: e, dt,
                position: [t.position.x, t.position.y, t.position.z],
                velocity: [v.linear.x, v.linear.y, v.linear.z],
                key_w:     input.is_key_pressed(KeyCode::KeyW as u32),
                key_a:     input.is_key_pressed(KeyCode::KeyA as u32),
                key_s:     input.is_key_pressed(KeyCode::KeyS as u32),
                key_d:     input.is_key_pressed(KeyCode::KeyD as u32),
                key_space: input.is_key_pressed(KeyCode::Space as u32),
                key_up:    input.is_key_pressed(KeyCode::ArrowUp as u32),
                key_down:  input.is_key_pressed(KeyCode::ArrowDown as u32),
                key_left:  input.is_key_pressed(KeyCode::ArrowLeft as u32),
                key_right: input.is_key_pressed(KeyCode::ArrowRight as u32),
            };
            if let Some(engine) = engine_opt.as_mut() {
                let _ = engine.reload_if_changed(&script.file_path);
                let func_name = {
                    // Convention-over-configuration: dosya adından entry fonksiyon ismi türet
                    // örn. "scripts/car_controller.lua" → "car_controller_update"
                    let stem = std::path::Path::new(&script.file_path)
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("on");
                    let auto_name = format!("{}_update", stem);
                    // Fonksiyon script'te varsa onu kullan, yoksa fallback "on_update"
                    if engine.has_function(&auto_name) {
                        auto_name
                    } else {
                        "on_update".to_string()
                    }
                };
                
                match engine.run_entity_update(&func_name, &ctx) {
                    Ok(res) => {
                        if let Some(pos) = res.new_position { t.position = Vec3::new(pos[0], pos[1], pos[2]); }
                        if let Some(vel) = res.new_velocity  { v.linear   = Vec3::new(vel[0], vel[1], vel[2]); }
                    }
                    Err(err) => {
                        println!("[Lua Runtime Error] {}: {}", func_name, err);
                    }
                }
            }
        }
    }
    if let Some(engine) = engine_opt {
        unhandled = engine.flush_commands(world);
        world.insert_resource(engine);
    }
    unhandled
}

/// Lua command queue'daki oyun komutlarını GameState'e uygular
fn process_game_commands(world: &mut World, state: &mut GameState, dt: f32, commands: Vec<gizmo::scripting::commands::ScriptCommand>) {
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
        if let Some(transforms) = world.borrow::<Transform>() {
            if let Some(t) = transforms.get(target_id) {
                target_pos = Some(t.position);
            }
        }
        if let Some(tpos) = target_pos {
            if let Some(mut transforms) = world.borrow_mut::<Transform>() {
                if let Some(cam_t) = transforms.get_mut(state.player_id) {
                    // Chase kamera: hedefin arkasına + yukarıya yerleş
                    let offset = Vec3::new(0.0, 4.0, 10.0);
                    cam_t.position = cam_t.position.lerp(tpos + offset, dt * 5.0);
                }
            }
        }
    }

    // Checkpoint temas kontrolü
    {
        let mut player_pos = None;
        if let Some(transforms) = world.borrow::<Transform>() {
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
            // Tüm checkpoint'ler geçildiyse yarış bitti
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
            ScriptCommand::ShowDialogue { speaker, text, duration } => {
                state.active_dialogue = Some(crate::state::DialogueEntry { speaker, text, timer: duration });
            }
            ScriptCommand::HideDialogue => {
                state.active_dialogue = None;
            }
            ScriptCommand::TriggerCutscene(name) => {
                state.active_cutscene = Some(name.clone());
                state.free_cam = false; // cutscene sırasında kamera kilitle
                println!("[Cutscene] Başladı: {}", name);
            }
            ScriptCommand::EndCutscene => {
                state.active_cutscene = None;
                state.free_cam = true;
                println!("[Cutscene] Bitti.");
            }
            ScriptCommand::AddCheckpoint { id, position, radius } => {
                state.checkpoints.push(crate::state::Checkpoint { id, position, radius, activated: false });
                println!("[Race] Checkpoint {} eklendi ({:.1}, {:.1}, {:.1})", id, position.x, position.y, position.z);
            }
            ScriptCommand::ActivateCheckpoint(id) => {
                if let Some(cp) = state.checkpoints.iter_mut().find(|c| c.id == id) {
                    cp.activated = true;
                }
            }
            ScriptCommand::StartRace => {
                state.race_status = crate::state::RaceStatus::Running;
            }
            ScriptCommand::FinishRace { winner_name } => {
                state.race_status = crate::state::RaceStatus::Finished;
                println!("[Race] Kazanan: {} | Süre: {:.2}s", winner_name, state.race_timer);
            }
            ScriptCommand::ResetRace => {
                for cp in &mut state.checkpoints { cp.activated = false; }
                state.race_timer = 0.0;
                state.race_status = crate::state::RaceStatus::Idle;
            }
            ScriptCommand::SetCameraTarget(entity_id) => {
                state.camera_follow_target = Some(entity_id);
                state.free_cam = false;
            }
            ScriptCommand::SetCameraFov(fov) => {
                if let Some(mut cameras) = world.borrow_mut::<Camera>() {
                    if let Some(cam) = cameras.get_mut(state.player_id) {
                        cam.fov = fov.to_radians();
                    }
                }
            }
            _ => {} // diğer komutlar flush_commands'ta zaten işlendi
        }
    }
}
