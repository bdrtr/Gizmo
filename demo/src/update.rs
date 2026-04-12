use crate::gizmo_input;
use crate::state::GameState;
use gizmo::core::input::mouse;
use gizmo::prelude::*;
use gizmo::winit::keyboard::KeyCode;

pub fn update_demo(world: &mut World, state: &mut GameState, dt: f32, input: &Input) {
    let active_camera = state
        .basic_scene
        .as_ref()
        .map(|s| s.camera_entity)
        .or_else(|| state.ps1_race.as_ref().map(|r| r.camera_entity))
        .unwrap_or(state.player_id);

    world.insert_resource(crate::state::EngineConfig {
        free_cam: state.free_cam,
        active_camera_entity: active_camera,
        show_devtools: state.show_devtools,
    });

    state.current_fps = 1.0 / dt;

    // Hot-reload texture dosya takibi
    crate::hot_reload_sys::poll_hot_reload(world, state);

    // Seçim isteği uygula
    if let Some(mut events) =
        world.get_resource_mut::<gizmo::core::event::Events<crate::state::SelectionEvent>>()
    {
        for ev in events.drain() {
            state.inspector_selected_entity = Some(ev.entity_id);
        }
    }

    // Mouse tıklaması → raycast bayrağı
    if input.is_mouse_button_just_pressed(mouse::LEFT) {
        state.do_raycast = true;
    }
    if input.is_mouse_button_just_released(mouse::LEFT) {
        state.dragging_axis = None;
    }

    if input.is_key_just_pressed(KeyCode::F3 as u32) {
        state.show_devtools = !state.show_devtools;
    }

    // --- PAUSE MENU (ESC) KONTROLÜ ---
    if input.is_key_just_pressed(KeyCode::Escape as u32) {
        if let Some(mut m) = world.get_resource_mut::<crate::state::AppMode>() {
            if *m == crate::state::AppMode::InGame {
                *m = crate::state::AppMode::PauseMenu;
            } else if *m == crate::state::AppMode::PauseMenu {
                *m = crate::state::AppMode::InGame;
            }
        }
    }

    let is_in_game = world.get_resource::<crate::state::AppMode>().map(|m| *m)
        == Some(crate::state::AppMode::InGame);

    // --- OYUN DURAKLATMA MANTIĞI ---
    // Sadece InGame modunda kamerayı hareket ettir, raycast at ve hesaplama yap.
    if is_in_game {
        // Ray hesapla
        let (mx, my) = input.mouse_position();
        let (ww, wh) = input.window_size();
        let ndc_x = (2.0 * mx) / ww - 1.0;
        let ndc_y = 1.0 - (2.0 * my) / wh;
        let current_ray = gizmo_input::build_ray(world, state.player_id, ndc_x, ndc_y, ww, wh);

        // Gizmo Input (raycast + drag)
        if let Some(ray) = current_ray {
            let do_rc = state.do_raycast && !state.egui_wants_pointer;
            if do_rc {
                state.do_raycast = false;
            }
            if state.show_devtools {
                crate::gizmo_input::handle_gizmo_input(world, state, ray, do_rc);
            }
        }

        // Gizmo görsel senkron
        gizmo_input::sync_gizmos(world, state);

        // Zaman kaynağı
        state.total_elapsed += dt as f64;
        world.insert_resource(Time {
            dt,
            elapsed_seconds: state.total_elapsed,
        });

        // Fizik (sabit adım)
        state.physics_accumulator += dt;
        let fixed_dt = 1.0 / state.target_physics_fps;
        // Death spiral önleme: accumulator'ı max 16 adımla sınırla
        state.physics_accumulator = state.physics_accumulator.min(fixed_dt * 16.0);
        let mut steps = 0;
        while state.physics_accumulator >= fixed_dt && steps < 16 {
            gizmo::physics::integration::physics_apply_forces_system(world, fixed_dt);
            gizmo::physics::vehicle::physics_vehicle_system(world, fixed_dt);
            gizmo::physics::system::physics_collision_system(world, fixed_dt);
            gizmo::physics::character::physics_character_system(world, fixed_dt);
            gizmo::physics::race_ai_system(world, fixed_dt);

            // AI Navigasyon sistemi
            gizmo_ai::ai_navigation_system(world, fixed_dt);

            gizmo::physics::integration::physics_movement_system(world, fixed_dt);
            state.physics_accumulator -= fixed_dt;
            steps += 1;
        }

        // GIZMO CITY DASH OYUN MANTIĞI
        crate::systems::gizmo_city_dash_system(world, state, dt);
    } else {
        // Oyun dışındaysa (Menü/Pause) physics_accumulator'ı boşalt (birikmesin)
        state.physics_accumulator = 0.0;
    }

    crate::systems::transform_hierarchy_system(world);

    // AI Hedef Güncelleme (Oyuncuyu Takip Et)
    if let Some(mut agents) = world.borrow_mut::<gizmo_ai::NavAgent>() {
        let player_pos =
            if let Some(transforms) = world.borrow::<gizmo::physics::components::Transform>() {
                transforms.get(state.player_id).map(|t| t.position)
            } else {
                None
            };

        if let Some(ppos) = player_pos {
            let keys = agents.dense.iter().map(|e| e.entity).collect::<Vec<_>>();
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

    // AudioManager Memory/GC Temizliği: biten ses bağlantılarını temizle
    if let Some(ref mut audio) = state.audio {
        audio.clean_dead_sinks();
    }

    // Script bileşeni olan entity'ler için per-entity script çalıştır
    let cmds = crate::script_sys::run_scripts(world, state, dt, input);

    // Lua'dan gelen oyun komutlarını işle
    crate::script_sys::process_game_commands(world, state, dt, cmds);

    if let Some(ref mut race) = state.ps1_race {
        crate::race::update_race(world, race, dt);

        // CHASE CAM UPDATE
        let (mut p_pos, mut p_forward) = (gizmo::math::Vec3::ZERO, gizmo::math::Vec3::ZERO);
        let mut cam_pos = gizmo::math::Vec3::ZERO;

        if let Some(trans) = world.borrow::<gizmo::physics::components::Transform>() {
            if let Some(player_t) = trans.get(race.player_entity) {
                p_pos = player_t.position;
                p_forward = player_t.rotation * gizmo::math::Vec3::new(0.0, 0.0, 1.0);
            }
            if let Some(cam_t) = trans.get(race.camera_entity) {
                cam_pos = cam_t.position;
            }
        }

        if p_pos != gizmo::math::Vec3::ZERO && cam_pos != gizmo::math::Vec3::ZERO {
            let target_cam_pos = p_pos - p_forward * 6.0 + gizmo::math::Vec3::new(0.0, 3.0, 0.0);
            let new_cam_pos = cam_pos.lerp(target_cam_pos, 8.0 * dt);
            let dir = (p_pos - new_cam_pos).normalize();
            let new_yaw = dir.z.atan2(dir.x);
            let new_pitch = dir.y.asin();

            if let Some(mut trans) = world.borrow_mut::<gizmo::physics::components::Transform>() {
                if let Some(cam_t) = trans.get_mut(race.camera_entity) {
                    cam_t.position = new_cam_pos;
                }
            }
            if let Some(mut cameras) = world.borrow_mut::<gizmo::renderer::components::Camera>() {
                if let Some(cam) = cameras.get_mut(race.camera_entity) {
                    cam.yaw = new_yaw;
                    cam.pitch = new_pitch;
                }
            }
        }
    }

    // CHASE CAM UPDATE FOR BASIC SCENE
    // Serbest kamera (free cam) kapalıysa arabayı takip et
    if !state.free_cam {
        if let Some(ref basic) = state.basic_scene {
            let (mut p_pos, mut p_forward) = (gizmo::math::Vec3::ZERO, gizmo::math::Vec3::ZERO);
            let mut cam_pos = gizmo::math::Vec3::ZERO;

            if let Some(trans) = world.borrow::<gizmo::physics::components::Transform>() {
                if let Some(player_t) = trans.get(basic.player_entity) {
                    p_pos = player_t.position;
                    p_forward = player_t.rotation * gizmo::math::Vec3::new(0.0, 0.0, 1.0);
                }
                if let Some(cam_t) = trans.get(basic.camera_entity) {
                    cam_pos = cam_t.position;
                }
            }

            if p_pos != gizmo::math::Vec3::ZERO && cam_pos != gizmo::math::Vec3::ZERO {
                // Kamerayı araca yaklaştıralım (Mesafe 5.5, Yükseklik 2.0)
                let target_cam_pos =
                    p_pos - p_forward * 5.5 + gizmo::math::Vec3::new(0.0, 2.0, 0.0);

                let new_cam_pos = cam_pos.lerp(target_cam_pos, 15.0 * dt);

                let dir = (p_pos - new_cam_pos).normalize();
                let new_yaw = dir.z.atan2(dir.x);
                let new_pitch = dir.y.asin();

                if let Some(mut trans) = world.borrow_mut::<gizmo::physics::components::Transform>()
                {
                    if let Some(cam_t) = trans.get_mut(basic.camera_entity) {
                        cam_t.position = new_cam_pos;
                    }
                }
                if let Some(mut cameras) = world.borrow_mut::<gizmo::renderer::components::Camera>()
                {
                    if let Some(cam) = cameras.get_mut(basic.camera_entity) {
                        cam.yaw = new_yaw;
                        cam.pitch = new_pitch;
                    }
                }
            }
        }
    }
}
