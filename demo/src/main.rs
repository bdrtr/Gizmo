use gizmo::prelude::*;

pub mod scene;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct EntityName(pub String);

pub mod state;       pub use state::*;
pub mod scene_setup; pub use scene_setup::*;
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
        setup_default_scene(world, renderer)
    });

    // ── UPDATE ─────────────────────────────────────────────────────────────
    app = app.set_update(|world, state, dt, input| {
        state.current_fps = 1.0 / dt;

        // Hot-reload texture dosya takibi
        poll_hot_reload(world, state);

        // Seçim isteği uygula
        if let Some(new_sel) = state.new_selection_request.get() {
            state.inspector_selected_entity = Some(new_sel);
            state.new_selection_request.set(None);
        }

        // Mouse tıklaması → raycast bayrağı
        if input.is_mouse_button_just_pressed(mouse::LEFT) { state.do_raycast = true; }
        if input.is_mouse_button_just_released(mouse::LEFT) { state.dragging_axis = None; }

        // Kamera fare ile döndür (Serbest kamera modunda)
        if state.free_cam && input.is_mouse_button_pressed(mouse::RIGHT) {
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
        if state.free_cam {
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
        world.insert_resource(Time { dt, elapsed_seconds: 0.0 });

        // Fizik (sabit adım)
        state.physics_accumulator += dt;
        let fixed_dt = 1.0 / state.target_physics_fps;
        let mut steps = 0;
        while state.physics_accumulator >= fixed_dt && steps < 16 {
            gizmo::physics::system::physics_collision_system(world, 1.0 / 60.0);
            gizmo::physics::character::physics_character_system(world, fixed_dt);
            if let Some(jw) = world.get_resource::<gizmo::physics::JointWorld>() {
                gizmo::physics::solve_constraints(&*jw, world, fixed_dt);
            }
            state.physics_accumulator -= fixed_dt;
            steps += 1;
        }

        transform_hierarchy_system(world);

        // Lua Script motoru
        run_scripts(world, state, dt, input);
    });

    // ── UI ─────────────────────────────────────────────────────────────────
    app = app.set_ui(|world, state, ctx| {
        state.egui_wants_pointer = ctx.is_pointer_over_area();
        render_ui(ctx, state, world);
    });

    app = app.set_render(|world, state, encoder, view, renderer: &mut gizmo::renderer::Renderer, light_time| {
        // Post-process ayarlarını uygula
        {
            let pp = *state.post_process_settings.borrow();
            renderer.update_post_process(&renderer.queue, pp);
        }
        
        // Shader reload isteği
        if state.shader_reload_request.get() {
            renderer.rebuild_shaders();
            state.shader_reload_request.set(false);
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

fn run_scripts(world: &mut World, state: &mut GameState, dt: f32, input: &Input) {
    if state.script_engine.borrow().is_none() { return; }

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
            };
            if let Some(engine) = state.script_engine.borrow_mut().as_mut() {
                let _ = engine.reload_if_changed(&script.file_path);
                if let Ok(res) = engine.run_update(&ctx) {
                    if let Some(pos) = res.new_position { t.position = Vec3::new(pos[0], pos[1], pos[2]); }
                    if let Some(vel) = res.new_velocity  { v.linear   = Vec3::new(vel[0], vel[1], vel[2]); }
                }
            }
        }
    }
    if let Some(engine) = state.script_engine.borrow_mut().as_mut() {
        engine.flush_commands(world);
    }
}
