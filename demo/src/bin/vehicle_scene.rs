//! # RAYCAST ARAÇ SANDBOX — fizik-araç kum havuzu (temiz sürüm)
//!
//! Kırmızı bir spor araba; rampadan atla, kutu duvarını dağıt. Bu sürüm motorun
//! yüksek-seviye kurulum idiomlarını kullanır ama demonun DAVRANIŞINI birebir korur.
//! NEYİN motora, NEYİN demoya ait olduğu konusunda dürüst olalım:
//!   * **`Prefab` + `auto_box_collider`** — KUTU DUVARI 20 özdeş bloktan tek blueprint'le;
//!     collider `Transform.scale`'den OTOMATİK türetilir (boyut bir kez). Eski sürümdeki
//!     `spawn()` + elle `RigidBody::new` + `update_inertia_from_collider` + `Velocity` zinciri
//!     kalktı.
//!   * **`spawn_bundle` + `RigidBodyBundle`** — zemin/rampa/kasa TEK-TÜK nesneler (Prefab değil):
//!     her biri kendi collider'ıyla `RigidBodyBundle` içinde. `dynamic(mass)` ataleti collider'dan
//!     KENDİ türetir (elle `update_inertia_from_collider` gerekmez); `static_body()` immovable
//!     zemin/rampa için.
//!   * **ELLE SABİT-ADIM fiziği KORUNDU** — bu demo `PhysicsPlugin` KAYDETMEZ; kendi
//!     `PhysicsWorld` kaynağını sürer. Her sabit adımda ÖNCE `vehicle_controller_system`
//!     (Pacejka lastik + süspansiyon kuvvetlerini Velocity'ye yazar) SONRA `cpu_physics_step_system`
//!     çalışmalı — bu SIRA plugin'e devredilemez, o yüzden accumulator döngüsü OLDUĞU GİBİ kaldı.
//!     Ömür schedule'ı YOK + geçici/uçan varlık YOK → `DespawnAfter/BelowY` BURAYA UYGULANMAZ.
//!   * **Sahne render = `default_render_pass` DOĞRUDAN** — `with_scene_render()` tek-satır kısayolu
//!     SSR/SSGI/volumetric/TAA'yı kapatırdı; deferred boru hattını açık tutmak için çıplak çağrı.
//!
//! ## Kontroller
//!   * **I** — gaz · **K** — geri · **SPACE** — fren · **J / L** — direksiyon sol/sağ
//!   * **Sağ-tık + fare** — kamerayı döndür (third-person) · **R** — anlık fizik durumunu konsola bas

use gizmo::egui;
use gizmo::physics::vehicle::{Axle, VehicleController, Wheel};
use gizmo::physics::vehicle_controller_system;
use gizmo::physics::world::PhysicsWorld;
use gizmo::prelude::*;
use gizmo::systems::{cpu_physics_step_system, default_render_pass};
use std::f32::consts::{FRAC_PI_3, PI};

struct VehicleState {
    camera_pitch: f32,
    camera_yaw: f32,
    camera_pos: Vec3,
    phys_accum: f32,

    // Araç
    car_entity: Entity,
    wheel_entities: [Entity; 4],
}

fn setup(world: &mut World, renderer: &Renderer) -> VehicleState {
    let mut asset_manager = AssetManager::new();
    let phys_world = PhysicsWorld::new();

    // Paylaşılan mesh + doku + zemin materyali (kasa/tekerlek/kutu aynı damalı dokuyu kullanır).
    let cube_mesh = AssetManager::create_cube(&renderer.device);
    let ground_tex = asset_manager.create_checkerboard_texture(
        &renderer.device,
        &renderer.queue,
        &renderer.scene.texture_bind_group_layout,
    );
    let ground_mat =
        Material::new(ground_tex.clone()).with_pbr(Vec4::new(0.6, 0.6, 0.6, 1.0), 0.8, 0.1);

    // --- KURU ZEMİN (DRY TERRAIN) ---
    world.spawn_bundle((
        Transform::new(Vec3::new(0.0, -1.0, 0.0)).with_scale(Vec3::new(2000.0, 1.0, 2000.0)),
        cube_mesh.clone(),
        ground_mat.clone(),
        MeshRenderer::new(),
        RigidBodyBundle::static_body()
            .with_collider(Collider::box_collider(Vec3::new(2000.0, 1.0, 2000.0))),
    ));

    // --- ATLAMA RAMPASI (JUMP RAMP) ---
    world.spawn_bundle((
        Transform::new(Vec3::new(0.0, 2.0, -30.0))
            .with_scale(Vec3::new(8.0, 0.5, 15.0))
            .with_rotation(Quat::from_rotation_x(PI / 8.0)), // Hafif eğim
        cube_mesh.clone(),
        ground_mat.clone(),
        MeshRenderer::new(),
        RigidBodyBundle::static_body()
            .with_collider(Collider::box_collider(Vec3::new(8.0, 0.5, 15.0))),
    ));

    // --- FİZİKSEL ARAÇ (RAYCAST VEHICLE) ---
    // Tek-tük dinamik gövde → spawn_bundle + explicit collider. `dynamic(800)` ataleti
    // kutu collider'dan otomatik türetir (eskiden elle update_inertia_from_collider).
    let car_w = 1.0;
    let car_h = 0.4;
    let car_l = 2.0;
    let car_mat =
        Material::new(ground_tex.clone()).with_pbr(Vec4::new(0.9, 0.1, 0.1, 1.0), 0.3, 0.5); // Kırmızı spor araba rengi
    let car_ent = world.spawn_bundle((
        Transform::new(Vec3::new(0.0, 3.0, 0.0)).with_scale(Vec3::new(car_w, car_h, car_l)),
        cube_mesh.clone(),
        car_mat,
        MeshRenderer::new(),
        RigidBodyBundle::dynamic(800.0)
            .with_collider(Collider::box_collider(Vec3::new(car_w, car_h, car_l))),
    ));

    // Tekerlek görselleri (collider/rigidbody YOK; kasaya bağlı raycast ile sürülürler).
    let wheel_mesh = AssetManager::create_sphere(&renderer.device, 1.0, 16, 16);
    let wheel_mat =
        Material::new(ground_tex.clone()).with_pbr(Vec4::new(0.1, 0.1, 0.1, 1.0), 0.9, 0.0);
    let wheel_radius = 0.4;
    let wheel_local_pos = [
        Vec3::new(car_w + 0.2, -0.2, car_l - 0.5),   // Ön Sol
        Vec3::new(-car_w - 0.2, -0.2, car_l - 0.5),  // Ön Sağ
        Vec3::new(car_w + 0.2, -0.2, -car_l + 0.5),  // Arka Sol
        Vec3::new(-car_w - 0.2, -0.2, -car_l + 0.5), // Arka Sağ
    ];

    let mut wheel_entities = [car_ent; 4];
    for i in 0..4 {
        wheel_entities[i] = world.spawn_bundle((
            Transform::new(wheel_local_pos[i]).with_scale(Vec3::splat(wheel_radius)),
            wheel_mesh.clone(),
            wheel_mat.clone(),
            MeshRenderer::new(),
        ));
    }

    // ECS ARAÇ DENETLEYİCİSİ (Pacejka lastik + süspansiyon) kasaya eklenir.
    let mut vehicle = VehicleController::new();
    for (i, &local_pos) in wheel_local_pos.iter().enumerate() {
        let axle_type = if i < 2 { Axle::Front } else { Axle::Rear };
        let is_left = i % 2 == 0; // 0, 2 = Sol; 1, 3 = Sağ

        vehicle.add_wheel(Wheel {
            attachment_local_pos: local_pos,
            radius: wheel_radius,
            axle_type,
            is_left,
            suspension_rest_length: 0.6,
            suspension_stiffness: 30000.0, // Araç ağır olduğu için (800kg) sert yay
            suspension_damping: 2500.0,
            ..Default::default()
        });
    }
    world.add_component(car_ent, vehicle);

    // --- KUTU DUVARI (BOX WALL TO SMASH) ---
    // 20 özdeş dinamik blok → tek Prefab; collider Transform.scale'den otomatik.
    let box_prefab = Prefab::new(
        cube_mesh.clone(),
        Material::new(ground_tex.clone()).with_pbr(Vec4::new(0.8, 0.6, 0.2, 1.0), 0.8, 0.0),
    )
    .with_body(RigidBodyBundle::dynamic(20.0))
    .auto_box_collider();
    for x in -2..3 {
        for y in 0..4 {
            box_prefab.spawn(
                world,
                Transform::new(Vec3::new(x as f32 * 1.1, 0.5 + y as f32 * 1.1, -45.0))
                    .with_scale(Vec3::splat(0.5)),
            );
        }
    }

    // --- GÜNEŞ ---
    world.spawn_bundle((
        Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_x(-PI / 4.0)),
        DirectionalLight::new(Vec3::new(1.0, 0.95, 0.9), 4.0, LightRole::Sun),
    ));

    // --- KAMERA ---
    world.spawn_bundle((
        Transform::new(Vec3::new(0.0, 8.0, 15.0)).with_rotation(Quat::from_rotation_x(-0.3)),
        Camera::new(FRAC_PI_3, 0.1, 5000.0, 0.0, -0.3, true),
    ));

    world.insert_resource(phys_world);
    world.insert_resource(asset_manager);

    VehicleState {
        camera_pitch: -0.3,
        camera_yaw: 0.0,
        camera_pos: Vec3::new(0.0, 8.0, 15.0),
        phys_accum: 0.0,
        car_entity: car_ent,
        wheel_entities,
    }
}

fn update(world: &mut World, state: &mut VehicleState, dt: f32, input: &Input) {
    // --- ARAÇ GİRDİLERİ (basılı-tut = sürekli; kenar-tespiti gerekmez) ---
    let throttle = if input.is_key_pressed(KeyCode::KeyI as u32) {
        1.0_f32
    } else {
        0.0
    };
    let reverse = input.is_key_pressed(KeyCode::KeyK as u32);
    let brake = if input.is_key_pressed(KeyCode::Space as u32) {
        1.0_f32
    } else {
        0.0
    };
    let steering = if input.is_key_pressed(KeyCode::KeyJ as u32) {
        1.0_f32
    } else if input.is_key_pressed(KeyCode::KeyL as u32) {
        -1.0
    } else {
        0.0
    };

    // ECS VehicleController'a girdileri aktar
    if let Some(mut q_v) = world.query_mut::<Mut<VehicleController>>() {
        if let Some(mut vehicle) = q_v.get_mut(state.car_entity.id()) {
            vehicle.throttle_input = throttle;
            vehicle.brake_input = brake;
            vehicle.steering_input = steering;
            vehicle.set_reverse(reverse);
        }
    }

    // GERÇEK sabit zaman adımı (accumulator) — car_demo ile aynı desen. Her sabit adımda ÖNCE
    // `vehicle_controller_system` (Pacejka lastik + süspansiyon kuvvetlerini Velocity'ye yazar)
    // SONRA fizik adımı çalışır; salt `cpu_physics_step_system` VehicleController'ı SÜRMEZ.
    // Sabit dt ayrıca sert süspansiyonun frame-jitter'la titremesini engeller.
    const FIXED_DT: f32 = 1.0 / 240.0;
    state.phys_accum += dt.min(0.1);
    let mut steps = 0;
    while state.phys_accum >= FIXED_DT && steps < 32 {
        vehicle_controller_system(world, FIXED_DT);
        cpu_physics_step_system(world, FIXED_DT);
        state.phys_accum -= FIXED_DT;
        steps += 1;
    }

    // Kasa durumunu oku
    let mut car_pos = Vec3::ZERO;
    let mut car_rot = Quat::IDENTITY;
    if let Some(mut q) = world.query_mut::<Mut<Transform>>() {
        if let Some(t) = q.get_mut(state.car_entity.id()) {
            car_pos = t.position;
            car_rot = t.rotation;
        }
    }

    // Tekerlek görsellerini araca göre güncelle
    let mut wheel_positions = [Vec3::ZERO; 4];
    let mut wheel_rotations = [Quat::IDENTITY; 4];

    if let Some(mut q_v) = world.query_mut::<Mut<VehicleController>>() {
        if let Some(vehicle) = q_v.get_mut(state.car_entity.id()) {
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

    if let Some(mut q) = world.query_mut::<Mut<Transform>>() {
        for i in 0..4 {
            if let Some(mut wt) = q.get_mut(state.wheel_entities[i].id()) {
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

    if let Some(mut q) = world.query_mut::<(Mut<Transform>, Mut<Camera>)>() {
        for (_, (mut trans, mut cam)) in q.iter_mut() {
            trans.position = state.camera_pos;
            trans.rotation = rot;
            cam.yaw = state.camera_yaw;
            cam.pitch = state.camera_pitch;
        }
    }

    // Debug (R basılıyken anlık fizik durumunu konsola basar)
    if input.is_key_pressed(KeyCode::KeyR as u32) {
        if let Some(mut q_v) = world.query_mut::<Mut<VehicleController>>() {
            if let Some(v) = q_v.get_mut(state.car_entity.id()) {
                println!(
                    "[Vehicle] Speed: {:.1} km/h | RPM: {:.0} | Gear: {} | Thr: {:.2} | Rev: {}",
                    v.current_speed_kmh,
                    v.engine_rpm,
                    v.current_gear,
                    v.throttle_input,
                    v.reverse_input
                );
            }
        }
    }
}

fn render(
    world: &mut World,
    _state: &VehicleState,
    encoder: &mut gizmo::wgpu::CommandEncoder,
    view: &gizmo::wgpu::TextureView,
    renderer: &mut Renderer,
    _light_time: f32,
) {
    default_render_pass(world, encoder, view, renderer);
}

fn ui(world: &mut World, state: &mut VehicleState, ctx: &egui::Context) {
    let mut speed_kmh = 0.0;
    let mut rpm = 0.0;
    let mut gear = 0;
    let mut grounded_wheels = 0;
    let mut throttle = 0.0;
    let mut brake = 0.0;
    let hz = 120.0;
    let mut ms_per_phase = 0.0;

    if let Some(phys_world) = world.get_resource::<PhysicsWorld>() {
        ms_per_phase = phys_world.metrics.solver_ms;
    }

    if let Some(mut q_v) = world.query_mut::<Mut<VehicleController>>() {
        if let Some(v) = q_v.get_mut(state.car_entity.id()) {
            speed_kmh = v.current_speed_kmh;
            rpm = v.engine_rpm;
            gear = v.current_gear;
            grounded_wheels = v.wheels.iter().filter(|w| w.is_grounded).count();
            throttle = v.throttle_input;
            brake = v.brake_input;
        }
    }

    egui::Window::new("Fizik HUD (Phase 6.3)").show(ctx, |ui| {
        ui.label(format!("Hız: {:.1} km/h", speed_kmh));
        ui.label(format!("Motor RPM: {:.0}", rpm));
        ui.label(format!("Vites: {}", gear));
        ui.label(format!("Grounded Tekerlek: {}/4", grounded_wheels));
        ui.label(format!("Gaz (Throttle): {:.2}", throttle));
        ui.label(format!("Fren (Brake): {:.2}", brake));
        ui.separator();
        ui.label(format!("Fizik Güncelleme: {:.0} Hz", hz));
        ui.label(format!("Solver Süresi: {:.2} ms", ms_per_phase));
    });
}

fn main() {
    // NOT: `PhysicsPlugin` KAYDEDİLMEZ — fizik `update` içinde elle sürülür (yukarıya bak).
    App::<VehicleState>::new("Gizmo Engine - Raycast Vehicle Sandbox", 1280, 720)
        .set_setup(setup)
        .set_update(update)
        .set_render(render)
        .set_ui(ui)
        .run()
        .expect("uygulama çalıştırılamadı");
}
