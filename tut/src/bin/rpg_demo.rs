use gizmo::prelude::*;
use gizmo::physics::components::{Collider, RigidBody, Transform, Velocity, CharacterController};
use gizmo::physics::world::PhysicsWorld;
use gizmo::renderer::asset::AssetManager;
use gizmo::renderer::components::{Camera, Material, MeshRenderer, LodGroup, LodLevel};
use gizmo::renderer::async_assets::AsyncAssetLoader;
use gizmo::audio::{AudioManager, AudioSource};
use gizmo::ai::components::NavAgent;
use std::f32::consts::PI;
use gizmo::winit::keyboard::KeyCode;
use rand::Rng;

struct RpgState {
    character_entity: gizmo::core::Entity,
    camera_yaw: f32,
    camera_pitch: f32,
}

fn setup(world: &mut World, renderer: &Renderer) -> RpgState {
    println!("⚔️ GIZMO ENGINE RPG TEST BAŞLIYOR ⚔️");
    let mut asset_manager = AssetManager::new();
    let phys_world = PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0));
    world.insert_resource(phys_world);
    world.insert_resource(AsyncAssetLoader::new());


    // --- SKYBOX ---
    let skybox_mesh = AssetManager::create_inverted_cube(&renderer.device);
    let sky_path = if std::path::Path::new("tut/assets/sky.jpg").exists() {
        "tut/assets/sky.jpg"
    } else {
        "assets/sky.jpg"
    };
    let sky_tex = asset_manager.load_material_texture(&renderer.device, &renderer.queue, &renderer.scene.texture_bind_group_layout, sky_path).expect("Failed to load skybox texture");
    let sky_mat = Material::new(sky_tex).with_skybox();
    
    let sky_ent = world.spawn();
    world.add_component(sky_ent, Transform::new(Vec3::ZERO).with_scale(Vec3::splat(2000.0)));
    world.add_component(sky_ent, skybox_mesh);
    world.add_component(sky_ent, sky_mat);
    world.add_component(sky_ent, MeshRenderer::new());

    // --- GROUND (DEVASA AÇIK DÜNYA ZEMİNİ) ---
    let ground_mesh = AssetManager::create_cube(&renderer.device);
    let ground_tex = asset_manager.create_checkerboard_texture(&renderer.device, &renderer.queue, &renderer.scene.texture_bind_group_layout);
    // Asenkron Doku Akışı test edilmesi için Ground texture kaynağı bırakıyoruz
    let ground_mat = Material::new(ground_tex.clone())
        .with_pbr(Vec4::new(0.4, 0.6, 0.3, 1.0), 0.9, 0.0)
        .with_texture_source("assets/textures/grass_high_res.png".to_string());
    
    let ground = world.spawn();
    world.add_component(ground, Transform::new(Vec3::new(0.0, -1.0, 0.0)).with_scale(Vec3::new(500.0, 1.0, 500.0)));
    world.add_component(ground, ground_mesh.clone());
    world.add_component(ground, ground_mat.clone());
    world.add_component(ground, MeshRenderer::new());
    world.add_component(ground, Collider::box_collider(Vec3::new(500.0, 1.0, 500.0)));
    world.add_component(ground, RigidBody::new_static());
    world.add_component(ground, Velocity::default());

    // --- ORMAN (LOD TESTİ) ---
    println!("LOD TESTİ: Orman oluşturuluyor...");
    let high_res_tree = AssetManager::create_sphere(&renderer.device, 1.0, 32, 32);
    let low_res_tree = AssetManager::create_sphere(&renderer.device, 1.0, 8, 8);
    let tree_mat = Material::new(ground_tex.clone()).with_pbr(Vec4::new(0.1, 0.8, 0.1, 1.0), 0.8, 0.0);

    let mut rng = rand::thread_rng();
    for _ in 0..1000 {
        let x = rng.gen_range(-200.0..200.0);
        let z = rng.gen_range(-200.0..200.0);
        
        let tree = world.spawn();
        world.add_component(tree, Transform::new(Vec3::new(x, 0.0, z)).with_scale(Vec3::new(2.0, 5.0, 2.0)));
        world.add_component(tree, low_res_tree.clone());
        world.add_component(tree, tree_mat.clone());
        world.add_component(tree, MeshRenderer::new());
        world.add_component(tree, Collider::capsule(2.0, 5.0));
        world.add_component(tree, RigidBody::new_static());
        world.add_component(tree, Velocity::default());

        // LOD (Level of Detail) Swapping System Entegrasyonu
        let lod_group = LodGroup::new(vec![
            LodLevel::new(high_res_tree.clone(), 30.0), // 30 metreye kadar Yüksek Kalite
            LodLevel::new(low_res_tree.clone(), 150.0), // 150 metreye kadar Düşük Kalite (Proxy)
            // 150 metreden sonrası görünmez olacak (Culling)
        ]);
        world.add_component(tree, lod_group);
    }

    // --- YAPAY ZEKA NPCLER ---
    println!("YAPAY ZEKA: Köylüler spawn oluyor...");
    for i in 0..5 {
        let npc = world.spawn();
        let npc_mesh = AssetManager::create_sphere(&renderer.device, 0.5, 16, 16);
        let npc_mat = Material::new(ground_tex.clone()).with_pbr(Vec4::new(1.0, 0.2, 0.2, 1.0), 0.5, 0.0);
        
        world.add_component(npc, Transform::new(Vec3::new(10.0 + i as f32 * 5.0, 1.0, 10.0)));
        world.add_component(npc, npc_mesh);
        world.add_component(npc, npc_mat);
        world.add_component(npc, MeshRenderer::new());
        world.add_component(npc, Collider::capsule(0.5, 0.5));
        world.add_component(npc, RigidBody::new_kinematic());
        world.add_component(npc, Velocity::default());
        world.add_component(npc, CharacterController::default());
        world.add_component(npc, NavAgent::default()); // AI sistemi tarafından yürütülecek
        
        // 3D Spatial Ses Denemesi
        let mut audio = AudioSource::new("assets/sounds/villager_hum.wav");
        audio.is_3d = true;
        audio.max_distance = 20.0;
        world.add_component(npc, audio);
    }

    // --- ANA KARAKTER (PLAYER) ---
    println!("KARAKTER: Oyuncu yaratılıyor...");
    let char_ent = world.spawn();
    let char_mesh = AssetManager::create_sphere(&renderer.device, 0.5, 16, 16);
    let char_mat = Material::new(ground_tex.clone()).with_pbr(Vec4::new(0.2, 0.2, 1.0, 1.0), 0.5, 0.5);
    
    world.add_component(char_ent, Transform::new(Vec3::new(0.0, 2.0, 0.0)));
    world.add_component(char_ent, char_mesh);
    world.add_component(char_ent, char_mat);
    world.add_component(char_ent, MeshRenderer::new());
    
    let mut kcc = CharacterController::default();
    kcc.speed = 10.0;
    kcc.jump_speed = 8.0;
    kcc.step_height = 0.5;
    
    world.add_component(char_ent, kcc);
    world.add_component(char_ent, Collider::capsule(0.5, 0.5));
    world.add_component(char_ent, RigidBody::new_kinematic());
    world.add_component(char_ent, Velocity::default());

    // --- KAMERA ---
    let camera_ent = world.spawn();
    world.add_component(camera_ent, Transform::new(Vec3::new(0.0, 5.0, 10.0)));
    world.add_component(
        camera_ent,
        Camera::new(std::f32::consts::FRAC_PI_3, 0.1, 1500.0, 0.0, -PI / 8.0, true),
    );

    // --- GÜNEŞ (DIRECTIONAL LIGHT) ---
    let sun = world.spawn();
    world.add_component(sun, Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_x(-PI / 4.0)));
    world.add_component(sun, gizmo::renderer::components::DirectionalLight::new(
        Vec3::new(1.0, 0.95, 0.9), 5.0, gizmo::renderer::components::LightRole::Sun
    ));

    println!("✅ SETUP: Tamamlandı!");
    RpgState {
        character_entity: char_ent,
        camera_yaw: 0.0,
        camera_pitch: -PI / 8.0,
    }
}

fn update(world: &mut World, state: &mut RpgState, dt: f32, input: &gizmo::core::input::Input) {
    // --- KAMERA KONTROLLERİ ---
    if input.is_mouse_button_pressed(1) {
        let delta = input.mouse_delta();
        state.camera_yaw -= delta.0 * 0.005;
        state.camera_pitch -= delta.1 * 0.005;
        state.camera_pitch = state.camera_pitch.clamp(-PI / 2.0 + 0.1, PI / 2.0 - 0.1);
    }
    
    let fx = state.camera_yaw.cos() * state.camera_pitch.cos();
    let fy = state.camera_pitch.sin();
    let fz = state.camera_yaw.sin() * state.camera_pitch.cos();
    let forward = Vec3::new(fx, fy, fz).normalize_or_zero();
    let right = Vec3::new(-state.camera_yaw.sin(), 0.0, state.camera_yaw.cos()).normalize_or_zero();
    
    let mut move_forward = forward; move_forward.y = 0.0; move_forward = move_forward.normalize_or_zero();
    let mut move_right = right; move_right.y = 0.0; move_right = move_right.normalize_or_zero();
    let cam_rot = Quat::from_rotation_y(-state.camera_yaw);

    // --- KARAKTER HAREKETİ ---
    let mut char_pos = Vec3::ZERO;
    if let Some(kcc) = world.borrow_mut::<CharacterController>().get_mut(state.character_entity.id()) {
        let mut move_dir = Vec3::ZERO;
        if input.is_key_pressed(KeyCode::KeyW as u32) { move_dir += move_forward; }
        if input.is_key_pressed(KeyCode::KeyS as u32) { move_dir -= move_forward; }
        if input.is_key_pressed(KeyCode::KeyD as u32) { move_dir += move_right; }
        if input.is_key_pressed(KeyCode::KeyA as u32) { move_dir -= move_right; }
        
        move_dir = move_dir.normalize_or_zero();
        kcc.target_velocity = move_dir * kcc.speed;
        
        if input.is_key_pressed(KeyCode::Space as u32) {
            kcc.jump_buffer_timer = kcc.jump_buffer_time;
        }
    }
    
    // --- FİZİK MOTORU VE ASENKRON DOKU AKIŞI (STREAMING) ADIMI ---
    let mut physics_dt = dt.min(0.1);
    while physics_dt > 0.0 {
        let step = physics_dt.min(0.016);
        gizmo::physics::system::physics_step_system(world, step);
        physics_dt -= step;
    }

    // Kamera pozisyonuna göre Doku Akış (Texture Streaming) Sistemi Çalıştırılır
    {
        if let Some(trans) = world.borrow::<Transform>().get(state.character_entity.id()) {
            char_pos = trans.position;
        }
    }
    gizmo::systems::texture_streaming_system(world, char_pos);
    
    if let Some(trans) = world.borrow_mut::<Transform>().get_mut(state.character_entity.id()) {
        trans.rotation = Quat::from_rotation_y(state.camera_yaw);
    }
    
    // TPS/FPS Kamera Takibi
    let cam_pos = char_pos + Vec3::new(0.0, 1.5, 0.0);
    if let Some(mut q) = world.query::<(gizmo::core::query::Mut<Transform>, gizmo::core::query::Mut<Camera>)>() {
        for (_, (mut trans, mut cam)) in q.iter_mut() {
            trans.position = cam_pos;
            trans.rotation = cam_rot;
            cam.yaw = state.camera_yaw;
            cam.pitch = state.camera_pitch;
        }
    }
}

fn render(
    world: &mut World,
    _state: &RpgState,
    encoder: &mut gizmo::wgpu::CommandEncoder,
    view: &gizmo::wgpu::TextureView,
    renderer: &mut Renderer,
    _light_time: f32,
) {
    renderer.gpu_physics = None;
    gizmo::systems::default_render_pass(world, encoder, view, renderer);
}

fn main() {
    App::<RpgState>::new("Gizmo Engine - Open World RPG Demo", 1600, 900)
        .set_setup(setup)
        .set_update(update)
        .set_render(render)
        .run();
}
