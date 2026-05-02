use gizmo::prelude::*;
use gizmo::physics::components::{Collider, RigidBody, Transform, Velocity};
use gizmo::physics::world::PhysicsWorld;
use gizmo::renderer::asset::AssetManager;
use gizmo::renderer::components::{Camera, Material, MeshRenderer};
use std::f32::consts::PI;

struct VehicleState {
    _camera_speed: f32,
    camera_pitch: f32,
    camera_yaw: f32,
    camera_pos: Vec3,
    time: f32,
    
    // Vehicle
    car_entity: gizmo::core::Entity,
    wheel_entities: [gizmo::core::Entity; 4],
    _wheel_local_pos: [Vec3; 4],
    
    _suspension_rest: f32,
    _suspension_stiff: f32,
    _suspension_damp: f32,
    _wheel_radius: f32,
}

fn setup(world: &mut World, renderer: &Renderer) -> VehicleState {
    let mut asset_manager = AssetManager::new();
    let phys_world = PhysicsWorld::new();
    
    // --- KURU ZEMİN (DRY TERRAIN) ---
    let ground_mesh = AssetManager::create_cube(&renderer.device);
    let ground_tex = asset_manager.create_checkerboard_texture(&renderer.device, &renderer.queue, &renderer.scene.texture_bind_group_layout);
    let ground_mat = Material::new(ground_tex.clone()).with_pbr(Vec4::new(0.6, 0.6, 0.6, 1.0), 0.8, 0.1);
    
    let ground = world.spawn();
    world.add_component(ground, Transform::new(Vec3::new(0.0, -1.0, 0.0)).with_scale(Vec3::new(200.0, 1.0, 200.0)));
    world.add_component(ground, ground_mesh.clone());
    world.add_component(ground, ground_mat.clone());
    world.add_component(ground, MeshRenderer::new());
    world.add_component(ground, Collider::box_collider(Vec3::new(200.0, 1.0, 200.0)));
    world.add_component(ground, RigidBody::new(0.0, 0.1, 0.8, false));

    // --- ATLAMA RAMPASI (JUMP RAMP) ---
    let ramp = world.spawn();
    let mut ramp_trans = Transform::new(Vec3::new(0.0, 2.0, -30.0)).with_scale(Vec3::new(8.0, 0.5, 15.0));
    ramp_trans.rotation = Quat::from_rotation_x(PI / 8.0); // Hafif eğim
    world.add_component(ramp, ramp_trans);
    world.add_component(ramp, ground_mesh.clone());
    world.add_component(ramp, ground_mat.clone());
    world.add_component(ramp, MeshRenderer::new());
    world.add_component(ramp, Collider::box_collider(Vec3::new(8.0, 0.5, 15.0)));
    world.add_component(ramp, RigidBody::new(0.0, 0.1, 0.8, false));

    // --- FİZİKSEL ARAÇ (RAYCAST VEHICLE) ---
    let car_ent = world.spawn();
    let car_w = 1.0;
    let car_h = 0.4;
    let car_l = 2.0;
    
    world.add_component(car_ent, Transform::new(Vec3::new(0.0, 3.0, 0.0)).with_scale(Vec3::new(car_w, car_h, car_l)));
    world.add_component(car_ent, ground_mesh.clone());
    let car_mat = Material::new(ground_tex.clone()).with_pbr(Vec4::new(0.9, 0.1, 0.1, 1.0), 0.3, 0.5); // Kırmızı spor araba rengi
    world.add_component(car_ent, car_mat);
    world.add_component(car_ent, MeshRenderer::new());
    world.add_component(car_ent, Collider::box_collider(Vec3::new(car_w, car_h, car_l)));
    // Araç kütlesi yüksek olmalı ki süspansiyonları esnetsin
    world.add_component(car_ent, RigidBody::new(800.0, 0.1, 0.5, true)); 
    world.add_component(car_ent, Velocity::default());

    // Tekerlek pozisyonları (Görsel Tekerlekler)
    let wheel_mesh = AssetManager::create_sphere(&renderer.device, 1.0, 16, 16);
    let wheel_mat = Material::new(ground_tex.clone()).with_pbr(Vec4::new(0.1, 0.1, 0.1, 1.0), 0.9, 0.0);
    
    let mut wheel_entities = [car_ent; 4];
    let wheel_local_pos = [
        Vec3::new(car_w + 0.2, -0.2, car_l - 0.5),   // Ön Sol
        Vec3::new(-car_w - 0.2, -0.2, car_l - 0.5),  // Ön Sağ
        Vec3::new(car_w + 0.2, -0.2, -car_l + 0.5),  // Arka Sol
        Vec3::new(-car_w - 0.2, -0.2, -car_l + 0.5), // Arka Sağ
    ];

    let wheel_radius = 0.4;

    for i in 0..4 {
        let w = world.spawn();
        world.add_component(w, Transform::new(wheel_local_pos[i]).with_scale(Vec3::splat(wheel_radius)));
        world.add_component(w, wheel_mesh.clone());
        world.add_component(w, wheel_mat.clone());
        world.add_component(w, MeshRenderer::new());
        // Tekerlekler collider/rigidbody'e sahip değil, kasaya bağlı raycast ile yönetilecekler
        wheel_entities[i] = w;
    }

    // YENİ ECS ARAÇ DENETLEYİCİSİNİ EKLİYORUZ
    let mut vehicle = gizmo::physics::vehicle::VehicleController::new();
    for i in 0..4 {
        vehicle.add_wheel(gizmo::physics::vehicle::Wheel {
            attachment_local_pos: wheel_local_pos[i],
            radius: wheel_radius,
            suspension_rest_length: 0.6,
            suspension_stiffness: 30000.0,
            suspension_damping: 2500.0,
            friction_slip: 10.5,
            ..Default::default()
        });
    }
    world.add_component(car_ent, vehicle);

    // --- KUTU DUVARI (BOX WALL TO SMASH) ---
    for x in -2..3 {
        for y in 0..4 {
            let b = world.spawn();
            world.add_component(b, Transform::new(Vec3::new((x as f32) * 1.1, 0.5 + (y as f32) * 1.1, -45.0)).with_scale(Vec3::splat(0.5)));
            world.add_component(b, ground_mesh.clone());
            let b_mat = Material::new(ground_tex.clone()).with_pbr(Vec4::new(0.8, 0.6, 0.2, 1.0), 0.8, 0.0);
            world.add_component(b, b_mat);
            world.add_component(b, MeshRenderer::new());
            world.add_component(b, Collider::box_collider(Vec3::splat(0.5)));
            world.add_component(b, RigidBody::new(20.0, 0.1, 0.8, true));
            world.add_component(b, Velocity::default());
        }
    }

    // --- GÜNEŞ ---
    let sun = world.spawn();
    world.add_component(sun, Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_x(-PI / 4.0)));
    world.add_component(sun, gizmo::renderer::components::DirectionalLight::new(
        Vec3::new(1.0, 0.95, 0.9), 4.0, gizmo::renderer::components::LightRole::Sun
    ));

    // --- KAMERA ---
    let camera_ent = world.spawn();
    world.add_component(
        camera_ent,
        Transform::new(Vec3::new(0.0, 8.0, 15.0)).with_rotation(Quat::from_rotation_x(-0.3)),
    );
    world.add_component(
        camera_ent,
        Camera::new(
            std::f32::consts::FRAC_PI_3,
            0.1,
            5000.0,
            0.0,
            -0.3,
            true,
        ),
    );

    world.insert_resource(phys_world);
    world.insert_resource(asset_manager);

    VehicleState {
        _camera_speed: 20.0,
        camera_pitch: -0.3,
        camera_yaw: 0.0,
        camera_pos: Vec3::new(0.0, 8.0, 15.0),
        time: 0.0,
        car_entity: car_ent,
        wheel_entities,
        _wheel_local_pos: wheel_local_pos,
        _suspension_rest: 0.6,
        _suspension_stiff: 30000.0, // Araç ağır olduğu için (800kg) sert yay
        _suspension_damp: 2500.0,
        _wheel_radius: wheel_radius,
    }
}

fn update(world: &mut World, state: &mut VehicleState, dt: f32, input: &gizmo::core::input::Input) {
    state.time += dt;

    // --- ARAÇ GİRDİLERİ ---
    let mut engine_force = 0.0;
    let mut steering_angle = 0.0;
    let mut brake_force = 0.0;

    if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::KeyI as u32) { engine_force = 8000.0; } // İleri (I)
    if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::KeyK as u32) { brake_force = 5000.0; } // Fren/Geri (K)
    if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::KeyJ as u32) { steering_angle = 0.5; }  // Sol (J)
    if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::KeyL as u32) { steering_angle = -0.5; } // Sağ (L)

    // ECS VehicleController'a girdileri aktar
    if let Some(q_v) = world.query::<gizmo::core::query::Mut<gizmo::physics::vehicle::VehicleController>>() {
        if let Some(mut vehicle) = q_v.get(state.car_entity.id()) {
            for (i, wheel) in vehicle.wheels.iter_mut().enumerate() {
                let is_front = i < 2;
                if is_front {
                    wheel.steering_angle = steering_angle;
                } else {
                    wheel.engine_force = engine_force;
                }
                wheel.brake_force = brake_force;
            }
        }
    }

    // CPU Physics (VehicleController içeride çalışıp kuvvetleri uygulayacak)
    gizmo::systems::cpu_physics_step_system(world, dt);

    // Kasa durumunu oku
    let mut car_pos = Vec3::ZERO;
    let mut car_rot = Quat::IDENTITY;
    if let Some(q) = world.query::<gizmo::core::query::Mut<Transform>>() {
        if let Some(t) = q.get(state.car_entity.id()) {
            car_pos = t.position;
            car_rot = t.rotation;
        }
    }

    // Tekerlek görsellerini araca göre güncelle
    let mut wheel_positions = [Vec3::ZERO; 4];
    let mut wheel_rotations = [Quat::IDENTITY; 4];
    
    if let Some(q_v) = world.query::<gizmo::core::query::Mut<gizmo::physics::vehicle::VehicleController>>() {
        if let Some(vehicle) = q_v.get(state.car_entity.id()) {
            for i in 0..4 {
                let wheel = &vehicle.wheels[i];
                let anchor_world = car_pos + car_rot.mul_vec3(wheel.attachment_local_pos);
                let up = car_rot.mul_vec3(Vec3::new(0.0, 1.0, 0.0));
                wheel_positions[i] = anchor_world - up * wheel.suspension_length;
                
                let steer_rot = Quat::from_rotation_y(wheel.steering_angle);
                let spin_rot = Quat::from_rotation_x(wheel.rotation_angle);
                wheel_rotations[i] = car_rot * steer_rot * spin_rot;
            }
        }
    }

    if let Some(q) = world.query::<gizmo::core::query::Mut<Transform>>() {
        for i in 0..4 {
            if let Some(mut wt) = q.get(state.wheel_entities[i].id()) {
                wt.set_position(wheel_positions[i]);
                wt.set_rotation(wheel_rotations[i]);
            }
        }
    }

    // --- KAMERA TAKİBİ ---
    if input.is_mouse_button_pressed(1) {
        let delta = input.mouse_delta();
        state.camera_yaw -= delta.0 * 0.005;
        state.camera_pitch -= delta.1 * 0.005;
        state.camera_pitch = state.camera_pitch.clamp(-PI / 2.0 + 0.1, PI / 2.0 - 0.1);
    }
    
    // Kamerayı arabaya sabitle (Third-person)
    let rot = Quat::from_rotation_y(state.camera_yaw) * Quat::from_rotation_x(state.camera_pitch);
    let offset = rot.mul_vec3(Vec3::new(0.0, 0.0, -10.0)); // Arkasında dur
    state.camera_pos = car_pos + Vec3::new(0.0, 4.0, 0.0) + offset;

    if let Some(mut q) = world.query::<(gizmo::core::query::Mut<Transform>, gizmo::core::query::Mut<Camera>)>() {
        for (_, (mut trans, mut cam)) in q.iter_mut() {
            trans.position = state.camera_pos;
            trans.rotation = rot;
            cam.yaw = state.camera_yaw;
            cam.pitch = state.camera_pitch;
        }
    }

    // -------------------------
}

fn render(
    world: &mut World,
    _state: &VehicleState,
    encoder: &mut gizmo::wgpu::CommandEncoder,
    view: &gizmo::wgpu::TextureView,
    renderer: &mut Renderer,
    _light_time: f32,
) {
    renderer.gpu_physics = None;
    gizmo::systems::default_render_pass(world, encoder, view, renderer);
}

fn main() {
    App::<VehicleState>::new("Gizmo Engine - Raycast Vehicle Sandbox", 1280, 720)
        .set_setup(setup)
        .set_update(update)
        .set_render(render)
        .run();
}
