use gizmo::app::App;
use gizmo::core::input::Input;
use gizmo::core::world::World;
use gizmo::math::{Quat, Vec3, Vec4};
use gizmo::physics::components::{Collider, GlobalTransform, RigidBody, Velocity, Vehicle, Wheel};
use gizmo::physics::world::PhysicsWorld;
use gizmo::physics::Transform;
use gizmo::prelude::*;
use gizmo::renderer::asset::AssetManager;
use gizmo::renderer::components::{Camera, DirectionalLight, Material, MeshRenderer};
use gizmo::egui;
#[derive(Debug, Clone, Copy)]
pub struct CarConfig {
    pub engine_power: f32,
    pub steer_speed: f32,
    pub steer_auto_return: f32,
    pub steer_max_angle: f32,
    pub steer_torque: f32,
    pub engine_brake: f32,
    pub base_grip: f32,
    pub slip_threshold: f32,
    pub drift_grip: f32,
    pub chassis_mass: f32,
    pub linear_damping: f32,
    pub angular_damping: f32,
    pub friction: f32,
    pub gravity_y: f32,
}

impl Default for CarConfig {
    fn default() -> Self {
        Self {
            engine_power: 3000.0, // Increased slightly since mass is higher
            steer_speed: 6.0,
            steer_auto_return: 15.0,
            steer_max_angle: 1.0,
            steer_torque: 1000.0,
            engine_brake: 2.5,
            base_grip: 8.0,
            slip_threshold: 6.0,
            drift_grip: 1.0,
            chassis_mass: 1200.0, // Standard car weight
            linear_damping: 0.9,
            angular_damping: 1.8,
            friction: 0.8,
            gravity_y: -20.0, // Stronger gravity to keep it grounded
        }
    }
}

struct CarDemoState {
    config: CarConfig,
    cam_yaw: f32,
    cam_pitch: f32,
    cam_pos: Vec3,
    chassis_id: u32,
    camera_entity_id: u32,
    ground_id: Option<u32>,
    weather_idx: usize,
    steer_angle: f32,
    wheel_spin: f32,
    wheel_fl: Option<u32>,
    wheel_fr: Option<u32>,
    wheel_bl: Option<u32>,
    wheel_br: Option<u32>,
    wheel_fl_rot: Quat,
    wheel_fr_rot: Quat,
    wheel_bl_rot: Quat,
    wheel_br_rot: Quat,
    skid_pool: Vec<u32>,
    skid_idx: usize,
    last_skid_time: f32,
    is_drifting: bool,
    show_physics_debug: bool,
}

impl CarDemoState {
    fn new(config: CarConfig) -> Self {
        Self {
            config,
            cam_yaw: -std::f32::consts::FRAC_PI_2,
            cam_pitch: -0.4,
            cam_pos: Vec3::new(0.0, 5.0, 15.0),
            chassis_id: 0,
            camera_entity_id: 0,
            ground_id: None,
            weather_idx: 0,
            steer_angle: 0.0,
            wheel_spin: 0.0,
            wheel_fl: None,
            wheel_fr: None,
            wheel_bl: None,
            wheel_br: None,
            wheel_fl_rot: Quat::IDENTITY,
            wheel_fr_rot: Quat::IDENTITY,
            wheel_bl_rot: Quat::IDENTITY,
            wheel_br_rot: Quat::IDENTITY,
            skid_pool: Vec::new(),
            skid_idx: 0,
            last_skid_time: 0.0,
            is_drifting: false,
            show_physics_debug: true,
        }
    }
}

/// Yardımcı: Entity'ye hem Transform hem de varsayılan GlobalTransform ekler.
fn add_transform(world: &mut World, entity: gizmo::core::Entity, t: Transform) {
    world.add_component(entity, t);
    world.add_component(entity, GlobalTransform { matrix: t.local_matrix });
}

fn main() {
    gizmo::app::setup_panic_hook();

    App::<CarDemoState>::new("Gizmo — Car Demo", 1600, 900)
        .add_plugin(gizmo::plugins::TransformPlugin)
        .set_setup(setup_scene)
        .set_update(|world, state, dt, input| {
            let mut throttle = 0.0;
            let mut brake = 0.0;
            if input.is_key_pressed(gizmo::prelude::KeyCode::ArrowUp as u32) || input.is_key_pressed(gizmo::prelude::KeyCode::KeyW as u32) {
                throttle += 1.0;
            }
            if input.is_key_pressed(gizmo::prelude::KeyCode::ArrowDown as u32) || input.is_key_pressed(gizmo::prelude::KeyCode::KeyS as u32) {
                throttle -= 1.0;
                if throttle < 0.0 {
                    brake = 1.0;
                    throttle = -0.5;
                }
            }

            let mut is_steering = false;
            if input.is_key_pressed(gizmo::prelude::KeyCode::ArrowLeft as u32) || input.is_key_pressed(gizmo::prelude::KeyCode::KeyA as u32) {
                state.steer_angle = (state.steer_angle + state.config.steer_speed * dt).min(state.config.steer_max_angle);
                is_steering = true;
            }
            if input.is_key_pressed(gizmo::prelude::KeyCode::ArrowRight as u32) || input.is_key_pressed(gizmo::prelude::KeyCode::KeyD as u32) {
                state.steer_angle = (state.steer_angle - state.config.steer_speed * dt).max(-state.config.steer_max_angle);
                is_steering = true;
            }
            if !is_steering {
                state.steer_angle *= (-state.config.steer_auto_return * dt).exp();
            }

            {
                let vehicle_store = world.borrow_mut::<Vehicle>();
                if let Some(mut vehicle) = vehicle_store.get_mut(state.chassis_id) {
                    vehicle.current_throttle = throttle;
                    vehicle.current_brake = brake;
                    vehicle.current_steer = state.steer_angle;
                    
                    // Sync from UI
                    vehicle.engine_power = state.config.engine_power;
                    vehicle.brake_force = state.config.engine_power * 0.8;
                    vehicle.downforce_coefficient = 1.5; // High downforce to prevent flying

                    if input.is_key_just_pressed(gizmo::prelude::KeyCode::KeyT as u32) {
                        vehicle.gearbox.is_automatic = !vehicle.gearbox.is_automatic;
                    }

                    if !vehicle.gearbox.is_automatic {
                        if input.is_key_just_pressed(gizmo::prelude::KeyCode::KeyE as u32)
                            && vehicle.gearbox.current_gear < vehicle.gearbox.gears.len() - 1 {
                                vehicle.gearbox.current_gear += 1;
                            }
                        if input.is_key_just_pressed(gizmo::prelude::KeyCode::KeyQ as u32)
                            && vehicle.gearbox.current_gear > 0 {
                                vehicle.gearbox.current_gear -= 1;
                            }
                    }
                }
            }
            
            // Reset Car Position and Velocity
            if input.is_key_just_pressed(gizmo::prelude::KeyCode::KeyR as u32) {
                if let Some(mut phys_world) = world.get_resource_mut::<PhysicsWorld>() {
                    if let Some(rb_idx) = phys_world.entities.iter().position(|e| e.id() == state.chassis_id) {
                        phys_world.transforms[rb_idx] = Transform::new(Vec3::new(0.0, 1.5, 0.0));
                        phys_world.velocities[rb_idx] = Velocity::default();
                    }
                }
                let transforms = world.borrow_mut::<Transform>();
                let velocities = world.borrow_mut::<Velocity>();
                if let Some(mut t) = transforms.get_mut(state.chassis_id) {
                    *t = Transform::new(Vec3::new(0.0, 1.5, 0.0));
                }
                if let Some(mut v) = velocities.get_mut(state.chassis_id) {
                    *v = Velocity::default();
                }
            }

            // Sabit Fizik Adımı (Fixed Timestep) - Titremeyi ve hayalet etkisini çözer
            let mut physics_dt = dt.min(0.1);
            while physics_dt > 0.0 {
                let step = physics_dt.min(1.0 / 60.0);
                gizmo::physics::physics_vehicle_system(world, step);
                gizmo::physics::physics_step_system(world, step);
                physics_dt -= step;
            }

            if state.show_physics_debug {
                gizmo::systems::physics::physics_debug_system(world);
            }

            let mut car_speed = 0.0;
            {
                let vel_store = world.borrow::<Velocity>();
                let trans_store = world.borrow::<Transform>();
                if let (Some(vel), Some(t)) = (vel_store.get(state.chassis_id), trans_store.get(state.chassis_id)) {
                    let forward = t.rotation * Vec3::new(0.0, 0.0, 1.0);
                    car_speed = vel.linear.dot(forward);
                }
            }

            // Tekerlek animasyonları
            state.wheel_spin -= car_speed * dt * 2.5; 
            let spin_quat = Quat::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), state.wheel_spin);
            let steer_quat = Quat::from_axis_angle(Vec3::new(0.0, 1.0, 0.0), state.steer_angle * 0.5);

            {
                let ecs_transforms = world.borrow_mut::<Transform>();
                // Tekerlekleri animate et
                if let Some(id) = state.wheel_fl {
                    if let Some(mut t) = ecs_transforms.get_mut(id) {
                        t.rotation = state.wheel_fl_rot * steer_quat * spin_quat;
                        t.update_local_matrix();
                    }
                }
                if let Some(id) = state.wheel_fr {
                    if let Some(mut t) = ecs_transforms.get_mut(id) {
                        t.rotation = state.wheel_fr_rot * steer_quat * spin_quat;
                        t.update_local_matrix();
                    }
                }
                if let Some(id) = state.wheel_bl {
                    if let Some(mut t) = ecs_transforms.get_mut(id) {
                        t.rotation = state.wheel_bl_rot * spin_quat;
                        t.update_local_matrix();
                    }
                }
                if let Some(id) = state.wheel_br {
                    if let Some(mut t) = ecs_transforms.get_mut(id) {
                        t.rotation = state.wheel_br_rot * spin_quat;
                        t.update_local_matrix();
                    }
                }
            }

            // Hiyerarşiyi (Parent/Child) ve GlobalTransform'ları güncelle!
            // Bu olmadan GLTF modelinin parçaları (tekerlek vb.) ana şasiyi takip etmez
            // ve orjinde (0,0,0) asılı kalır/çizilmez.
            // NOT: Artık TransformPlugin üzerinden scheduler tarafından otomatik çalıştırılıyor.

            // Lastik İzi (Skid Mark) oluşturma
            if state.is_drifting && state.last_skid_time > 0.02 && !state.skid_pool.is_empty() {
                state.last_skid_time = 0.0;
                
                // 1. Oku (Immutable Borrows) - Scope içine alarak borrow'u hemen serbest bırakıyoruz
                let car_rot;
                let bl_pos;
                let br_pos;
                {
                    let transforms = world.borrow::<Transform>();
                    let globals = world.borrow::<GlobalTransform>();
                    
                    car_rot = transforms.get(state.chassis_id).map(|t| t.rotation).unwrap_or(Quat::IDENTITY);
                    bl_pos = state.wheel_bl.and_then(|id| globals.get(id)).map(|g| g.matrix.transform_point3(Vec3::ZERO));
                    br_pos = state.wheel_br.and_then(|id| globals.get(id)).map(|g| g.matrix.transform_point3(Vec3::ZERO));
                }

                // 2. Yaz (Mutable Borrow)
                let transforms = world.borrow_mut::<Transform>();
                
                if let Some(pos) = bl_pos {
                    if let Some(&skid_id) = state.skid_pool.get(state.skid_idx) {
                        state.skid_idx = (state.skid_idx + 1) % state.skid_pool.len();
                        if let Some(mut t) = transforms.get_mut(skid_id) {
                            t.position = Vec3::new(pos.x, 0.02, pos.z);
                            t.rotation = car_rot;
                            t.update_local_matrix();
                        }
                    }
                }
                
                if let Some(pos) = br_pos {
                    if let Some(&skid_id) = state.skid_pool.get(state.skid_idx) {
                        state.skid_idx = (state.skid_idx + 1) % state.skid_pool.len();
                        if let Some(mut t) = transforms.get_mut(skid_id) {
                            t.position = Vec3::new(pos.x, 0.02, pos.z);
                            t.rotation = car_rot;
                            t.update_local_matrix();
                        }
                    }
                }
            } else {
                state.last_skid_time += dt;
            }

            // Kamera: Arabayı yarış oyunu gibi takip et
            if let Some(t) = world.borrow::<Transform>().get(state.chassis_id) {
                // Arabanın forward yönü (+Z ekseni)
                let forward = t.rotation * Vec3::new(0.0, 0.0, 1.0);
                
                // Hedef pozisyon: Arabanın arkasında ve yukarısında
                let distance = 8.0; // Biraz daha yaklaştırdık (eskiden 10.0 idi)
                let height = 3.0;
                let target_pos = t.position - forward * distance + Vec3::new(0.0, height, 0.0);
                
                // Kamerayı hedef pozisyona çok daha yumuşak ve framerate-bağımsız (sabit) götür
                let lerp_factor = 1.0 - (-15.0 * dt).exp();
                state.cam_pos = state.cam_pos.lerp(target_pos, lerp_factor);
                
                // Kameranın hedeften maksimum ne kadar uzaklaşabileceğini sınırla (lastik bant etkisi)
                let max_lag_distance = 3.0;
                let diff = state.cam_pos - target_pos;
                if diff.length() > max_lag_distance {
                    state.cam_pos = target_pos + diff.normalize() * max_lag_distance;
                }

                // Kameranın bakacağı nokta (arabanın biraz üstü)
                let look_target = t.position + Vec3::new(0.0, 1.0, 0.0);
                let look_dir = (look_target - state.cam_pos).normalize();

                // Hedef yaw ve pitch'i hesapla
                // atan2(y, x) -> atan2(fz, fx)
                state.cam_yaw = look_dir.z.atan2(look_dir.x);
                state.cam_pitch = look_dir.y.asin();
            }

            update_camera(world, state, input, dt);
        })
        .set_ui(|world, state, ctx| {
            let mut speed_kmh = 0.0;
            if let Some(phys_world) = world.get_resource::<PhysicsWorld>() {
                if let Some(&idx) = phys_world.entity_index_map.get(&state.chassis_id) {
                    let vel = phys_world.velocities[idx];
                    speed_kmh = vel.linear.length() * 3.6; // m/s to km/h
                }
            }

            egui::Area::new(egui::Id::new("hud_area"))
                .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-40.0, -40.0))
                .show(ctx, |ui| {
                    ui.label(
                        egui::RichText::new(format!("{:.0} km/h", speed_kmh))
                            .size(64.0)
                            .color(egui::Color32::WHITE)
                            .strong(),
                    );
                    ui.label(format!("Wheels Found: FL:{} FR:{} BL:{} BR:{}", 
                        state.wheel_fl.is_some(), state.wheel_fr.is_some(), 
                        state.wheel_bl.is_some(), state.wheel_br.is_some()));
                });

            egui::Window::new("🛠 Gizmo Debugger")
                .default_pos([10.0, 10.0])
                .show(ctx, |ui| {
                    ui.checkbox(&mut state.show_physics_debug, "Görsel Çarpışma (Debug Draw)");
                    if let Some(mut phys) = world.get_resource_mut::<PhysicsWorld>() {
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut phys.is_paused, "Durdur (Pause)");
                            if ui.button("1 Adım İleri (Step)").clicked() {
                                phys.step_once = true;
                            }
                            if ui.button("Geri Al (Rewind)").clicked() {
                                phys.rewind_requested = true;
                            }
                        });
                        ui.label(format!("Aktif Obje Sayısı: {}", phys.rigid_bodies.len()));
                    }
                });

            egui::Window::new("Car Inspector")
                .default_pos([10.0, 150.0])
                .show(ctx, |ui| {
                    let vehicle_store = world.borrow::<Vehicle>();
                    if let Some(vehicle) = vehicle_store.get(state.chassis_id) {
                        ui.separator();
                        ui.heading("Gearbox");
                        let gear_display = if vehicle.gearbox.is_reversing { "R".to_string() } else { (vehicle.gearbox.current_gear + 1).to_string() };
                        let mode = if vehicle.gearbox.is_automatic { "Auto (Press T to switch)" } else { "Manual (Q/E to shift)" };
                        ui.label(format!("Gear: {} | Mode: {}", gear_display, mode));
                        ui.label(format!("Throttle: {:.2} | Brake: {:.2} | Steer: {:.2}", vehicle.current_throttle, vehicle.current_brake, vehicle.current_steer));
                        ui.separator();
                    }

                    ui.heading("Engine & Steering");
                    ui.add(egui::Slider::new(&mut state.config.engine_power, 500.0..=20000.0).text("Engine Power"));
                    ui.add(egui::Slider::new(&mut state.config.engine_brake, 0.0..=10.0).text("Engine Brake"));
                    ui.add(egui::Slider::new(&mut state.config.steer_speed, 1.0..=20.0).text("Steer Speed"));
                    ui.add(egui::Slider::new(&mut state.config.steer_auto_return, 1.0..=30.0).text("Steer Auto Return"));
                    ui.add(egui::Slider::new(&mut state.config.steer_torque, 500.0..=10000.0).text("Steer Torque"));
                    
                    ui.separator();
                    ui.heading("Friction & Drift");
                    ui.add(egui::Slider::new(&mut state.config.base_grip, 1.0..=30.0).text("Base Grip (Asphalt)"));
                    ui.add(egui::Slider::new(&mut state.config.drift_grip, 0.1..=10.0).text("Drift Grip"));
                    ui.add(egui::Slider::new(&mut state.config.slip_threshold, 1.0..=20.0).text("Slip Threshold"));
                    
                    ui.separator();
                    ui.separator();
                    ui.heading("World Physics & Weather");
                    
                    let prev_weather = state.weather_idx;
                    egui::ComboBox::from_label("Weather")
                        .selected_text(match state.weather_idx {
                            0 => "Sunny",
                            1 => "Rain",
                            2 => "Snow",
                            _ => "Sunny",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut state.weather_idx, 0, "Sunny");
                            ui.selectable_value(&mut state.weather_idx, 1, "Rain");
                            ui.selectable_value(&mut state.weather_idx, 2, "Snow");
                        });
                        
                    if prev_weather != state.weather_idx {
                        if let Some(mut phys_world) = world.get_resource_mut::<PhysicsWorld>() {
                            phys_world.weather = match state.weather_idx {
                                0 => gizmo::physics::world::Weather::Sunny,
                                1 => gizmo::physics::world::Weather::Rain,
                                2 => gizmo::physics::world::Weather::Snow,
                                _ => gizmo::physics::world::Weather::Sunny,
                            };
                        }
                        
                        // Update Ground Material Color
                        if let Some(gid) = state.ground_id {
                            let materials = world.borrow_mut::<Material>();
                            if let Some(mut mat) = materials.get_mut(gid) {
                                match state.weather_idx {
                                    0 => { // Sunny
                                        mat.albedo = Vec4::new(0.15, 0.15, 0.15, 1.0);
                                        mat.roughness = 0.9;
                                    },
                                    1 => { // Rain (Dark and Shiny)
                                        mat.albedo = Vec4::new(0.15, 0.15, 0.2, 1.0);
                                        mat.roughness = 0.2;
                                    },
                                    2 => { // Snow (White)
                                        mat.albedo = Vec4::new(0.9, 0.9, 0.95, 1.0);
                                        mat.roughness = 0.9;
                                    },
                                    _ => {}
                                }
                            }
                        }
                    }

                    if ui.add(egui::Slider::new(&mut state.config.gravity_y, -50.0..=0.0).text("Gravity (Y)")).changed() {
                        if let Some(mut phys_world) = world.get_resource_mut::<PhysicsWorld>() {
                            phys_world.integrator.gravity.y = state.config.gravity_y;
                        }
                    }
                });
        })
        .set_render(|world, _state, encoder, view, renderer, _light_time| {
            // Bu sahnede sıvı/parçacık/GPU fizik yok — kapat.
            renderer.gpu_fluid = None;
            renderer.gpu_particles = None;
            renderer.gpu_physics = None;

            // Ağır post-processing pass'lerini kapat.
            renderer.ssr = None;
            renderer.ssgi = None;
            renderer.volumetric = None;
            renderer.taa = None;

            gizmo::systems::default_render_pass(world, encoder, view, renderer);
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

fn setup_scene(world: &mut World, renderer: &gizmo::renderer::Renderer) -> CarDemoState {
    println!("Araba Demo Yükleniyor...");

    let config = CarConfig::default();

    let mut asset_manager = AssetManager::new();
    let mut phys_world = PhysicsWorld::new();
    phys_world.integrator.gravity = Vec3::new(0.0, config.gravity_y, 0.0);

    let tex = asset_manager.create_white_texture(
        &renderer.device,
        &renderer.queue,
        &renderer.scene.texture_bind_group_layout,
    );

    // 1. ZEMİN
    let ground_mesh = AssetManager::create_plane(&renderer.device, 200.0);
    let ground = world.spawn();
    add_transform(world, ground, Transform::new(Vec3::ZERO).with_scale(Vec3::splat(1.0)));
    world.add_component(ground, ground_mesh);
    world.add_component(
        ground,
        Material::new(tex.clone()).with_pbr(Vec4::new(0.15, 0.15, 0.15, 1.0), 0.9, 0.0),
    );
    world.add_component(ground, MeshRenderer::new());

    let ground_rb = RigidBody::new_static();
    world.add_component(ground, ground_rb);
    world.add_component(ground, Velocity::default());
    world.add_component(
        ground,
        Collider::plane(Vec3::Y, 0.0),
    );
    world.add_component(ground, gizmo::physics::components::PhysicsMaterial::ASPHALT);
    
    phys_world.add_body(
        Entity::new(ground.id(), 0),
        ground_rb,
        Transform::new(Vec3::ZERO),
        Velocity::default(),
        Collider::plane(Vec3::Y, 0.0),
    );

    // --- BUZ ZEMİNİ (KAYGAN) ---
    let ice_mesh = AssetManager::create_plane(&renderer.device, 40.0);
    let ice = world.spawn();
    add_transform(world, ice, Transform::new(Vec3::new(40.0, 0.01, 0.0)));
    world.add_component(ice, ice_mesh);
    world.add_component(ice, Material::new(tex.clone()).with_pbr(Vec4::new(0.6, 0.8, 1.0, 1.0), 0.1, 0.9));
    world.add_component(ice, MeshRenderer::new());
    world.add_component(ice, Collider::box_collider(Vec3::new(20.0, 0.05, 20.0)));
    world.add_component(ice, gizmo::physics::components::PhysicsMaterial::ICE);
    phys_world.add_body(
        Entity::new(ice.id(), 0),
        RigidBody::new_static(),
        Transform::new(Vec3::new(40.0, 0.01, 0.0)),
        Velocity::default(),
        Collider::box_collider(Vec3::new(20.0, 0.05, 20.0))
    );

    // --- KUM/ÇAMUR ZEMİNİ (YÜKSEK DİRENÇ) ---
    let sand_mesh = AssetManager::create_plane(&renderer.device, 40.0);
    let sand = world.spawn();
    add_transform(world, sand, Transform::new(Vec3::new(-40.0, 0.01, 0.0)));
    world.add_component(sand, sand_mesh);
    world.add_component(sand, Material::new(tex.clone()).with_pbr(Vec4::new(0.8, 0.6, 0.4, 1.0), 0.9, 0.0));
    world.add_component(sand, MeshRenderer::new());
    world.add_component(sand, Collider::box_collider(Vec3::new(20.0, 0.05, 20.0)));
    world.add_component(sand, gizmo::physics::components::PhysicsMaterial::SAND);
    phys_world.add_body(
        Entity::new(sand.id(), 0),
        RigidBody::new_static(),
        Transform::new(Vec3::new(-40.0, 0.01, 0.0)),
        Velocity::default(),
        Collider::box_collider(Vec3::new(20.0, 0.05, 20.0))
    );

    phys_world.add_body(
        Entity::new(ground.id(), 0),
        ground_rb,
        Transform::new(Vec3::ZERO),
        Velocity::default(),
        Collider::plane(Vec3::Y, 0.0),
    );

    // 2. GÜNEŞ
    let sun = world.spawn();
    let sun_transform = Transform::new(Vec3::new(30.0, 80.0, 40.0)).with_rotation(
        Quat::from_axis_angle(Vec3::new(1.0, 0.3, 0.0).normalize(), -0.8),
    );
    add_transform(world, sun, sun_transform);
    world.add_component(
        sun,
        DirectionalLight::new(
            Vec3::new(1.0, 0.97, 0.90),
            2.5,
            gizmo::renderer::components::LightRole::Sun,
        ),
    );

    // 3. KAMERA
    let camera_ent = world.spawn();
    add_transform(world, camera_ent, Transform::new(Vec3::new(0.0, 5.0, 15.0)));
    world.add_component(
        camera_ent,
        Camera::new(
            std::f32::consts::FRAC_PI_4,
            0.1,
            1000.0,
            -std::f32::consts::FRAC_PI_2,
            -0.4,
            true,
        ),
    );

    // Commands kullanabilmek için asset_manager'i world'e ekleyelim
    world.insert_resource(asset_manager);

    // 4. ŞASİ (GLTF)
    let chassis_pos = Vec3::new(0.0, 1.5, 0.0);

    let chassis_entity = {
        let mut cmd = gizmo::prelude::SpawnCommands::new(world, renderer);
        let path = "/home/bedir/Documents/assets/kenney_racing-kit/Models/GLTF format/raceCarRed.glb";
        let builder = cmd.spawn_gltf(chassis_pos, path, false).unwrap();
        builder.id()
    };
    
    // Modelin scale'ini ayarlayalım (Kenney modelleri bazen küçük gelebiliyor)
    {
        let transforms = world.borrow_mut::<Transform>();
        if let Some(mut t) = transforms.get_mut(chassis_entity.id()) {
            t.scale = Vec3::splat(2.0); // 5.0 çok büyük gelmişti
            t.update_local_matrix();
        }
    }

    // RigidBody ayarlamaları
    let mut chassis_rb = RigidBody::new(config.chassis_mass, true);
    chassis_rb.linear_damping = config.linear_damping; // Asfalt sürtünmesini hissettirmek için sönümlemeyi artırdık
    chassis_rb.angular_damping = config.angular_damping; // Araba dönerken daha rahat kaysın / savrulsun
    chassis_rb.calculate_box_inertia(2.0, 1.0, 4.0);
    // Ağırlık merkezini (COM) yere çok daha yakın yapıyoruz (şahlanmayı önlemek için -1.0)
    chassis_rb.center_of_mass = Vec3::new(0.0, -1.0, 0.0);

    // Araç fiziğinde dönüş süspansiyondan geleceği için kilitleri kaldırıyoruz (Gerçekçi fizik)
    chassis_rb.lock_rotation_x = false;
    chassis_rb.lock_rotation_y = false;
    chassis_rb.lock_rotation_z = false;
    
    let mut vehicle = Vehicle {
        engine_power: config.engine_power,
        brake_force: config.engine_power * 0.8,
        ..Default::default()
    };
    
    let w_x = 1.0;
    let w_z = 1.5;
    let w_y = -0.55; // Kasanın hemen altından başlasın (Çarpışma hatasını önlemek için)
    
    // Front Left
    let wheel_fl = Wheel {
        local_position: Vec3::new(w_x, w_y, w_z),
        is_steering: true,
        is_drive: true,
        suspension_rest_length: 0.6,
        ..Default::default()
    };
    vehicle.wheels.push(wheel_fl);

    // Front Right
    let wheel_fr = Wheel {
        local_position: Vec3::new(-w_x, w_y, w_z),
        is_steering: true,
        is_drive: true,
        suspension_rest_length: 0.6,
        ..Default::default()
    };
    vehicle.wheels.push(wheel_fr);

    // Back Left
    let wheel_bl = Wheel {
        local_position: Vec3::new(w_x, w_y, -w_z),
        is_drive: true,
        suspension_rest_length: 0.6,
        ..Default::default()
    };
    vehicle.wheels.push(wheel_bl);

    // Back Right
    let wheel_br = Wheel {
        local_position: Vec3::new(-w_x, w_y, -w_z),
        is_drive: true,
        suspension_rest_length: 0.6,
        ..Default::default()
    };
    vehicle.wheels.push(wheel_br);

    world.add_component(chassis_entity, vehicle);
    world.add_component(chassis_entity, chassis_rb);
    world.add_component(chassis_entity, Velocity::new(Vec3::ZERO));
    world.add_component(
        chassis_entity,
        Collider::box_collider(Vec3::new(1.0, 0.5, 2.0)),
    );

    phys_world.add_body(
        Entity::new(chassis_entity.id(), 0),
        chassis_rb,
        Transform::new(chassis_pos),
        Velocity::default(),
        Collider::box_collider(Vec3::new(1.0, 0.5, 2.0)),
    );

    world.insert_resource(phys_world);

    println!("Sahne Hazır!");
    let mut state = CarDemoState::new(config);
    state.chassis_id = chassis_entity.id();
    state.camera_entity_id = camera_ent.id();
    state.ground_id = Some(ground.id());

    // GLTF yüklendiği için tekerlek entity'lerini isimlerinden bulup cache'le
    {
        let names = world.borrow::<gizmo::core::EntityName>();
        let ecs_transforms = world.borrow::<Transform>();
        for (id, name) in names.iter() {
            if name.0 == "wheelFrontLeft" { 
                state.wheel_fl = Some(id); 
                state.wheel_fl_rot = ecs_transforms.get(id).map(|t| t.rotation).unwrap_or(Quat::IDENTITY);
            } else if name.0 == "wheelFrontRight" { 
                state.wheel_fr = Some(id); 
                state.wheel_fr_rot = ecs_transforms.get(id).map(|t| t.rotation).unwrap_or(Quat::IDENTITY);
            } else if name.0 == "wheelBackLeft" { 
                state.wheel_bl = Some(id); 
                state.wheel_bl_rot = ecs_transforms.get(id).map(|t| t.rotation).unwrap_or(Quat::IDENTITY);
            } else if name.0 == "wheelBackRight" { 
                state.wheel_br = Some(id); 
                state.wheel_br_rot = ecs_transforms.get(id).map(|t| t.rotation).unwrap_or(Quat::IDENTITY);
            }
        }
    }

    // Skid mark (lastik izi) object pooling hazırlığı
    let skid_mat = {
        let mut asset_manager = world.get_resource_mut::<AssetManager>().unwrap();
        let default_bg = asset_manager.create_white_texture(
            &renderer.device,
            &renderer.queue,
            &renderer.scene.texture_bind_group_layout,
        );
        // Siyah ve biraz şeffaf bir unlit material (skid mark için)
        Material::new(default_bg)
            .with_unlit(Vec4::new(0.0, 0.0, 0.0, 0.8))
            .with_transparent(true)
    };

    let skid_mesh = AssetManager::create_plane(&renderer.device, 1.0);
    
    for _ in 0..100 {
        let e = world.spawn();
        // X ekseninde 0.4 (lastik genişliği), Z ekseninde 1.5 (lastik izi boyu)
        let t = Transform::new(Vec3::new(0.0, -100.0, 0.0)).with_scale(Vec3::new(0.4, 1.0, 1.5));
        world.add_component(e, t);
        world.add_component(e, GlobalTransform { matrix: t.local_matrix });
        world.add_component(e, skid_mesh.clone());
        world.add_component(e, skid_mat.clone());
        world.add_component(e, MeshRenderer::new());
        state.skid_pool.push(e.id());
    }

    state
}

fn update_camera(world: &mut World, state: &mut CarDemoState, input: &Input, _dt: f32) {
    if input.is_mouse_button_pressed(gizmo::core::input::mouse::RIGHT) {
        let delta = input.mouse_delta();
        state.cam_yaw += delta.0 * 0.005;
        state.cam_pitch += delta.1 * 0.005;
    }

    state.cam_pitch = state.cam_pitch.clamp(
        -std::f32::consts::FRAC_PI_2 + 0.1,
        std::f32::consts::FRAC_PI_2 - 0.1,
    );

    let fx = state.cam_yaw.cos() * state.cam_pitch.cos();
    let fy = state.cam_pitch.sin();
    let fz = state.cam_yaw.sin() * state.cam_pitch.cos();
    let _fwd = Vec3::new(fx, fy, fz).normalize();
    let _right = _fwd.cross(Vec3::Y).normalize();

    // Kamera entity'sini güncelle (hem Transform, hem Camera, hem GlobalTransform)
    {
        let cam_id = state.camera_entity_id;
        let transforms = world.borrow_mut::<Transform>();
        let globals = world.borrow_mut::<GlobalTransform>();
        let cameras = world.borrow_mut::<Camera>();

        if let Some(mut t) = transforms.get_mut(cam_id) {
            t.position = state.cam_pos;
            t.update_local_matrix();
            if let Some(mut g) = globals.get_mut(cam_id) {
                g.matrix = t.local_matrix;
            }
        }
        if let Some(mut cam) = cameras.get_mut(cam_id) {
            cam.yaw = state.cam_yaw;
            cam.pitch = state.cam_pitch;
        }
    }
}
