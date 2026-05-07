use gizmo::prelude::*;
use gizmo::physics::components::{Collider, RigidBody, Transform, Velocity, CharacterController};
use gizmo::physics::world::PhysicsWorld;
use gizmo::renderer::asset::AssetManager;
use gizmo::renderer::components::{Camera, Material, MeshRenderer, DirectionalLight, LightRole};
use std::f32::consts::PI;
use gizmo::winit::keyboard::KeyCode;

#[derive(Clone)]
struct MovingPlatform {
    pub start_pos: Vec3,
    pub end_pos: Vec3,
    pub speed: f32,
    pub going_to_end: bool,
}
gizmo::core::impl_component!(MovingPlatform);

#[derive(Clone)]
struct DeathZone;
gizmo::core::impl_component!(DeathZone);

struct PlatformerState {
    player: gizmo::core::Entity,
    camera_yaw: f32,
    camera_pitch: f32,
    respawn_point: Vec3,
}

fn setup(world: &mut World, renderer: &Renderer) -> PlatformerState {
    println!("🏃 3D PLATFORMER DEMO BAŞLIYOR 🏃");
    
    let mut asset_manager = AssetManager::new();
    let phys_world = PhysicsWorld::new().with_gravity(Vec3::new(0.0, -15.0, 0.0));
    world.insert_resource(phys_world);

    // --- MATERIALS & MESHES ---
    let cube_mesh = AssetManager::create_cube(&renderer.device);
    let sphere_mesh = AssetManager::create_sphere(&renderer.device, 0.5, 16, 16);
    
    let tex = asset_manager.create_checkerboard_texture(&renderer.device, &renderer.queue, &renderer.scene.texture_bind_group_layout);
    
    let ground_mat = Material::new(tex.clone()).with_pbr(Vec4::new(0.3, 0.8, 0.3, 1.0), 0.8, 0.0);
    let platform_mat = Material::new(tex.clone()).with_pbr(Vec4::new(0.8, 0.3, 0.8, 1.0), 0.5, 0.0);
    let lava_mat = Material::new(tex.clone()).with_pbr(Vec4::new(1.0, 0.2, 0.0, 1.0), 0.9, 0.0);
    let player_mat = Material::new(tex.clone()).with_pbr(Vec4::new(0.2, 0.4, 1.0, 1.0), 0.2, 0.0);

    // --- LEVEL DESIGN ---
    // Start Platform
    let start_plat = world.spawn();
    world.add_component(start_plat, Transform::new(Vec3::new(0.0, 0.0, 0.0)).with_scale(Vec3::new(5.0, 1.0, 5.0)));
    world.add_component(start_plat, cube_mesh.clone());
    world.add_component(start_plat, ground_mat.clone());
    world.add_component(start_plat, MeshRenderer::new());
    world.add_component(start_plat, Collider::box_collider(Vec3::new(2.5, 0.5, 2.5)));
    world.add_component(start_plat, RigidBody::new_static());

    // Moving Platform 1
    let move_plat1 = world.spawn();
    world.add_component(move_plat1, Transform::new(Vec3::new(0.0, 0.0, 10.0)).with_scale(Vec3::new(3.0, 1.0, 3.0)));
    world.add_component(move_plat1, cube_mesh.clone());
    world.add_component(move_plat1, platform_mat.clone());
    world.add_component(move_plat1, MeshRenderer::new());
    world.add_component(move_plat1, Collider::box_collider(Vec3::new(1.5, 0.5, 1.5)));
    world.add_component(move_plat1, RigidBody::new_kinematic());
    world.add_component(move_plat1, Velocity::default());
    world.add_component(move_plat1, MovingPlatform {
        start_pos: Vec3::new(0.0, 0.0, 10.0),
        end_pos: Vec3::new(15.0, 0.0, 10.0),
        speed: 5.0,
        going_to_end: true,
    });

    // Safe Zone
    let safe_plat = world.spawn();
    world.add_component(safe_plat, Transform::new(Vec3::new(20.0, 2.0, 10.0)).with_scale(Vec3::new(4.0, 1.0, 4.0)));
    world.add_component(safe_plat, cube_mesh.clone());
    world.add_component(safe_plat, ground_mat.clone());
    world.add_component(safe_plat, MeshRenderer::new());
    world.add_component(safe_plat, Collider::box_collider(Vec3::new(2.0, 0.5, 2.0)));
    world.add_component(safe_plat, RigidBody::new_static());

    // Lava (Death Zone)
    let lava = world.spawn();
    world.add_component(lava, Transform::new(Vec3::new(10.0, -5.0, 5.0)).with_scale(Vec3::new(50.0, 1.0, 50.0)));
    world.add_component(lava, cube_mesh.clone());
    world.add_component(lava, lava_mat.clone());
    world.add_component(lava, MeshRenderer::new());
    world.add_component(lava, Collider::box_collider(Vec3::new(25.0, 0.5, 25.0)));
    // We make lava a static trigger (if trigger API exists, else just static that kills on touch via height check)
    world.add_component(lava, RigidBody::new_static());
    world.add_component(lava, DeathZone);

    // --- PLAYER ---
    let respawn = Vec3::new(0.0, 3.0, 0.0);
    let player = world.spawn();
    world.add_component(player, Transform::new(respawn));
    world.add_component(player, sphere_mesh.clone());
    world.add_component(player, player_mat);
    world.add_component(player, MeshRenderer::new());
    world.add_component(player, Collider::capsule(0.5, 0.5));
    world.add_component(player, RigidBody::new_kinematic());
    world.add_component(player, Velocity::default());
    
    let mut kcc = CharacterController::default();
    kcc.speed = 8.0;
    kcc.jump_speed = 12.0;
    world.add_component(player, kcc);

    // --- CAMERA ---
    let camera_ent = world.spawn();
    world.add_component(camera_ent, Transform::new(Vec3::new(0.0, 5.0, -10.0)));
    world.add_component(
        camera_ent,
        Camera::new(std::f32::consts::FRAC_PI_3, 0.1, 1500.0, 0.0, -PI / 8.0, true),
    );

    // --- SUN ---
    let sun = world.spawn();
    world.add_component(sun, Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_x(-PI / 4.0)));
    world.add_component(sun, DirectionalLight::new(
        Vec3::new(1.0, 0.9, 0.9), 3.0, LightRole::Sun
    ));

    PlatformerState {
        player,
        camera_yaw: 0.0,
        camera_pitch: -PI / 8.0,
        respawn_point: respawn,
    }
}

fn update(world: &mut World, state: &mut PlatformerState, dt: f32, input: &gizmo::core::input::Input) {
    // 1. Hareketli Platformlar
    for (_id, (mut p_trans, mut p_vel, mut plat)) in world.query::<(gizmo::core::query::Mut<Transform>, gizmo::core::query::Mut<Velocity>, gizmo::core::query::Mut<MovingPlatform>)>().unwrap().iter_mut() {
        let target = if plat.going_to_end { plat.end_pos } else { plat.start_pos };
        let dir = target - p_trans.position;
        let dist = dir.length();
        
        if dist < 0.1 {
            plat.going_to_end = !plat.going_to_end;
            p_vel.linear = Vec3::ZERO;
        } else {
            p_vel.linear = dir.normalize() * plat.speed;
            p_trans.position += p_vel.linear * dt; // Kinematik objeler için manuel pozisyon güncellemesi gerekebilir
        }
    }

    // 2. Kamera Kontrolleri
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

    let mut player_pos = Vec3::ZERO;

    // 3. Karakter Kontrolü
    if let Some(kcc) = world.borrow_mut::<CharacterController>().get_mut(state.player.id()) {
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

    // Death Check
    if let Some(t) = world.borrow_mut::<Transform>().get_mut(state.player.id()) {
        player_pos = t.position;
        if t.position.y < -3.0 { // Lav yüksekliği
            println!("💀 ÖLDÜN! Respawn oluyorsun...");
            t.position = state.respawn_point;
            if let Some(v) = world.borrow_mut::<Velocity>().get_mut(state.player.id()) {
                v.linear = Vec3::ZERO;
            }
        }
    }

    // 4. Kamerayı Oyuncuya Takip Ettir
    let cam_pos = player_pos - forward * 8.0 + Vec3::new(0.0, 2.0, 0.0);
    if let Some(mut q) = world.query::<(gizmo::core::query::Mut<Transform>, gizmo::core::query::Mut<Camera>)>() {
        for (_, (mut trans, mut cam)) in q.iter_mut() {
            trans.position = cam_pos;
            trans.rotation = Quat::from_rotation_y(-state.camera_yaw);
            cam.yaw = state.camera_yaw;
            cam.pitch = state.camera_pitch;
        }
    }
}

fn render(
    world: &mut World,
    _state: &PlatformerState,
    encoder: &mut gizmo::wgpu::CommandEncoder,
    view: &gizmo::wgpu::TextureView,
    renderer: &mut Renderer,
    _light_time: f32,
) {
    gizmo::systems::render::default_render_pass(world, encoder, view, renderer);
}

fn main() {
    gizmo::app::App::<PlatformerState>::new("Gizmo Engine - 3D Platformer", 1280, 720)
        .set_setup(setup)
        .set_update(update)
        .set_render(render)
        .run();
}
