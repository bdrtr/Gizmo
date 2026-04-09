use gizmo::prelude::*;
use gizmo::math::{Vec3, Vec4, Quat};

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
        // .load_scene("scene1.json") // Sahneleri otomatik yüklemek için bu satırı açabilirsiniz
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
        state.player_id = basic_state.camera_entity;
        state.basic_scene = Some(basic_state);
        state.free_cam = true; // Oyuncunun kamerayı WASD ile gezdirebilmesi için aktif edildi.
        
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

        let _is_in_game = world.get_resource::<crate::state::AppMode>().map(|m| *m) == Some(crate::state::AppMode::InGame);

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
            if state.show_devtools {
                handle_gizmo_input(world, state, ray, do_rc);
            }
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
            gizmo::physics::race_ai_system(world, fixed_dt);
            gizmo::physics::vehicle::physics_vehicle_system(world, fixed_dt);
            
            // AI Navigasyon sistemi
            gizmo_ai::ai_navigation_system(world, fixed_dt);
            
            gizmo::physics::integration::physics_movement_system(world, fixed_dt);
            state.physics_accumulator -= fixed_dt;
            steps += 1;
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

        // SCENE VS GAME VIEW TAB ODAK KONTROLÜ
        if let Some(editor) = world.get_resource::<gizmo::editor::EditorState>() {
            state.free_cam = true; // Hep serbest kamera
        }

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
        // EDITOR SCENE SAVE/LOAD İşlemleri
        let mut save_req: Option<String> = None;
        let mut load_req: Option<String> = None;
        let mut spawn_req: Option<String> = None;
        let mut spawn_asset_req: Option<String> = None;
        let mut spawn_asset_pos: Option<gizmo::math::Vec3> = None;
        let mut despawn_req: Option<u32> = None;
        let mut reparent_req: Option<(u32, u32)> = None;
        let mut unparent_req: Option<u32> = None;
        let mut toggle_vis_req: Option<u32> = None;
        let mut add_comp_req: Option<(u32, String)> = None;
        
        if let Some(mut editor) = world.get_resource_mut::<gizmo::editor::EditorState>() {
            save_req = editor.scene_save_request.take();
            load_req = editor.scene_load_request.take();
            spawn_req = editor.spawn_request.take();
            spawn_asset_req = editor.spawn_asset_request.take();
            spawn_asset_pos = editor.spawn_asset_position.take();
            despawn_req = editor.despawn_request.take();
            reparent_req = editor.reparent_request.take();
            unparent_req = editor.unparent_request.take();
            toggle_vis_req = editor.toggle_visibility_request.take();
            add_comp_req = editor.add_component_request.take();
        }

        // TOGGLE VISIBILITY
        if let Some(id) = toggle_vis_req {
            if let Some(entity) = world.get_entity(id) {
                let mut is_hidden = false;
                if let Some(mut hidden_comp) = world.borrow_mut::<gizmo::core::component::IsHidden>() {
                    if hidden_comp.contains(id) {
                        hidden_comp.remove(id);
                    } else {
                        is_hidden = true;
                    }
                }
                if is_hidden {
                    world.add_component(entity, gizmo::core::component::IsHidden);
                }
            }
        }

        // DESPAWN
        if let Some(id) = despawn_req {
            world.despawn_by_id(id);
            if let Some(mut editor) = world.get_resource_mut::<gizmo::editor::EditorState>() {
                let msg = format!("Silindi: {}", id);
                editor.status_message = msg.clone();
                editor.log_warning(&msg);
                editor.selected_entity = None;
            }
        }

        // ADD COMPONENT
        if let Some((ent_id, comp_name)) = add_comp_req {
            if let Some(entity) = world.get_entity(ent_id) {
                match comp_name.as_str() {
                    "Transform" => world.add_component(entity, gizmo::physics::components::Transform::new(gizmo::math::Vec3::ZERO)),
                    "Velocity" => world.add_component(entity, gizmo::physics::components::Velocity::new(gizmo::math::Vec3::ZERO)),
                    "RigidBody" => world.add_component(entity, gizmo::physics::components::RigidBody::new(1.0, 0.5, 0.5, true)),
                    "Collider" => world.add_component(entity, gizmo::physics::shape::Collider::new_sphere(1.0)),
                    "Camera" => world.add_component(entity, gizmo::renderer::components::Camera::new(
                        std::f32::consts::FRAC_PI_4,
                        0.1,
                        1000.0,
                        0.0,
                        0.0,
                        true,
                    )),
                    "PointLight" => {
                        let light = gizmo::renderer::components::PointLight::new(gizmo::math::Vec3::new(1.0, 1.0, 1.0), 10.0);
                        world.add_component(entity, light);
                    },
                    "Material" => {
                        // Basit white material
                        if let Some(mut asset_manager) = world.remove_resource::<gizmo::renderer::asset::AssetManager>() {
                            let base_tbind = asset_manager.create_white_texture(&renderer.device, &renderer.queue, &renderer.scene.texture_bind_group_layout);
                            let mat = gizmo::renderer::components::Material::new(base_tbind).with_pbr(gizmo::math::Vec4::new(1.0, 1.0, 1.0, 1.0), 0.5, 0.5);
                            world.add_component(entity, mat);
                            world.insert_resource(asset_manager);
                        }
                    },
                    _ => {}
                }

                if let Some(mut editor) = world.get_resource_mut::<gizmo::editor::EditorState>() {
                    editor.log_info(&format!("Bileşen eklendi: {} (Entity {})", comp_name, ent_id));
                }
            }
        }

        // SPAWN
        if let Some(spawn_type) = spawn_req {
            let ent = world.spawn();
            world.add_component(ent, gizmo::prelude::Transform::new(gizmo::prelude::Vec3::new(0.0, 0.0, 0.0)));
            
            let name = match spawn_type.as_str() {
                "Cube" => {
                    let mesh = gizmo::renderer::asset::AssetManager::create_cube(&renderer.device);
                    world.add_component(ent, mesh);
                    world.add_component(ent, gizmo::renderer::components::MeshRenderer::new());
                    "Küp"
                },
                "Sphere" => {
                    let mesh = gizmo::renderer::asset::AssetManager::create_sphere(&renderer.device, 1.0, 16, 16);
                    world.add_component(ent, mesh);
                    world.add_component(ent, gizmo::renderer::components::MeshRenderer::new());
                    "Küre"
                },
                _ => "Boş Entity"
            };
            
            world.add_component(ent, gizmo::prelude::EntityName(name.to_string()));
            
            if let Some(mut editor) = world.get_resource_mut::<gizmo::editor::EditorState>() {
                let msg = format!("Oluşturuldu: {} ({})", name, ent.id());
                editor.status_message = msg.clone();
                editor.log_info(&msg);
                editor.selected_entity = Some(ent.id());
            }
        }

        // ASSET SPAWN (GLB/GLTF)
        if let Some(path) = spawn_asset_req {
            let mut status = "Desteklenmeyen dosya türü!".to_string();
            let mut new_ent_id = None;
            
            if let Some(mut asset_manager) = world.remove_resource::<gizmo::renderer::asset::AssetManager>() {
                let ext = path.rsplit('.').next().unwrap_or("").to_lowercase();
                
                if ext == "glb" || ext == "gltf" {
                    let base_tbind = asset_manager.create_white_texture(&renderer.device, &renderer.queue, &renderer.scene.texture_bind_group_layout);
                    let def_mat = gizmo::renderer::components::Material::new(base_tbind.clone()).with_pbr(gizmo::math::Vec4::new(1.0, 1.0, 1.0, 1.0), 0.5, 0.5);
                    
                    match asset_manager.load_gltf_scene(&renderer.device, &renderer.queue, &renderer.scene.texture_bind_group_layout, base_tbind, &path) {
                        Ok(asset) => {
                            // Kameradan 10 birim ilerisini bul (Eğer Scene View Sürükle ve Bırak ise)
                            let mut drop_pos = gizmo::math::Vec3::ZERO;
                            if spawn_asset_pos.is_some() {
                                // Kameranın baktığı yönü bulalım
                                if let (Some(cameras), Some(transforms)) = (world.borrow::<gizmo::renderer::components::Camera>(), world.borrow::<gizmo::physics::components::Transform>()) {
                                    if let (Some(cam), Some(t)) = (cameras.get(state.player_id), transforms.get(state.player_id)) {
                                        drop_pos = t.position + cam.get_front() * 10.0;
                                    }
                                }
                            }
                        
                            let root_transform = gizmo::physics::components::Transform::new(drop_pos);
                            let root_entity = crate::scene_setup::spawn_gltf_asset(world, &asset, renderer, def_mat, root_transform);
                            
                            // Adını dosya adı yap
                            let file_name = std::path::Path::new(&path).file_name().unwrap_or_else(|| std::ffi::OsStr::new("Asset")).to_string_lossy().to_string();
                            world.add_component(root_entity, gizmo::prelude::EntityName(file_name));
                            
                            status = format!("Model Yüklendi: {}", path);
                            new_ent_id = Some(root_entity.id());
                        }
                        Err(e) => {
                            status = format!("Model yüklenemedi: {:?}", e);
                        }
                    }
                }
                
                world.insert_resource(asset_manager);
            } else {
                status = "Hata: AssetManager bulunamadı!".to_string();
            }
            
            if let Some(mut editor) = world.get_resource_mut::<gizmo::editor::EditorState>() {
                editor.status_message = status.clone();
                if new_ent_id.is_some() {
                    editor.log_info(&status);
                    editor.selected_entity = new_ent_id;
                } else {
                    editor.log_error(&status);
                }
            }
        }

        // REPARENT
        if let Some((child_id, new_parent_id)) = reparent_req {
            let mut old_parent_id = None;
            
            // Çocuğun eski Parent'ı kim?
            if let Some(parents) = world.borrow::<gizmo::core::component::Parent>() {
                if let Some(p) = parents.get(child_id) {
                    old_parent_id = Some(p.0);
                }
            }

            // Çocuğa yeni ebeveyni kaydet
            world.add_component(gizmo::core::entity::Entity::new(child_id, 0), gizmo::core::component::Parent(new_parent_id));

            // Yeni ebeveynin Children dizisine çocuğu ekle
            let mut target_had_children = false;
            if let Some(mut children_comp) = world.borrow_mut::<gizmo::core::component::Children>() {
                if let Some(c) = children_comp.get_mut(new_parent_id) {
                    if !c.0.contains(&child_id) {
                        c.0.push(child_id);
                    }
                    target_had_children = true;
                }
            }
            if !target_had_children {
                world.add_component(gizmo::core::entity::Entity::new(new_parent_id, 0), gizmo::core::component::Children(vec![child_id]));
            }

            // Eski ebeveynin Children dizisinden çocuğu sil
            if let Some(old_pid) = old_parent_id {
                if let Some(mut children_comp) = world.borrow_mut::<gizmo::core::component::Children>() {
                    if let Some(c) = children_comp.get_mut(old_pid) {
                        c.0.retain(|&id| id != child_id);
                    }
                }
            }

            if let Some(mut editor) = world.get_resource_mut::<gizmo::editor::EditorState>() {
                editor.status_message = format!("Bağlandı: {} -> Parent {}", child_id, new_parent_id);
            }
        }

        // UNPARENT (Kök yap)
        if let Some(child_id) = unparent_req {
            let mut old_parent_id = None;
            if let Some(parents) = world.borrow::<gizmo::core::component::Parent>() {
                if let Some(p) = parents.get(child_id) {
                    old_parent_id = Some(p.0);
                }
            }
            if let Some(old_pid) = old_parent_id {
                if let Some(mut children_comp) = world.borrow_mut::<gizmo::core::component::Children>() {
                    if let Some(c) = children_comp.get_mut(old_pid) {
                        c.0.retain(|&id| id != child_id);
                    }
                }
                // Component'i silmek zor, yerine id'sini parent=0 veya root için bir yapı koymak gerekiyorsa koyarız.
                // Motorumuzda kök entitylerde Parent componenti olması opsiyonel.
            }
            if let Some(mut editor) = world.get_resource_mut::<gizmo::editor::EditorState>() {
                editor.status_message = format!("Kök yapıldı: {}", child_id);
            }
        }

        if let Some(save_path) = save_req {
            gizmo::scene::SceneData::save(world, &save_path);
            if let Some(mut editor) = world.get_resource_mut::<gizmo::editor::EditorState>() {
                editor.status_message = format!("✅ Kaydedildi: {}", save_path);
            }
        }

        if let Some(load_path) = load_req {
            let mut status = String::new();
            if let Some(mut asset_manager) = world.remove_resource::<gizmo::renderer::asset::AssetManager>() {
                let dummy_rgba = [255, 255, 255, 255];
                let dummy_bg = renderer.create_texture(&dummy_rgba, 1, 1);
                gizmo::scene::SceneData::load_into(
                    &load_path,
                    world,
                    &renderer.device,
                    &renderer.queue,
                    &renderer.scene.texture_bind_group_layout,
                    &mut asset_manager,
                    std::sync::Arc::new(dummy_bg)
                );
                world.insert_resource(asset_manager);
                status = format!("✅ Yüklendi: {}", load_path);
            } else {
                status = "❌ Hata: AssetManager bulunamadı!".to_string();
            }
            if let Some(mut editor) = world.get_resource_mut::<gizmo::editor::EditorState>() {
                editor.status_message = status;
            }
        }

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
                    // Fonksiyon script'te varsa onu kullan, yoksa bu entity'yi atla
                    // NOT: Global "on_update" zaten engine.update() tarafından çağrılıyor,
                    // burada tekrar çağırmak çift çalıştırma (double-call) sorununa yol açar.
                    if engine.has_function(&auto_name) {
                        auto_name
                    } else {
                        continue; // entity-specific fonksiyon yok → atla
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
