use gizmo::prelude::*;
use gizmo::physics::components::{RigidBody, Velocity};
use gizmo::physics::shape::Collider;
use gizmo::physics::vehicle::{VehicleController, Wheel};
use crate::state::{GameState, GizmoMode};

pub fn setup_car_scene(world: &mut World, renderer: &gizmo::renderer::renderer::Renderer) -> GameState {
    println!("Gizmo Engine: Süspansiyon ve Arazi Sahnesi başlatılıyor...");

    let audio = gizmo::audio::AudioManager::new();
    let asset_manager = gizmo::renderer::asset::AssetManager::new();

    let tbind_asphalt = gizmo::renderer::asset::AssetManager::new().load_material_texture(
         &renderer.device, &renderer.queue, &renderer.texture_bind_group_layout,
         "demo/assets/stone_tiles.jpg"
    ).expect("texture bulunamadi!");

    let bouncing_box_id = world.spawn().id(); 

    // --- FİZİKLİ ZEMİN (Asfalt, Ice) ---
    // Ana Zemin (Friction 0.9)
    let ground_mesh = gizmo::renderer::asset::AssetManager::create_plane(&renderer.device, 500.0);
    let ground_entity = world.spawn();
    world.add_component(ground_entity, ground_mesh.clone());
    world.add_component(ground_entity, Transform::new(Vec3::new(0.0, -1.0, 0.0)).with_scale(Vec3::new(500.0, 1.0, 500.0)));
    world.add_component(ground_entity, Material::new(tbind_asphalt.clone()).with_pbr(Vec4::new(0.2, 0.2, 0.2, 1.0), 0.9, 0.1));
    world.add_component(ground_entity, gizmo::renderer::components::MeshRenderer::new());
    let mut ground_rb = RigidBody::new_static();
    ground_rb.friction = 1.0;
    world.add_component(ground_entity, ground_rb);
    world.add_component(ground_entity, Collider::new_aabb(250.0, 0.05, 250.0));
    
    // Rampa (Tümsek 1)
    let ramp1 = world.spawn();
    let ramp1_mesh = gizmo::renderer::asset::AssetManager::create_cube(&renderer.device);
    world.add_component(ramp1, ramp1_mesh.clone());
    world.add_component(ramp1, Transform::new(Vec3::new(0.0, 0.0, 30.0))
        .with_scale(Vec3::new(10.0, 1.0, 10.0))
        .with_rotation(Quat::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), std::f32::consts::PI / 12.0))); // Hafif eğim
    world.add_component(ramp1, Material::new(tbind_asphalt.clone()).with_unlit(Vec4::new(0.8, 0.4, 0.1, 1.0)));
    world.add_component(ramp1, gizmo::renderer::components::MeshRenderer::new());
    let mut ramp_rb = RigidBody::new_static(); ramp_rb.friction = 1.0;
    world.add_component(ramp1, ramp_rb);
    // collider half extents is half of scale
    world.add_component(ramp1, Collider::new_aabb(5.0, 0.5, 5.0));

    // Buzlu Alan (Friction 0.05)
    let ice_field = world.spawn();
    world.add_component(ice_field, ramp1_mesh.clone());
    world.add_component(ice_field, Transform::new(Vec3::new(0.0, -0.9, 80.0))
        .with_scale(Vec3::new(40.0, 0.1, 80.0)));
    world.add_component(ice_field, Material::new(tbind_asphalt.clone()).with_unlit(Vec4::new(0.6, 0.8, 1.0, 0.8))); 
    world.add_component(ice_field, gizmo::renderer::components::MeshRenderer::new());
    let mut ice_rb = RigidBody::new_static(); ice_rb.friction = 0.05; // ÇOK KAYGAN
    world.add_component(ice_field, ice_rb);
    world.add_component(ice_field, Collider::new_aabb(20.0, 0.05, 40.0));

    // Tümsek Serisi (Ağırlık Transferi zorlanması)
    for i in 0..5 {
        let bump = world.spawn();
        world.add_component(bump, ramp1_mesh.clone());
        world.add_component(bump, Transform::new(Vec3::new(0.0, -0.7, 140.0 + (i as f32 * 8.0)))
            .with_scale(Vec3::new(20.0, 0.6, 2.0)));
        world.add_component(bump, Material::new(tbind_asphalt.clone()).with_unlit(Vec4::new(0.9, 0.9, 0.2, 1.0))); 
        world.add_component(bump, gizmo::renderer::components::MeshRenderer::new());
        let mut bump_rb = RigidBody::new_static(); bump_rb.friction = 0.8;
        world.add_component(bump, bump_rb);
        world.add_component(bump, Collider::new_aabb(10.0, 0.3, 1.0));
    }

    // --- GÜNEŞ VE GÖKYÜZÜ ---
    let sun = world.spawn();
    world.add_component(sun, Transform::new(Vec3::new(0.0, 50.0, 50.0)).with_rotation(Quat::from_axis_angle(Vec3::new(1.0, 0.5, 0.0).normalize(), -std::f32::consts::FRAC_PI_4)));
    world.add_component(sun, gizmo::renderer::components::DirectionalLight::new(Vec3::new(1.0, 1.0, 0.9), 1.5, true));
    world.add_component(sun, EntityName("Güneş".into()));

    let skybox = world.spawn();
    world.add_component(skybox, Transform::new(Vec3::ZERO).with_scale(Vec3::new(500.0, 500.0, 500.0)));
    world.add_component(skybox, gizmo::renderer::asset::AssetManager::create_inverted_cube(&renderer.device));
    world.add_component(skybox, Material::new(tbind_asphalt.clone()).with_unlit(Vec4::new(0.4, 0.6, 0.9, 1.0)));
    world.add_component(skybox, gizmo::renderer::components::MeshRenderer::new());
    world.add_component(skybox, EntityName("Skybox".into()));

    world.insert_resource(gizmo::physics::JointWorld::new());
    world.insert_resource(gizmo::physics::system::PhysicsSolverState::new());

    // --- PLAYER (KAMERA) ---
    let player = world.spawn();
    let player_id = player.id();
    world.add_component(player, Transform::new(Vec3::new(0.0, 5.0, -10.0)));
    world.add_component(player, Camera::new(std::f32::consts::FRAC_PI_4, 0.1, 2000.0, -std::f32::consts::FRAC_PI_2, -0.3, true));
    world.add_component(player, EntityName("Kamera".into()));

    // --- ARKA TEKERLEK ÇEKİŞLİ ARAÇ ---
    let car = world.spawn();
    world.add_component(car, EntityName("Araç (Raycast)".into()));
    world.add_component(car, gizmo::renderer::asset::AssetManager::create_cube(&renderer.device));
    world.add_component(car, Material::new(tbind_asphalt.clone()).with_unlit(Vec4::new(0.8, 0.1, 0.1, 1.0))); 
    world.add_component(car, gizmo::renderer::components::MeshRenderer::new());
    world.add_component(car, Transform::new(Vec3::new(0.0, 5.0, 0.0)).with_scale(Vec3::new(1.8, 0.6, 4.0))); 
    let mut car_rb = RigidBody::new(1400.0, 0.1, 0.5, true);
    car_rb.ccd_enabled = true;
    world.add_component(car, car_rb);
    world.add_component(car, Velocity::new(Vec3::ZERO));
    world.add_component(car, Collider::new_aabb(0.9, 0.3, 2.0)); 

    // VehicleController Ekleme (Raycast Süspansiyon)
    let rest_len = 0.5;
    let stiffness = 60000.0;
    let damping = 4500.0;
    let wheel_radius = 0.4;
    let mut vehicle = VehicleController::new();
    
    // Sağ Ön (1)
    vehicle.add_wheel(Wheel::new(Vec3::new(0.95, -0.3, 1.6), rest_len, stiffness, damping, wheel_radius));
    // Sol Ön (2)
    vehicle.add_wheel(Wheel::new(Vec3::new(-0.95, -0.3, 1.6), rest_len, stiffness, damping, wheel_radius));
    // Sağ Arka (3)
    vehicle.add_wheel(Wheel::new(Vec3::new(0.95, -0.3, -1.6), rest_len, stiffness, damping, wheel_radius).with_drive());
    // Sol Arka (4)
    vehicle.add_wheel(Wheel::new(Vec3::new(-0.95, -0.3, -1.6), rest_len, stiffness, damping, wheel_radius).with_drive());
    
    world.add_component(car, vehicle);

    // Kaba Taslak Gizmo Okları (Bypass)
    let gizmo_x = world.spawn().id();
    let gizmo_y = world.spawn().id();
    let gizmo_z = world.spawn().id();

    world.insert_resource(gizmo::core::event::Events::<crate::state::SpawnDominoEvent>::new());
    world.insert_resource(gizmo::core::event::Events::<crate::state::ReleaseDominoEvent>::new());
    world.insert_resource(gizmo::core::event::Events::<crate::state::TextureLoadEvent>::new());
    world.insert_resource(gizmo::core::event::Events::<crate::state::AssetSpawnEvent>::new());
    world.insert_resource(gizmo::core::event::Events::<crate::state::ShaderReloadEvent>::new());
    world.insert_resource(gizmo::core::event::Events::<crate::state::SelectionEvent>::new());
    world.insert_resource(crate::state::DominoAppState { active_ball_id: None });
    world.insert_resource(crate::state::PachinkoSpawnerState { timer: 0.0, count: 0 });
    world.insert_resource(asset_manager);
    
    if let Ok(engine) = gizmo::scripting::ScriptEngine::new() {
        world.insert_resource(engine);
    }
    
    world.insert_resource(gizmo::renderer::renderer::PostProcessUniforms {
        bloom_intensity: 0.8, bloom_threshold: 0.8,
        chromatic_aberration: 1.0, exposure: 1.2, vignette_intensity: 0.5,
        _padding: [0.0; 3],
    });
    
    world.insert_resource(gizmo::editor::EditorState::new());

    GameState {
        bouncing_box_id, player_id, skybox_id: skybox.id(), inspector_selected_entity: None,
        audio, do_raycast: false, gizmo_x, gizmo_y, gizmo_z, dragging_axis: None,
        drag_start_t: 0.0, drag_original_pos: Vec3::ZERO, drag_original_scale: Vec3::ONE,
        drag_original_rot: Quat::IDENTITY, current_fps: 60.0,
        gizmo_mode: GizmoMode::Translate,
        egui_wants_pointer: false,
        asset_watcher: gizmo::renderer::hot_reload::AssetWatcher::new(&["demo/assets"]),
        physics_accumulator: 0.0,
        target_physics_fps: 240.0,
        sphere_prefab_id: bouncing_box_id,
        cube_prefab_id: bouncing_box_id,
        checkpoints: Vec::new(),
        race_status: crate::state::RaceStatus::Idle,
        race_timer: 0.0,
        active_dialogue: None,
        active_cutscene: None,
        camera_follow_target: Some(car.id()),
        free_cam: false,
        total_elapsed: 0.0,
    }
}
