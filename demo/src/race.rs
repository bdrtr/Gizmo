/// PS1 Tarzı Yarış Sahnesi — GLTF model tabanlı pist ve araçlar
/// 
/// Kullanım:
/// 1. Blender'da pist modeli tasarla → `demo/assets/track.glb` olarak kaydet
/// 2. Araç modelleri → `demo/assets/car_player.glb`, `car_ai_1.glb` vb.
/// 3. Waypoint'leri `race_waypoints()` fonksiyonunda tanımla
/// 4. `setup_race_scene()` çağır
///
/// Pist modeli için Collider ekleme:
/// - Pist modeli sadece görsel. Fizik için AABB duvarlar eklenir.
/// - Ya da Blender'da her duvarı ayrı obje yapıp isimle Collider eşle.
use gizmo::prelude::*;
use gizmo::physics::{RaceAI, VehicleController, Wheel, RigidBody, Collider};

/// Yarış durumu (ECS Resource olarak saklanır)
#[derive(Clone, Debug)]
pub struct RaceState {
    pub phase: RacePhase,
    pub countdown_timer: f32,
    pub total_laps: u32,
    pub player_entity: u32,
    pub camera_entity: u32,
    pub ai_entities: Vec<u32>,
    pub race_timer: f32,
    /// Bitiş sıralaması (entity_id, süre)
    pub finish_order: Vec<(u32, f32)>,
    /// Oyuncunun tamamladığı tur
    pub player_laps: u32,
    /// Oyuncunun son checkpoint indeksi
    pub player_last_checkpoint: usize,
    /// Toplam checkpoint sayısı (tur tespiti için)
    pub checkpoint_count: usize,
    /// Checkpoint geçiş sayısı (tur hesabı)
    pub player_checkpoints_passed: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RacePhase {
    Countdown, // 3-2-1-GO!
    Racing,    // Yarış devam ediyor
    Finished,  // Yarış bitti
}

/// Pist konfigürasyonu — Waypoint'ler, grid pozisyonları, model yolları
pub struct TrackConfig {
    /// Pistteki waypoint'ler (AI yolu + checkpoint tespiti için)
    /// Bunları Blender'da "Empty" objeler koyarak da çıkarabilirsin
    pub waypoints: Vec<Vec3>,
    /// Başlangıç grid pozisyonları (index 0 = P1, 1 = P2, ...)
    pub grid_positions: Vec<Vec3>,
    /// Başlangıç yönleri
    pub grid_rotations: Vec<Quat>,
    /// Pist modeli dosya yolu (opsiyonel — None ise prosedürel)
    pub track_model: Option<String>,
    /// Araç modeli dosya yolları (opsiyonel — None ise küp placeholder)
    /// [0] = oyuncu, [1..] = AI araçlar
    pub car_models: Vec<Option<String>>,
    /// Araç renkleri (model yoksa küp rengi, model varsa tint)
    pub car_colors: Vec<[f32; 4]>,
    /// AI zorluk çarpanları
    pub ai_speeds: Vec<f32>,
    /// Toplam tur
    pub total_laps: u32,
    /// Fizik zemin yüksekliği
    pub ground_y: f32,
}

impl Default for TrackConfig {
    fn default() -> Self {
        // Oval / Racetrack şeklinde sıralı waypointler üret
        let mut waypoints = Vec::new();
        let num_points = 36;
        let radius_x = 80.0;
        let radius_z = 120.0;
        
        for i in 0..num_points {
            let angle = (i as f32 / num_points as f32) * std::f32::consts::PI * 2.0;
            // Hafif dalgalı oval
            let x = angle.cos() * radius_x;
            let z = angle.sin() * radius_z + (angle * 2.0).sin() * 20.0; 
            waypoints.push(Vec3::new(x, 0.5, z));
        }

        // Başlangıç noktaları (ilk wp = sin=0 -> z=0, cos=1 -> x=80, yani x ekseninde pozitif yan)
        // Araçların grid pozisyonları pist üzerinde başlangıcın hemen öncesinde olsun
        let mut grid_positions = Vec::new();
        let mut grid_rotations = Vec::new();
        
        let start_angle = -0.1_f32; // Başlangıçtan biraz geride
        for i in 0..4 {
            let offset_angle = start_angle - (i / 2) as f32 * 0.05;
            let side_offset = if i % 2 == 0 { -4.0 } else { 4.0 };
            
            let x = offset_angle.cos() * radius_x;
            let z = offset_angle.sin() * radius_z + (offset_angle * 2.0).sin() * 20.0;
            
            // Teğet yönünü (ileriyi) bulmak için türev/ufak delta alalım
            let delta_angle = offset_angle + 0.05;
            let dx = delta_angle.cos() * radius_x;
            let dz = delta_angle.sin() * radius_z + (delta_angle * 2.0).sin() * 20.0;
            
            let forward = Vec3::new(dx - x, 0.0, dz - z).normalize();
            let right = Vec3::new(-forward.z, 0.0, forward.x);
            
            grid_positions.push(Vec3::new(x, 1.0, z) + right * side_offset);
            
            let yaw = forward.x.atan2(forward.z);
            grid_rotations.push(Quat::from_axis_angle(Vec3::new(0.0, 1.0, 0.0), yaw));
        }

        Self {
            waypoints,
            grid_positions,
            grid_rotations,
            track_model: None,
            car_models: vec![None; 4],
            car_colors: vec![
                [0.8, 0.1, 0.1, 1.0], // Kırmızı (Player)
                [0.1, 0.4, 0.8, 1.0], // Mavi
                [0.1, 0.7, 0.2, 1.0], // Yeşil
                [0.9, 0.7, 0.1, 1.0], // Sarı
            ],
            ai_speeds: vec![0.8, 0.9, 1.0], // Daha agrasif AI
            total_laps: 3,
            ground_y: -0.5,
        }
    }
}

/// Araç entity'si oluştur
fn spawn_vehicle(
    world: &mut World,
    renderer: &gizmo::renderer::Renderer,
    position: Vec3,
    rotation: Quat,
    color: [f32; 4],
    model_path: &Option<String>,
    tbind: std::sync::Arc<wgpu::BindGroup>,
) -> Entity {
    let entity = world.spawn();
    world.add_component(entity, Transform::new(position)
        .with_rotation(rotation));

    // Daha hafif araç, daha düşük sürtünme (Yüksek top speed için)
    let mut rb = RigidBody::new(600.0, 0.02, 0.8, true); 
    rb.calculate_box_inertia(2.0, 1.0, 4.0);
    world.add_component(entity, rb);
    world.add_component(entity, Collider::new_aabb(1.0, 0.5, 2.0));
    world.add_component(entity, gizmo::physics::Velocity::new(Vec3::ZERO));

    // VehicleController — 4 tekerlek (RWD)
    let mut vc = VehicleController::new();
    // Arcade tuning overrides: Çok güçlü yanal tutuş ve yüksek direksiyon torku
    vc.lateral_grip = 18000.0;
    vc.steering_force_mult = 15000.0;
    vc.anti_slide_force = 12000.0;

    vc.add_wheel(Wheel::new(Vec3::new(-1.0, -0.3, 1.5), 0.6, 35000.0, 4000.0, 0.3));
    vc.add_wheel(Wheel::new(Vec3::new(1.0, -0.3, 1.5), 0.6, 35000.0, 4000.0, 0.3));
    vc.add_wheel(Wheel::new(Vec3::new(-1.0, -0.3, -1.2), 0.6, 35000.0, 4000.0, 0.3).with_drive());
    vc.add_wheel(Wheel::new(Vec3::new(1.0, -0.3, -1.2), 0.6, 35000.0, 4000.0, 0.3).with_drive());
    world.add_component(entity, vc);

    // Görsel: GLTF model varsa yükle, yoksa küp placeholder
    if let Some(path) = model_path {
        let mut asset_manager = gizmo::renderer::asset::AssetManager::new();
        if let Ok(asset) = asset_manager.load_gltf_scene(
            &renderer.device,
            &renderer.queue,
            &renderer.scene.texture_bind_group_layout,
            tbind.clone(),
            path,
        ) {
            let def_mat = gizmo::prelude::Material::new(tbind.clone()).with_pbr(Vec4::new(color[0], color[1], color[2], color[3]), 0.5, 0.1);
            let _children = crate::scene_setup::spawn_gltf_hierarchy(world, &asset.roots, Some(entity.id()), def_mat);
        } else {
            eprintln!("[Race] Araç modeli yüklenemedi: {}", path);
            // Fallback: küp
            world.add_component(entity, gizmo::renderer::asset::AssetManager::create_cube(&renderer.device));
            world.add_component(entity, gizmo::prelude::Material::new(tbind.clone()).with_pbr(Vec4::new(color[0], color[1], color[2], color[3]), 0.5, 0.1));
            world.add_component(entity, gizmo::renderer::components::MeshRenderer::new());
        }
    } else {
        // Küp placeholder
        world.add_component(entity, gizmo::renderer::asset::AssetManager::create_cube(&renderer.device));
        world.add_component(entity, gizmo::prelude::Material::new(tbind.clone()).with_pbr(Vec4::new(color[0], color[1], color[2], color[3]), 0.5, 0.1));
        world.add_component(entity, gizmo::renderer::components::MeshRenderer::new());
    }

    entity
}

/// Yarış sahnesini kur
pub fn setup_race_scene(
    world: &mut World,
    renderer: &gizmo::renderer::Renderer,
    config: TrackConfig,
) -> (u32, RaceState) {
    // Fizik konfigürasyonu
    world.insert_resource(gizmo::physics::components::PhysicsConfig {
        ground_y: config.ground_y,
    });
    world.insert_resource(gizmo::physics::JointWorld::new());
    world.insert_resource(gizmo::physics::system::PhysicsSolverState::new());

    let mut asset_manager = gizmo::renderer::asset::AssetManager::new();
    let base_tbind = asset_manager.create_white_texture(&renderer.device, &renderer.queue, &renderer.scene.texture_bind_group_layout);

    // Pist modeli yükle (varsa)
    if let Some(ref track_path) = config.track_model {
        if let Ok(asset) = asset_manager.load_gltf_scene(
            &renderer.device,
            &renderer.queue,
            &renderer.scene.texture_bind_group_layout,
            base_tbind.clone(),
            track_path,
        ) {
            let def_mat = gizmo::prelude::Material::new(base_tbind.clone()).with_pbr(Vec4::new(0.5, 0.5, 0.5, 1.0), 0.8, 0.0);
            let root_t = Transform::new(Vec3::ZERO);
            crate::scene_setup::spawn_gltf_asset(world, &asset, renderer, def_mat, root_t);
            println!("[Race] Pist modeli yüklendi: {}", track_path);
        } else {
            eprintln!("[Race] Pist modeli yüklenemedi: {}. Prosedürel pist kullanılıyor.", track_path);
            build_fallback_track(world, renderer, base_tbind.clone());
        }
    } else {
        println!("[Race] Pist modeli belirtilmedi —  prosedürel pist oluşturuluyor.");
        build_fallback_track(world, renderer, base_tbind.clone());
    }

    // Zemin düzlemi (her durumda)
    let ground = world.spawn();
    world.add_component(ground, Transform::new(Vec3::new(0.0, config.ground_y, 0.0)).with_scale(Vec3::new(200.0, 1.0, 200.0)));
    world.add_component(ground, RigidBody::new_static());
    world.add_component(ground, Collider::new_aabb(200.0, 0.5, 200.0));
    world.add_component(ground, gizmo::prelude::Material::new(base_tbind.clone()).with_pbr(Vec4::new(0.2, 0.5, 0.15, 1.0), 0.8, 0.1));
    world.add_component(ground, gizmo::renderer::asset::AssetManager::create_cube(&renderer.device));
    world.add_component(ground, gizmo::renderer::components::MeshRenderer::new());

    // Güneş
    let sun = world.spawn();
    world.add_component(sun, Transform::new(Vec3::new(0.0, 50.0, 50.0))
        .with_rotation(Quat::from_axis_angle(Vec3::new(1.0, 0.5, 0.0).normalize(), -std::f32::consts::FRAC_PI_4)));
    world.add_component(sun, gizmo::renderer::components::DirectionalLight::new(Vec3::new(1.0, 0.95, 0.85), 2.5, true));

    // Oyuncu aracı
    let player = spawn_vehicle(
        world, renderer,
        config.grid_positions[0],
        config.grid_rotations[0],
        config.car_colors[0],
        &config.car_models.get(0).cloned().flatten(),
        base_tbind.clone(),
    );
    // Bağımsız Chase Kamera (Sürekli oyuncuyu takip edecek)
    let camera_entity = world.spawn();
    world.add_component(camera_entity, Transform::new(config.grid_positions[0] + Vec3::new(0.0, 5.0, -10.0)));
    world.add_component(camera_entity, Camera {
        fov: 75.0_f32.to_radians(),
        near: 0.1,
        far: 1000.0,
        yaw: 0.0,
        pitch: -0.15,
        primary: true,
    });

    // AI araçları
    let ai_count = config.ai_speeds.len().min(config.grid_positions.len() - 1);
    let mut ai_entities = Vec::new();
    for i in 0..ai_count {
        let ai_entity = spawn_vehicle(
            world, renderer,
            config.grid_positions[i + 1],
            config.grid_rotations[i + 1],
            config.car_colors.get(i + 1).copied().unwrap_or([0.5, 0.5, 0.5, 1.0]),
            &config.car_models.get(i + 1).cloned().flatten(),
            base_tbind.clone(),
        );
        world.add_component(ai_entity, RaceAI::new(config.waypoints.clone(), config.ai_speeds[i]));
        ai_entities.push(ai_entity.id());
    }

    let checkpoint_count = config.waypoints.len();

    let race_state = RaceState {
        phase: RacePhase::Countdown,
        countdown_timer: 4.0,
        total_laps: config.total_laps,
        player_entity: player.id(),
        camera_entity: camera_entity.id(),
        ai_entities,
        race_timer: 0.0,
        finish_order: Vec::new(),
        player_laps: 0,
        player_last_checkpoint: 0,
        checkpoint_count,
        player_checkpoints_passed: 0,
    };

    (player.id(), race_state)
}

/// Model yokken basit AABB kutulardan oval pist oluştur (fallback / prototip)
fn build_fallback_track(world: &mut World, renderer: &gizmo::renderer::Renderer, tbind: std::sync::Arc<wgpu::BindGroup>) {
    let road_color = [0.3, 0.3, 0.35, 1.0];
    let wall_color = [0.7, 0.1, 0.1, 1.0];
    let road_half_w = 6.0;
    let wall_h = 2.0;
    let wall_thick = 0.5;
    let straight_len = 40.0;
    let curve_offset = 30.0;

    // Yapısal veriler: (pozisyon, half_extents, renk)
    let blocks: Vec<(Vec3, Vec3, [f32; 4])> = vec![
        // Düz yol 1
        (Vec3::new(0.0, 0.01, 0.0), Vec3::new(road_half_w, 0.1, straight_len), road_color),
        // Düz yol 2
        (Vec3::new(curve_offset, 0.01, 0.0), Vec3::new(road_half_w, 0.1, straight_len), road_color),
        // Viraj 1 (kuzey)
        (Vec3::new(curve_offset / 2.0, 0.01, straight_len + road_half_w), Vec3::new(curve_offset / 2.0 + road_half_w, 0.1, road_half_w), road_color),
        // Viraj 2 (güney)
        (Vec3::new(curve_offset / 2.0, 0.01, -straight_len - road_half_w), Vec3::new(curve_offset / 2.0 + road_half_w, 0.1, road_half_w), road_color),
        // Sol duvar yol 1
        (Vec3::new(-road_half_w - wall_thick, wall_h / 2.0, 0.0), Vec3::new(wall_thick, wall_h / 2.0, straight_len + road_half_w), wall_color),
        // Sağ duvar yol 2
        (Vec3::new(curve_offset + road_half_w + wall_thick, wall_h / 2.0, 0.0), Vec3::new(wall_thick, wall_h / 2.0, straight_len + road_half_w), wall_color),
        // Kuzey dış duvar
        (Vec3::new(curve_offset / 2.0, wall_h / 2.0, straight_len + road_half_w * 2.0 + wall_thick), Vec3::new(curve_offset / 2.0 + road_half_w + wall_thick, wall_h / 2.0, wall_thick), wall_color),
        // Güney dış duvar
        (Vec3::new(curve_offset / 2.0, wall_h / 2.0, -straight_len - road_half_w * 2.0 - wall_thick), Vec3::new(curve_offset / 2.0 + road_half_w + wall_thick, wall_h / 2.0, wall_thick), wall_color),
    ];

    for (pos, he, color) in &blocks {
        let e = world.spawn();
        world.add_component(e, Transform::new(*pos));
        world.add_component(e, RigidBody::new_static());
        world.add_component(e, Collider::new_aabb(he.x, he.y, he.z));
        world.add_component(e, gizmo::prelude::Material::new(tbind.clone()).with_pbr(Vec4::new(color[0], color[1], color[2], color[3]), 0.8, 0.0));
        world.add_component(e, gizmo::renderer::asset::AssetManager::create_cube(&renderer.device));
        world.add_component(e, gizmo::renderer::components::MeshRenderer::new());
    }
}

/// Yarış akışını güncelle (her frame çağrılır)
pub fn update_race(world: &mut World, race: &mut RaceState, dt: f32) {
    match race.phase {
        RacePhase::Countdown => {
            race.countdown_timer -= dt;
            if race.countdown_timer <= 0.0 {
                race.phase = RacePhase::Racing;
                race.race_timer = 0.0;
            }
            // Geri sayım sırasında araçları kilitle
            if let Some(mut vehicles) = world.borrow_mut::<VehicleController>() {
                for e in vehicles.entity_dense.clone() {
                    if let Some(vc) = vehicles.get_mut(e) {
                        vc.engine_force = 0.0;
                        vc.brake_force = 10000.0;
                    }
                }
            }
        }
        RacePhase::Racing => {
            race.race_timer += dt;

            // Oyuncu checkpoint tespiti
            if let Some(transforms) = world.borrow::<Transform>() {
                if let Some(pt) = transforms.get(race.player_entity) {
                    let pos = pt.position;
                    if let Some(ais) = world.borrow::<RaceAI>() {
                        if let Some(first_ai) = race.ai_entities.first() {
                            if let Some(ai) = ais.get(*first_ai) {
                                if !ai.waypoints.is_empty() {
                                    let next_cp = race.player_last_checkpoint % ai.waypoints.len();
                                    let wp = ai.waypoints[next_cp];
                                    let dist = Vec3::new(pos.x - wp.x, 0.0, pos.z - wp.z).length();
                                    if dist < 8.0 {
                                        race.player_last_checkpoint += 1;
                                        race.player_checkpoints_passed += 1;
                                        if race.player_last_checkpoint >= race.checkpoint_count {
                                            race.player_last_checkpoint = 0;
                                            race.player_laps += 1;
                                            if race.player_laps >= race.total_laps {
                                                race.finish_order.push((race.player_entity, race.race_timer));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // AI bitiş kontrolü
            if let Some(ais) = world.borrow::<RaceAI>() {
                for &ai_id in &race.ai_entities {
                    if let Some(ai) = ais.get(ai_id) {
                        if ai.laps_completed >= race.total_laps
                            && !race.finish_order.iter().any(|(id, _)| *id == ai_id)
                        {
                            race.finish_order.push((ai_id, race.race_timer));
                        }
                    }
                }
            }

            let all_count = 1 + race.ai_entities.len();
            if race.finish_order.len() >= all_count
                || race.finish_order.iter().any(|(id, _)| *id == race.player_entity)
            {
                race.phase = RacePhase::Finished;
            }
        }
        RacePhase::Finished => {}
    }
}

/// Yarış sıralamasını hesapla
pub fn get_player_position(race: &RaceState, world: &World) -> u32 {
    let player_score = race.player_checkpoints_passed;
    let mut position = 1_u32;
    if let Some(ais) = world.borrow::<gizmo::physics::RaceAI>() {
        for &ai_id in &race.ai_entities {
            let ai: Option<&gizmo::physics::RaceAI> = ais.get(ai_id);
            if let Some(ai) = ai {
                if ai.total_wp_passed > player_score {
                    position += 1;
                }
            }
        }
    }
    position
}

/// Geri sayım metnini döndür
pub fn countdown_text(race: &RaceState) -> &str {
    if race.phase != RacePhase::Countdown { return ""; }
    let t = race.countdown_timer;
    if t > 3.0 { "" }
    else if t > 2.0 { "3" }
    else if t > 1.0 { "2" }
    else if t > 0.0 { "1" }
    else { "GO!" }
}

/// Hız hesapla (km/h)
pub fn get_speed_kmh(world: &World, entity: u32) -> f32 {
    if let Some(vels) = world.borrow::<gizmo::physics::Velocity>() {
        let v: Option<&gizmo::physics::Velocity> = vels.get(entity);
        if let Some(v) = v {
            return v.linear.length() * 3.6;
        }
    }
    0.0
}
