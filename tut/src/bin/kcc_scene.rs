use gizmo::prelude::*;
use gizmo::physics::components::{Collider, RigidBody, Transform, Velocity, CharacterController};
use gizmo::physics::world::PhysicsWorld;
use gizmo::renderer::asset::AssetManager;
use gizmo::renderer::components::{Camera, Material, MeshRenderer};
use std::f32::consts::PI;
use gizmo::winit::keyboard::KeyCode;

struct KccState {
    character_entity: gizmo::core::Entity,
    camera_yaw: f32,
    camera_pitch: f32,
}

fn setup(world: &mut World, renderer: &Renderer) -> KccState {
    let mut asset_manager = AssetManager::new();
    let _phys_world = PhysicsWorld::new();
    
    // --- SKYBOX ---
    let skybox_mesh = AssetManager::create_inverted_cube(&renderer.device);
    let sky_tex = asset_manager.load_material_texture(&renderer.device, &renderer.queue, &renderer.scene.texture_bind_group_layout, "assets/sky.jpg").unwrap();
    let sky_mat = Material::new(sky_tex).with_skybox();
    
    let sky_ent = world.spawn();
    world.add_component(sky_ent, Transform::new(Vec3::ZERO).with_scale(Vec3::splat(2000.0)));
    world.add_component(sky_ent, skybox_mesh);
    world.add_component(sky_ent, sky_mat);
    world.add_component(sky_ent, MeshRenderer::new());
    
    // --- GROUND ---
    let ground_mesh = AssetManager::create_cube(&renderer.device);
    let ground_tex = asset_manager.create_checkerboard_texture(&renderer.device, &renderer.queue, &renderer.scene.texture_bind_group_layout);
    let ground_mat = Material::new(ground_tex.clone()).with_pbr(Vec4::new(0.6, 0.6, 0.6, 1.0), 0.8, 0.1);
    
    let ground = world.spawn();
    world.add_component(ground, Transform::new(Vec3::new(0.0, -1.0, 0.0)).with_scale(Vec3::new(100.0, 1.0, 100.0)));
    world.add_component(ground, ground_mesh.clone());
    world.add_component(ground, ground_mat.clone());
    world.add_component(ground, MeshRenderer::new());
    world.add_component(ground, Collider::box_collider(Vec3::new(100.0, 1.0, 100.0)));
    world.add_component(ground, RigidBody::new_static());

    // --- STAIRS (To test step_height) ---
    for i in 0..5 {
        let step = world.spawn();
        let step_h = 0.2 * (i as f32 + 1.0);
        let step_pos = Vec3::new(5.0 + i as f32, step_h - 1.0, 0.0);
        world.add_component(step, Transform::new(step_pos).with_scale(Vec3::new(1.0, step_h, 4.0)));
        world.add_component(step, ground_mesh.clone());
        world.add_component(step, ground_mat.clone());
        world.add_component(step, MeshRenderer::new());
        world.add_component(step, Collider::box_collider(Vec3::new(1.0, step_h, 4.0)));
        world.add_component(step, RigidBody::new_static());
    }

    // --- SLOPE (To test max_slope_angle) ---
    let slope = world.spawn();
    let mut slope_trans = Transform::new(Vec3::new(0.0, 1.0, 10.0)).with_scale(Vec3::new(5.0, 0.5, 10.0));
    slope_trans.rotation = Quat::from_rotation_x(PI / 6.0); // 30 degrees (walkable)
    world.add_component(slope, slope_trans);
    world.add_component(slope, ground_mesh.clone());
    world.add_component(slope, ground_mat.clone());
    world.add_component(slope, MeshRenderer::new());
    world.add_component(slope, Collider::box_collider(Vec3::new(5.0, 0.5, 10.0)));
    world.add_component(slope, RigidBody::new_static());

    let steep_slope = world.spawn();
    let mut steep_trans = Transform::new(Vec3::new(10.0, 2.0, 10.0)).with_scale(Vec3::new(5.0, 0.5, 10.0));
    steep_trans.rotation = Quat::from_rotation_x(PI / 3.0); // 60 degrees (unwalkable, should slide)
    world.add_component(steep_slope, steep_trans);
    world.add_component(steep_slope, ground_mesh.clone());
    world.add_component(steep_slope, ground_mat.clone());
    world.add_component(steep_slope, MeshRenderer::new());
    world.add_component(steep_slope, Collider::box_collider(Vec3::new(5.0, 0.5, 10.0)));
    world.add_component(steep_slope, RigidBody::new_static());

    // --- CHARACTER CONTROLLER ---
    let char_ent = world.spawn();
    let char_mesh = AssetManager::create_sphere(&renderer.device, 0.5, 16, 16);
    let char_mat = Material::new(ground_tex.clone()).with_pbr(Vec4::new(0.1, 0.8, 0.2, 1.0), 0.5, 0.5);
    
    world.add_component(char_ent, Transform::new(Vec3::new(0.0, 2.0, 0.0)));
    world.add_component(char_ent, char_mesh);
    world.add_component(char_ent, char_mat);
    world.add_component(char_ent, MeshRenderer::new());
    
    let mut kcc = CharacterController::default();
    kcc.speed = 8.0;
    kcc.jump_speed = 6.0;
    kcc.height = 2.0;
    kcc.radius = 0.5;
    kcc.step_height = 0.3;
    
    world.add_component(char_ent, kcc);
    
    // The character is kinematic, meaning physics forces don't affect it directly
    // but the KCC system will move it.
    world.add_component(char_ent, Collider::capsule(0.5, 0.5));
    world.add_component(char_ent, RigidBody::new_kinematic());
    world.add_component(char_ent, Velocity::default());

    // --- CAMERA ---
    let camera_ent = world.spawn();
    world.add_component(camera_ent, Transform::new(Vec3::new(0.0, 5.0, 10.0)));
    world.add_component(
        camera_ent,
        Camera::new(
            std::f32::consts::FRAC_PI_3,
            0.1,
            1000.0,
            0.0,
            -PI / 8.0,
            true,
        ),
    );

    // --- SUN ---
    let sun = world.spawn();
    world.add_component(sun, Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_x(-PI / 4.0)));
    world.add_component(sun, gizmo::renderer::components::DirectionalLight::new(
        Vec3::new(1.0, 0.95, 0.9), 4.0, gizmo::renderer::components::LightRole::Sun
    ));

    KccState {
        character_entity: char_ent,
        camera_yaw: 0.0,
        camera_pitch: -PI / 8.0,
    }
}

fn update(world: &mut World, state: &mut KccState, _dt: f32, input: &gizmo::core::input::Input) {
    // --- CAMERA CONTROLS ---
    if input.is_mouse_button_pressed(1) {
        let delta = input.mouse_delta();
        state.camera_yaw -= delta.0 * 0.005;
        state.camera_pitch -= delta.1 * 0.005;
        state.camera_pitch = state.camera_pitch.clamp(-PI / 2.0 + 0.1, PI / 2.0 - 0.1);
    }
    
    let cam_rot = Quat::from_euler(gizmo::math::EulerRot::YXZ, state.camera_yaw, state.camera_pitch, 0.0);
    let forward = cam_rot * Vec3::new(0.0, 0.0, -1.0);
    let right = cam_rot * Vec3::new(1.0, 0.0, 0.0);
    
    let mut move_forward = forward;
    move_forward.y = 0.0;
    move_forward = move_forward.normalize_or_zero();
    
    let mut move_right = right;
    move_right.y = 0.0;
    move_right = move_right.normalize_or_zero();

    // --- CHARACTER CONTROL ---
    let mut char_pos = Vec3::ZERO;
    
    if let Some(kcc) = world.borrow_mut::<CharacterController>().get_mut(state.character_entity.id()) {
        let mut move_dir = Vec3::ZERO;
        if input.is_key_pressed(KeyCode::KeyW as u32) { move_dir += move_forward; }
        if input.is_key_pressed(KeyCode::KeyS as u32) { move_dir -= move_forward; }
        if input.is_key_pressed(KeyCode::KeyD as u32) { move_dir += move_right; }
        if input.is_key_pressed(KeyCode::KeyA as u32) { move_dir -= move_right; }
        
        move_dir = move_dir.normalize_or_zero();
        kcc.target_velocity = move_dir * kcc.speed;
        
        if input.is_key_pressed(KeyCode::Space as u32) && kcc.is_grounded {
            kcc.velocity.y = kcc.jump_speed;
            kcc.is_grounded = false;
        }
    }
    
    if let Some(trans) = world.borrow::<Transform>().get(state.character_entity.id()) {
        char_pos = trans.position;
    }
    
    let cam_pos = char_pos - forward * 8.0 + Vec3::new(0.0, 2.0, 0.0);
    
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
    _state: &KccState,
    encoder: &mut gizmo::wgpu::CommandEncoder,
    view: &gizmo::wgpu::TextureView,
    renderer: &mut Renderer,
    _light_time: f32,
) {
    renderer.gpu_physics = None;
    gizmo::default_systems::default_render_pass(world, encoder, view, renderer);
}

fn main() {
    App::<KccState>::new("Gizmo Engine - KCC Sandbox", 1280, 720)
        .set_setup(setup)
        .set_update(update)
        .set_render(render)
        .run();
}
