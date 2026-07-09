use gizmo::app::App;
use gizmo::core::input::Input;
use gizmo::core::world::World;
use gizmo::math::{Quat, Vec3, Vec4};
use gizmo::physics::components::{Collider, GlobalTransform, RigidBody, Velocity};
use gizmo::physics::world::PhysicsWorld;
use gizmo::physics::Transform;
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
            engine_power: 800.0, // Gerçekçi güç: gearbox ×10.5 çarpanıyla ~0.5g; yüksek değer aşırı pitch (wheelie/göğe fırlama) yapar
            steer_speed: 6.0,
            steer_auto_return: 15.0,
            steer_max_angle: 1.0,
            steer_torque: 1000.0,
            engine_brake: 2.5,
            base_grip: 8.0,
            slip_threshold: 6.0,
            drift_grip: 1.0,
            chassis_mass: 1200.0, // Standard car weight
            linear_damping: 0.1, // düşük: direnç vehicle sisteminin rolling-resistance+drag'inden gelir. 0.9 global sönüm terminal hızı ~drive/0.9 ≈ 28 km/h'ye kilitliyordu (motor gücünden bağımsız).
            angular_damping: 1.8,
            friction: 0.8,
            gravity_y: -9.81, // Gerçek-dünya yerçekimi
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
    fps_smooth: f32,
    phys_accum: f32,
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
            fps_smooth: 0.0,
            phys_accum: 0.0,
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
            // W/↑ = ileri gaz, S/↓ = geri vites gazı, Space = fren.
            let mut throttle: f32 = 0.0;
            if input.is_key_pressed(gizmo::prelude::KeyCode::ArrowUp as u32) || input.is_key_pressed(gizmo::prelude::KeyCode::KeyW as u32) {
                throttle += 1.0;
            }
            if input.is_key_pressed(gizmo::prelude::KeyCode::ArrowDown as u32) || input.is_key_pressed(gizmo::prelude::KeyCode::KeyS as u32) {
                throttle -= 1.0;
            }
            let brake = if input.is_key_pressed(gizmo::prelude::KeyCode::Space as u32) { 1.0 } else { 0.0 };

            // FPS (yumuşatılmış). NOT: present_mode AutoNoVsync = FPS sınırsız, sahne yüküyle
            // dalgalanır; sayının inip çıkması normaldir, hata değil.
            if dt > 0.0 {
                let inst = 1.0 / dt;
                state.fps_smooth = if state.fps_smooth <= 0.0 { inst } else { state.fps_smooth * 0.92 + inst * 0.08 };
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
                let mut vehicle_store = world.borrow_mut::<gizmo::physics::vehicle::VehicleController>();
                if let Some(mut vehicle) = vehicle_store.get_mut(state.chassis_id) {
                    // Gerçekçi model girdileri: throttle/brake 0..1, steering -1..1, reverse bool.
                    // S geri vitese alır VE geri gaz verir (eskiden throttle.max(0) ile yutulup
                    // yalnız fren kalıyordu → geri gitmiyordu).
                    vehicle.set_reverse(throttle < 0.0);
                    vehicle.throttle_input = throttle.abs().min(1.0);
                    vehicle.brake_input = brake;
                    let max_steer = state.config.steer_max_angle.max(0.01);
                    vehicle.steering_input = (state.steer_angle / max_steer).clamp(-1.0, 1.0);

                    // T: oto-vites aç/kapa
                    if input.is_key_just_pressed(gizmo::prelude::KeyCode::KeyT as u32) {
                        vehicle.auto_shift = !vehicle.auto_shift;
                    }
                    // Q/E: manuel vites (yalnız oto-vites kapalıyken)
                    if !vehicle.auto_shift {
                        let max_gear = vehicle.tuning.gear_ratios.len().saturating_sub(1);
                        if input.is_key_just_pressed(gizmo::prelude::KeyCode::KeyE as u32)
                            && vehicle.current_gear < max_gear
                        {
                            vehicle.current_gear += 1;
                        }
                        if input.is_key_just_pressed(gizmo::prelude::KeyCode::KeyQ as u32)
                            && vehicle.current_gear > 2
                        {
                            vehicle.current_gear -= 1;
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
                let mut transforms = unsafe { world.borrow_mut_unchecked::<Transform>() }; // SAFETY: distinct component types
                let mut velocities = unsafe { world.borrow_mut_unchecked::<Velocity>() };
                if let Some(mut t) = transforms.get_mut(state.chassis_id) {
                    *t = Transform::new(Vec3::new(0.0, 1.5, 0.0));
                }
                if let Some(mut v) = velocities.get_mut(state.chassis_id) {
                    *v = Velocity::default();
                }
            }

            // GERÇEK Sabit Zaman Adımı (accumulator). ÖNCEDEN `step = dt.min(1/60)` idi:
            // yüksek fps'de (dt<1/60) fizik HAM DEĞİŞKEN dt ile adımlıyordu. Frame süresi
            // 2-12 ms arası zıpladığından (uncapped render), sert süspansiyonun (45000 N/m)
            // açık entegrasyonu her frame farklı dt görüp normal yükü/şasi yüksekliğini
            // yüksek-frekans SALINDIRIYORDU = hızlandıkça hissedilen TİTREME. Sabit dt ile
            // adımlamak fiziği frame timing jitter'ından tamamen ayırır → titreme biter.
            const FIXED_DT: f32 = 1.0 / 240.0;
            state.phys_accum += dt.min(0.1);
            let mut steps = 0;
            while state.phys_accum >= FIXED_DT && steps < 32 {
                run_vehicle_controllers(world, FIXED_DT);
                gizmo::physics::physics_step_system(world, FIXED_DT);
                state.phys_accum -= FIXED_DT;
                steps += 1;
            }

            if state.show_physics_debug {
                gizmo::systems::physics::physics_debug_system(world);
            }

            let mut car_speed = 0.0;
            {
                let vel_store = world.borrow::<Velocity>();
                let trans_store = world.borrow::<Transform>();
                if let (Some(vel), Some(t)) = (vel_store.get(state.chassis_id), trans_store.get(state.chassis_id)) {
                    // VehicleController ileri yönü = -Z (tekerlek spin işareti için).
                    let forward = t.rotation * Vec3::new(0.0, 0.0, -1.0);
                    car_speed = vel.linear.dot(forward);
                }
            }

            // Tekerlek animasyonları
            state.wheel_spin -= car_speed * dt * 2.5; 
            let spin_quat = Quat::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), state.wheel_spin);
            let steer_quat = Quat::from_axis_angle(Vec3::new(0.0, 1.0, 0.0), state.steer_angle * 0.5);

            {
                let mut ecs_transforms = world.borrow_mut::<Transform>();
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
                let mut transforms = world.borrow_mut::<Transform>();
                
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
                // Arabanın ileri yönü = VehicleController ile aynı: local -Z. Kamera bunun
                // ARKASINDA (+Z tarafında) durur → gaz aracı kameradan UZAĞA sürer.
                let forward = t.rotation * Vec3::new(0.0, 0.0, -1.0);

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
            // Hız göstergesi = İLERİ (longitudinal) yer hızı. ESKİDEN `velocity.linear.length()`
            // (toplam hız magnitüdü) idi → DİKEY zıplama (spawn/sıçrama) ve YANAL kayma (drift/
            // spin) hızı şişirip "hızlı değilim ama HUD yüksek/kontrolsüz artıyor" yapıyordu.
            // VehicleController.current_speed_kmh = v·forward, yalnız ileri bileşen (gerçek
            // kilometre saati gibi). abs(): geri viteste negatif göstermesin.
            let speed_kmh = world
                .borrow::<gizmo::physics::vehicle::VehicleController>()
                .get(state.chassis_id)
                .map(|vc| vc.current_speed_kmh.abs())
                .unwrap_or(0.0);

            egui::Area::new(egui::Id::new("hud_area"))
                .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-40.0, -40.0))
                .show(ctx, |ui| {
                    ui.label(
                        egui::RichText::new(format!("{:.0} km/h", speed_kmh))
                            .size(64.0)
                            .color(egui::Color32::WHITE)
                            .strong(),
                    );
                    // FPS göstergesi (yeşil>50, sarı 30-50, kırmızı<30). AutoNoVsync olduğu için
                    // uncapped; sahne yüküyle dalgalanır. Debug build'de release'in ~yarısı olur.
                    let fps = state.fps_smooth;
                    let fps_col = if fps >= 50.0 { egui::Color32::from_rgb(80, 220, 100) }
                        else if fps >= 30.0 { egui::Color32::from_rgb(230, 200, 60) }
                        else { egui::Color32::from_rgb(230, 90, 90) };
                    ui.label(
                        egui::RichText::new(format!("{:.0} FPS  ({:.1} ms)", fps, if fps > 0.0 { 1000.0 / fps } else { 0.0 }))
                            .size(22.0).color(fps_col).strong(),
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
                    let vehicle_store = world.borrow::<gizmo::physics::vehicle::VehicleController>();
                    if let Some(vehicle) = vehicle_store.get(state.chassis_id) {
                        ui.separator();
                        ui.heading("Motor & Şanzıman (gerçekçi)");
                        // gear_ratios: [0]=Geri, [1]=Nötr, [2..]=ileri → gösterim: R / 1..N
                        let gear_display = if vehicle.reverse_input {
                            "R".to_string()
                        } else {
                            vehicle.current_gear.saturating_sub(1).to_string()
                        };
                        let mode = if vehicle.auto_shift { "Auto (T ile değiştir)" } else { "Manual (Q/E)" };
                        ui.label(format!("Vites: {} | Mod: {}", gear_display, mode));
                        ui.label(format!("RPM: {:.0} | Hız: {:.1} km/h", vehicle.engine_rpm, vehicle.current_speed_kmh));
                        ui.label(format!(
                            "Gaz: {:.2} | Fren: {:.2} | Direksiyon: {:.2}",
                            vehicle.throttle_input, vehicle.brake_input, vehicle.steering_input
                        ));
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
                            let mut materials = world.borrow_mut::<Material>();
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

            // ── 🔬 İNCELEME / ANALİZ MODU ────────────────────────────────────
            // "Araç neden havada?" — şasi fiziği ile görsel modelin dikey ilişkisini
            // ve süspansiyon denge yüksekliğini canlı sayılarla gösterir. Fiziği
            // dondurmak için yukarıdaki 🛠 Gizmo Debugger → "Durdur" + "1 Adım İleri",
            // çarpışma kutusunu görmek için "Görsel Çarpışma (Debug Draw)".
            egui::Window::new("🔬 İnceleme / Analiz")
                .default_pos([10.0, 470.0])
                .show(ctx, |ui| {
                    let transforms = world.borrow::<Transform>();
                    let vehicles = world.borrow::<gizmo::physics::vehicle::VehicleController>();

                    const GROUND_Y: f32 = 0.0;
                    // Model 2× ölçekte: görsel lastik altı = şasi merkezi + model_min_y(−0.0335)×2.
                    const TIRE_BOTTOM_OFFSET: f32 = -0.067;
                    // offset_box(0,0.52,0) half_y 0.22 → collider alt kenarı = cy + 0.30.
                    const COLLIDER_BOTTOM_OFFSET: f32 = 0.30;

                    if let Some(t) = transforms.get(state.chassis_id) {
                        let cy = t.position.y;
                        ui.heading("Şasi (fizik gövdesi)");
                        ui.label(format!("Merkez Y:            {:>7.3} m", cy));
                        let tire_bottom = cy + TIRE_BOTTOM_OFFSET;
                        ui.colored_label(
                            if tire_bottom.abs() < 0.06 {
                                egui::Color32::LIGHT_GREEN
                            } else {
                                egui::Color32::from_rgb(255, 120, 80)
                            },
                            format!("Görsel lastik altı:  {:>7.3} m  (hedef 0)", tire_bottom),
                        );
                        ui.label(format!("Collider alt kenarı: {:>7.3} m", cy + COLLIDER_BOTTOM_OFFSET));
                        ui.label(format!("Zemin Y:             {:>7.3} m", GROUND_Y));
                        ui.label("Görsel model ölçeği: 2.0×");
                    }

                    ui.separator();
                    ui.heading("Tekerlekler (süspansiyon raycast)");
                    if let Some(v) = vehicles.get(state.chassis_id) {
                        let names = ["FL", "FR", "BL", "BR"];
                        let mut grounded_count = 0;
                        for (i, w) in v.wheels.iter().enumerate() {
                            let n = names.get(i).copied().unwrap_or("?");
                            if w.is_grounded { grounded_count += 1; }
                            let contact_y = w.ground_hit.as_ref().map(|h| h.point.y).unwrap_or(0.0);
                            ui.label(format!(
                                "{}: {} | susp_uz {:>5.3} | temas Y {:>6.3}",
                                n,
                                if w.is_grounded { "YERDE " } else { "HAVADA" },
                                w.suspension_length,
                                contact_y,
                            ));
                        }
                        if let Some(w0) = v.wheels.first() {
                            let max_dist =
                                w0.suspension_rest_length + w0.suspension_max_travel + w0.radius;
                            ui.separator();
                            ui.label(format!(
                                "max_dist = rest({:.2}) + travel({:.2}) + r({:.2}) = {:.2} m",
                                w0.suspension_rest_length, w0.suspension_max_travel, w0.radius, max_dist,
                            ));
                            ui.label(format!("yerdeki tekerlek: {}/{}", grounded_count, v.wheels.len()));
                        }
                    }

                    ui.separator();
                    ui.label(
                        egui::RichText::new(
                            "Fizik tekerlekleri GLB modelinin gerçek göbeklerine hizalandı \
                             (göbek 0.2152, yarıçap 0.282, rest 0.10) → şasi ~0.09 m'de dengelenip \
                             görsel lastikler yere değer. Gravity −9.81, COM footprint merkezinde \
                             (0,0.20,0) → gazda aşırı pitch/fırlama azaltıldı.",
                        )
                        .small()
                        .italics(),
                    );
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

/// Bir GLB tekerlek düğümünün şasiye göre fizik `local_position`'ını ve `radius`'unu
/// modelden türetir (elle sayı yerine). `Parent` zincirini yürüyerek local matrisleri
/// biriktirir → şasinin çocuk-çerçevesindeki konum; `chassis_scale` ile ölçeklenir
/// (fizik raycast'i scale uygulamaz, bu yüzden ölçek konuma gömülür). Yarıçap tekerlek
/// mesh'inin (kendi ya da bir çocuk "prim") Y/Z boyutundan alınır. Model isimli tekerlek
/// içermiyorsa `None` döner → çağıran varsayılana düşer.
fn derive_wheel(
    world: &World,
    chassis_id: u32,
    chassis_scale: Vec3,
    name: &str,
) -> Option<(Vec3, f32)> {
    // 1. İsimle tekerlek entity'sini bul.
    let wheel_id = {
        let names = world.borrow::<gizmo::core::EntityName>();
        let mut found = None;
        for (id, n) in names.iter() {
            if n.0 == name {
                found = Some(id);
                break;
            }
        }
        found?
    };

    // 2. Local matrisleri tekerlekten şasinin doğrudan çocuğuna kadar biriktir.
    let mut mat = {
        let ts = world.borrow::<Transform>();
        ts.get(wheel_id)?.local_matrix
    };
    let mut cur = wheel_id;
    for _ in 0..16 {
        // döngü-güvenliği
        let parent = world
            .borrow::<gizmo::core::component::Parent>()
            .get(cur)
            .map(|p| p.0);
        match parent {
            Some(p) if p == chassis_id => break,
            Some(p) => {
                let p_mat = {
                    let ts = world.borrow::<Transform>();
                    ts.get(p).map(|t| t.local_matrix)
                };
                match p_mat {
                    Some(m) => {
                        mat = m * mat;
                        cur = p;
                    }
                    None => break,
                }
            }
            None => break,
        }
    }
    let local_trans = mat.to_scale_rotation_translation().2;
    let pos = chassis_scale * local_trans; // bileşen-bileşen; ölçek konuma gömülür

    // 3. Yarıçap: tekerlek mesh'inin (kendi ya da çocuk prim) Y/Z yarı-boyutu × ölçek.
    let radius = wheel_mesh_radius(world, wheel_id, chassis_scale.y);
    Some((pos, radius))
}

/// Tekerlek entity'sinin (ya da bir çocuğunun) Mesh.bounds'undan lastik yarıçapını
/// kestirir: dönme düzlemindeki (Y/Z) en büyük yarı-boyut × ölçek.
fn wheel_mesh_radius(world: &World, wheel_id: u32, scale_y: f32) -> f32 {
    let meshes = world.borrow::<gizmo::renderer::components::Mesh>();
    let from = |m: &gizmo::renderer::components::Mesh| -> f32 {
        let y = (m.bounds.max.y - m.bounds.min.y) * 0.5;
        let z = (m.bounds.max.z - m.bounds.min.z) * 0.5;
        y.max(z) * scale_y
    };
    if let Some(m) = meshes.get(wheel_id) {
        return from(m);
    }
    let kids = world
        .borrow::<gizmo::core::component::Children>()
        .get(wheel_id)
        .map(|c| c.0.clone());
    if let Some(kids) = kids {
        for k in kids {
            if let Some(m) = meshes.get(k) {
                return from(m);
            }
        }
    }
    0.3 // makul varsayılan
}

/// Demo-yerel araç sürücüsü. Artık motorda karşılığı VAR: `VehicleController`'ın fizik
/// fonksiyonu `update_vehicle` M7.2'de `gizmo_physics_dynamics::vehicle_controller_system`
/// olarak `Phase::Physics`'e kaydedildi (yani ÖLÜ KOD DEĞİL). Bu demo hâlâ kendi yerel
/// kopyasını çağırıyor; sahnedeki tüm collider'ları toplayıp her araç için raycast+Pacejka+
/// anti-roll fiziğini çalıştırır. (İleride: bu demo'yu yerel sürücü yerine motor sistemine
/// bağla — sürüş hissi EKRAN doğrulaması gerektirdiğinden ayrı bir iş olarak ertelendi.)
fn run_vehicle_controllers(world: &World, dt: f32) {
    use gizmo::physics::BodyHandle;
    // 1. Tüm collider'lar — raycast için sahiplenilmiş anlık görüntü (borrow çakışmasını önler).
    let mut all: Vec<(BodyHandle, Transform, Collider)> = Vec::new();
    if let Some(q) = world.query::<(&Transform, &Collider)>() {
        for (id, (t, c)) in q.iter() {
            all.push((BodyHandle::from_id(id), *t, c.clone()));
        }
    }
    // 2. Her VehicleController'ı sür (update_vehicle rb/vel'i yerinde değiştirir; self'i dışlar).
    // SAFETY: physics_vehicle_system ile aynı desen — disjoint bileşen erişimi (RigidBody/
    // Velocity/VehicleController ayrı tipler), scheduler/demo tek-thread çağırır.
    if let Some(mut q) = unsafe {
        world.query_unchecked::<(
            &Transform,
            gizmo::core::query::Mut<RigidBody>,
            gizmo::core::query::Mut<Velocity>,
            gizmo::core::query::Mut<gizmo::physics::vehicle::VehicleController>,
        )>()
    } {
        for (id, (t, mut rb, mut vel, mut vc)) in q.iter_mut() {
            gizmo::physics::vehicle::update_vehicle(
                BodyHandle::from_id(id),
                &mut vc,
                &mut rb,
                t,
                &mut vel,
                &all,
                dt,
            );
        }
    }
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
        gizmo::physics::BodyHandle::from_id(ground.id()),
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
        gizmo::physics::BodyHandle::from_id(ice.id()),
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
        gizmo::physics::BodyHandle::from_id(sand.id()),
        RigidBody::new_static(),
        Transform::new(Vec3::new(-40.0, 0.01, 0.0)),
        Velocity::default(),
        Collider::box_collider(Vec3::new(20.0, 0.05, 20.0))
    );

    phys_world.add_body(
        gizmo::physics::BodyHandle::from_id(ground.id()),
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
        let mut transforms = world.borrow_mut::<Transform>();
        if let Some(mut t) = transforms.get_mut(chassis_entity.id()) {
            t.scale = Vec3::splat(2.0); // 5.0 çok büyük gelmişti
            t.update_local_matrix();
        }
    }

    // MODELİ ORİJİNE YENİDEN MERKEZLE: raceCarRed GLB kök düğümü
    // [-0.35, -0.01, -0.65] offset taşıyor → araç şasi orijininden yana/geriye
    // kayık duruyordu (kamera ve fizik footprint'i orijini takip ettiğinden araç
    // ekranda ve fizik-görsel hizasında kayıyordu). Doğrudan çocuğun (GLB kökü)
    // x/z'sini sıfırlayınca tüm gövde+tekerlekler orijine simetrik oturur; böylece
    // fizik tekerlek local_position'larını (aşağıda) simetrik verebiliriz.
    {
        let kids = world
            .borrow::<gizmo::core::component::Children>()
            .get(chassis_entity.id())
            .map(|c| c.0.clone());
        if let Some(kids) = kids {
            let mut transforms = world.borrow_mut::<Transform>();
            for kid in kids {
                if let Some(mut t) = transforms.get_mut(kid) {
                    t.position.x = 0.0;
                    t.position.z = 0.0;
                    // VehicleController "ileri" = local -Z (Ackermann konvansiyonu), ama Kenney
                    // modeli +Z'ye bakıyor. Modeli 180° Y çevir → görsel ön -Z olur; X ve Z işaret
                    // değişip ön/arka + sol/sağ etiketleri VC'nin (-Z ön, +X sağ) tam hizalanır
                    // (türetilen fizik tekerlekleri de bu rotasyonu içerir).
                    t.rotation = Quat::from_rotation_y(std::f32::consts::PI) * t.rotation;
                    t.update_local_matrix();
                }
            }
        }
    }

    // RigidBody ayarlamaları
    let mut chassis_rb = RigidBody::new(config.chassis_mass, true);
    chassis_rb.linear_damping = config.linear_damping; // Asfalt sürtünmesini hissettirmek için sönümlemeyi artırdık
    chassis_rb.angular_damping = config.angular_damping; // Araba dönerken daha rahat kaysın / savrulsun
    // Gerçek araç gövde boyutları (model 2× ölçekte ~1.46 × 0.8 × 2.7 m).
    chassis_rb.calculate_box_inertia(1.46, 0.8, 2.7);
    // COM tekerlek footprint'inin merkezinde ve makul yükseklikte (yerden ~0.3 m).
    // Model yeniden merkezlendiği için x/z = 0. Eski -1.0 offset + keyfi geometri,
    // tahrik kuvvetini COM'un çok altında uygulayıp aşırı pitch (göğe fırlama) yapıyordu.
    chassis_rb.center_of_mass = Vec3::new(0.0, 0.20, 0.0);

    // Araç fiziğinde dönüş süspansiyondan geleceği için kilitleri kaldırıyoruz (Gerçekçi fizik)
    chassis_rb.lock_rotation_x = false;
    chassis_rb.lock_rotation_y = false;
    chassis_rb.lock_rotation_z = false;
    
    // GERÇEKÇİ ARAÇ: motorun VehicleController'ı (Pacejka lastik + Ackermann direksiyon
    // + anti-roll bar + motor tork eğrisi). Arcade Vehicle yerine bunu kullanıyoruz;
    // fiziği `run_vehicle_controllers` (yukarıda yazdığımız eksik sistem) sürer.
    let mut vehicle = gizmo::physics::vehicle::VehicleController::new();

    // ── TEKERLEKLER MODELDEN OTOMATİK TÜRETİLİR ──────────────────────────────
    // (Aynı model-türetme: GLB isimli düğümleri → konum + yarıçap; başka model de çalışır.)
    let cs = Vec3::splat(2.0); // chassis (görsel) scale
    let wheel_names = [
        "wheelFrontLeft",
        "wheelFrontRight",
        "wheelBackLeft",
        "wheelBackRight",
    ];
    for name in wheel_names {
        let (pos, radius) = derive_wheel(world, chassis_entity.id(), cs, name).unwrap_or_else(|| {
            let x = if name.contains("Right") { -0.41 } else { 0.41 };
            let z = if name.contains("Front") { 0.655 } else { -0.947 };
            (Vec3::new(x, 0.2152, z), 0.282)
        });
        let axle = if name.contains("Front") {
            gizmo::physics::vehicle::Axle::Front
        } else {
            gizmo::physics::vehicle::Axle::Rear
        };
        vehicle.add_wheel(gizmo::physics::vehicle::Wheel {
            attachment_local_pos: pos,
            radius,
            axle_type: axle,
            is_left: name.contains("Left"),
            // Kısa süspansiyon → şasi alçakta oturur, görsel lastikler yere değer.
            suspension_rest_length: (radius * 0.22).max(0.05),
            suspension_max_travel: (radius * 0.4).max(0.10),
            suspension_stiffness: 45000.0,
            suspension_damping: 3500.0,
            wheel_mass: 25.0, // makul; spin kararlılığı artık implisit-güncelleme ile sağlanıyor (60 hack'i fantom sürüklemeyi 3×'liyordu)
            ..Default::default()
        });
    }
    // Ölçek küçük olduğundan wheelbase/track'i türetilen tekerleklerden ayarla.
    vehicle.tuning.wheelbase = 1.6;
    vehicle.tuning.track_width = 0.82;
    // Bu küçük araç + minik lastikler için motor torkunu makul tut: 350 N·m ×gear
    // lastik tutuşunu kat kat aşıp sürekli patinaj yapıyordu. 150 N·m grippy kalkış verir.
    vehicle.tuning.max_engine_torque = 450.0;
    // Tam direksiyon 30°(0.52) hızda sert scrub/yavaşlama yapıyordu → 18°(0.32).
    vehicle.max_steering_angle = 0.40;

    world.add_component(chassis_entity, vehicle);
    world.add_component(chassis_entity, chassis_rb);
    world.add_component(chassis_entity, Velocity::new(Vec3::ZERO));
    // Şasi collider'ı görsel gövdeye oturan, offset'li bir kutu. KRİTİK: alt kenarı
    // tekerlek göbeklerinin (+0.2152) ÜSTÜNDE (0.52−0.22 = 0.30) tutuldu; yoksa
    // aşağı bakan süspansiyon ışını collider'ın içinden başlayıp KENDİNİ vururdu
    // (raycast self-exclusion yapmaz) → tekerlek "yerde değil" sanılıp araç düşerdi.
    let chassis_collider = Collider::offset_box(
        Vec3::new(0.0, 0.52, 0.0),
        Vec3::new(0.73, 0.22, 1.35),
    );
    world.add_component(chassis_entity, chassis_collider.clone());

    phys_world.add_body(
        gizmo::physics::BodyHandle::from_id(chassis_entity.id()),
        chassis_rb,
        Transform::new(chassis_pos),
        Velocity::default(),
        chassis_collider,
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
        let mut transforms = unsafe { world.borrow_mut_unchecked::<Transform>() }; // SAFETY: distinct component types
        let mut globals = unsafe { world.borrow_mut_unchecked::<GlobalTransform>() };
        let mut cameras = unsafe { world.borrow_mut_unchecked::<Camera>() };

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
