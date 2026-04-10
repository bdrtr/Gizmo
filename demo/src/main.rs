use gizmo::prelude::*;
use gizmo::math::{Vec3, Quat};

#[derive(Clone, Copy)]
pub struct OriginalRotation(pub Quat);

#[derive(Clone, Copy)]
pub struct Player;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct EntityName(pub String);

pub mod state;       pub use state::*;
pub mod scene_setup; pub use scene_setup::*;
pub mod demo_car;    pub use demo_car::*;
pub mod ui;          pub use ui::*;
pub mod script_sys;  pub use script_sys::*;
pub mod render_pipeline; pub use render_pipeline::*;
pub mod systems;     pub use systems::*;
pub mod gizmo_input; pub use gizmo_input::*;
pub mod camera;      pub use camera::*;
pub mod hot_reload_sys; pub use hot_reload_sys::*;
pub mod components;
pub mod race;
pub mod basic_scene;
pub mod network;

fn main() {
    // Demo -> Sadece Voxygen (Client) Renderer olarak görev yapar!
    let mut app = App::<crate::state::GameState>::new("Gizmo Engine — Rust 3D Motor", 1280, 720)
        .add_system(crate::systems::vehicle_controller_system)
        .add_system(crate::systems::character_update_system)
        .add_system(crate::systems::free_camera_system)
        .add_system(crate::systems::chase_camera_system)
        .add_system(crate::systems::ccd_test_system)
        .add_system(crate::network::client_network_system)
        // .load_scene("scene1.gizmo") // Sahneleri otomatik yüklemek için bu satırı açabilirsiniz
        ;

    app = app.set_setup(|world, renderer| {
        use gizmo::core::input::ActionMap;
        use gizmo::winit::keyboard::KeyCode;

        let mut action_map = ActionMap::new();
        action_map.bind_action("Accelerate", KeyCode::ArrowUp as u32);
        action_map.bind_action("Reverse", KeyCode::ArrowDown as u32);
        action_map.bind_action("SteerLeft", KeyCode::ArrowLeft as u32);
        action_map.bind_action("SteerRight", KeyCode::ArrowRight as u32);
        action_map.bind_action("Brake", KeyCode::Space as u32);
        world.insert_resource(action_map);

        let mut state = scene_setup::setup_empty_scene(world, renderer);
        
        // Yarış yerine Basic Scene modunu başlat
        let basic_state = crate::basic_scene::setup_basic_scene(world, renderer);
        state.player_id = basic_state.player_entity; 
        state.camera_follow_target = Some(basic_state.player_entity);
        state.basic_scene = Some(basic_state);
        state.free_cam = false;
        
        // Gizmo-Net Client Initialize (Şimdilik devre dışı - sunucu yoksa çökmeyi önlemek için)
        // world.insert_resource(gizmo_net::client::NetworkClient::new("127.0.0.1:4000"));

        state
    });

    // ── UPDATE ─────────────────────────────────────────────────────────────
    app = app.set_update(|world, state, dt, input| {
        let active_camera = state.basic_scene.as_ref().map(|s| s.camera_entity)
            .or_else(|| state.ps1_race.as_ref().map(|r| r.camera_entity))
            .unwrap_or(state.player_id);

        world.insert_resource(crate::state::EngineConfig {
            free_cam: state.free_cam,
            active_camera_entity: active_camera,
            show_devtools: state.show_devtools,
        });

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

        let is_in_game = world.get_resource::<crate::state::AppMode>().map(|m| *m) == Some(crate::state::AppMode::InGame);

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
                if do_rc { state.do_raycast = false; }
                if state.show_devtools {
                    handle_gizmo_input(world, state, ray, do_rc);
                }
            }

            // Gizmo görsel senkron
            gizmo_input::sync_gizmos(world, state);

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
                gizmo::physics::race_ai_system(world, fixed_dt);
                gizmo::physics::vehicle::physics_vehicle_system(world, fixed_dt);
                
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

        // Script bileşeni olan entity'ler için per-entity script çalıştır
        let cmds = crate::script_sys::run_scripts(world, state, dt, input);

        // Lua'dan gelen oyun komutlarını işle
        crate::script_sys::process_game_commands(world, state, dt, cmds);

        if let Some(ref mut race) = state.ps1_race {
            crate::race::update_race(world, race, dt);
            
            // CHASE CAM UPDATE
            let (mut p_pos, mut p_forward) = (Vec3::ZERO, Vec3::ZERO);
            let mut cam_pos = Vec3::ZERO;
            
            if let Some(trans) = world.borrow::<Transform>() {
                if let Some(player_t) = trans.get(race.player_entity) {
                    p_pos = player_t.position;
                    p_forward = player_t.rotation * Vec3::new(0.0, 0.0, 1.0);
                }
                if let Some(cam_t) = trans.get(race.camera_entity) {
                    cam_pos = cam_t.position;
                }
            }
            
            if p_pos != Vec3::ZERO && cam_pos != Vec3::ZERO {
                let target_cam_pos = p_pos - p_forward * 6.0 + Vec3::new(0.0, 3.0, 0.0);
                let new_cam_pos = cam_pos.lerp(target_cam_pos, 8.0 * dt);
                let dir = (p_pos - new_cam_pos).normalize();
                let new_yaw = dir.z.atan2(dir.x);
                let new_pitch = dir.y.asin();
                
                if let Some(mut trans) = world.borrow_mut::<Transform>() {
                    if let Some(cam_t) = trans.get_mut(race.camera_entity) {
                        cam_t.position = new_cam_pos;
                    }
                }
                if let Some(mut cameras) = world.borrow_mut::<Camera>() {
                    if let Some(cam) = cameras.get_mut(race.camera_entity) {
                        cam.yaw = new_yaw;
                        cam.pitch = new_pitch;
                    }
                }
            }
        }

        // Serbest kamera kapalıysa CHASE CAM çalışır
        // CHASE CAM UPDATE FOR BASIC SCENE
        // Serbest kamera (free cam) kapalıysa arabayı takip et
        if !state.free_cam {
            if let Some(ref basic) = state.basic_scene {
                let (mut p_pos, mut p_forward) = (Vec3::ZERO, Vec3::ZERO);
                let mut cam_pos = Vec3::ZERO;
            
            if let Some(trans) = world.borrow::<Transform>() {
                if let Some(player_t) = trans.get(basic.player_entity) {
                    p_pos = player_t.position;
                    p_forward = player_t.rotation * Vec3::new(0.0, 0.0, 1.0);
                }
                if let Some(cam_t) = trans.get(basic.camera_entity) {
                    cam_pos = cam_t.position;
                }
            }
            
            if p_pos != Vec3::ZERO && cam_pos != Vec3::ZERO {
                // Kamerayı araca yaklaştıralım (Mesafe 5.5, Yükseklik 2.0)
                let target_cam_pos = p_pos - p_forward * 5.5 + Vec3::new(0.0, 2.0, 0.0);
                
                let new_cam_pos = cam_pos.lerp(target_cam_pos, 15.0 * dt);
                
                let dir = (p_pos - new_cam_pos).normalize();
                let new_yaw = dir.z.atan2(dir.x);
                let new_pitch = dir.y.asin();
                
                if let Some(mut trans) = world.borrow_mut::<Transform>() {
                    if let Some(cam_t) = trans.get_mut(basic.camera_entity) {
                        cam_t.position = new_cam_pos;
                    }
                }
                if let Some(mut cameras) = world.borrow_mut::<Camera>() {
                    if let Some(cam) = cameras.get_mut(basic.camera_entity) {
                        cam.yaw = new_yaw;
                        cam.pitch = new_pitch;
                    }
                }
            }
        }
        }
    });

    // ── UI ─────────────────────────────────────────────────────────────────
    app = app.set_ui(|world, state, ctx| {
        state.egui_wants_pointer = ctx.is_pointer_over_area();
        render_ui(ctx, state, world);
    });

    app = app.set_render(|world: &mut World, state: &crate::state::GameState, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, renderer: &mut gizmo::renderer::Renderer, light_time: f32| {

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

// (Bu kısımda taşınan kodlar script_sys.rs ve gizmo_input.rs modüllerine dağıtıldı)

