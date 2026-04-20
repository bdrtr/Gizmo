use gizmo::prelude::*;
use gizmo::physics::components::{PhysicsConfig, RigidBody, Velocity};
use gizmo::physics::shape::Collider;
use gizmo::physics::system::PhysicsSolverState;
use gizmo::physics::JointWorld;
use gizmo::physics::constraints::Joint;
use gizmo::renderer::asset::AssetManager;
use gizmo::renderer::components::{DirectionalLight, MeshRenderer};

const GROUND_Y: f32 = 0.0;
const BALL_MASS: f32 = 1.0;
const BALL_RADIUS: f32 = 0.5;
const HINGE_HEIGHT: f32 = 5.0;
const BALL_COUNT: usize = 5;

struct RopeData {
    ball_id: u32,
    rope_id: u32,
    pivot: Vec3,
}

struct CradleGame {
    ball_ids: Vec<u32>,
    ropes: Vec<RopeData>,
    cam_id: u32,
    cam_yaw: f32,
    cam_pitch: f32,
    cam_pos: Vec3,
    cam_speed: f32,
    triggered: bool,
    physics_acc: f32,
    physics_dt: f32,
    fps_timer: f32,
    frames: u32,
    fps: f32,
}

impl CradleGame {
    fn new() -> Self {
        Self {
            ball_ids: Vec::new(),
            ropes: Vec::new(),
            cam_id: 0,
            cam_yaw: -std::f32::consts::FRAC_PI_2,
            cam_pitch: -0.2,
            cam_pos: Vec3::new(0.0, 5.0, 15.0),
            cam_speed: 15.0,
            triggered: false,
            physics_acc: 0.0,
            physics_dt: 1.0 / 120.0,
            fps_timer: 0.0,
            frames: 0,
            fps: 0.0,
        }
    }
}

fn main() {
    App::<CradleGame>::new("Gizmo — Newton Sarkacı", 1600, 900)
        .add_event::<gizmo::physics::CollisionEvent>()
        .set_setup(|world, renderer| {
            let mut game = setup_scene(world, renderer);
            
            // Yüksek İterasyon: Newton sarkacı gibi klasik enerji dalgası taşınan simülasyonlarda
            // "hepsi birlikte hareket ediyor" sorununu çözmek ve impulse'un tüm zinciri
            // temiz bir şekilde aşmasını sağlamak için 8 yerine 64 Gaussian-Seidel iterasyonu atıyoruz.
            world.get_resource_mut_or_default::<gizmo::physics::system::PhysicsSolverState>().solver_iterations = 120;
            
            let cam_entity = world.spawn();
            world.add_component(
                cam_entity,
                Transform::new(game.cam_pos)
                    .with_rotation(pitch_yaw_quat(game.cam_pitch, game.cam_yaw)),
            );
            world.add_component(
                cam_entity,
                Camera::new(
                    std::f32::consts::FRAC_PI_3,
                    0.1,
                    500.0,
                    game.cam_yaw,
                    game.cam_pitch,
                    true,
                ),
            );
            world.add_component(cam_entity, EntityName("Kamera".into()));
            
            game.cam_id = cam_entity.id();
            game
        })
        .set_update(|world, state, dt, input| {
            state.fps_timer += dt;
            state.frames += 1;
            if state.fps_timer >= 1.0 {
                state.fps = state.frames as f32 / state.fps_timer;
                state.frames = 0;
                state.fps_timer = 0.0;
            }

            if input.is_key_pressed(KeyCode::Space as u32) && !state.triggered {
                trigger_cradle(world, state);
                state.triggered = true;
            }
            if input.is_key_pressed(KeyCode::KeyR as u32) {
                reset_cradle(world, state);
            }
            
            if input.is_mouse_button_pressed(gizmo::core::input::mouse::RIGHT) {
                let delta = input.mouse_delta();
                let sens = 0.003_f32;
                state.cam_yaw += delta.0 * sens;
                state.cam_pitch -= delta.1 * sens;
                state.cam_pitch = state.cam_pitch.clamp(
                    -std::f32::consts::FRAC_PI_2 + 0.05,
                    std::f32::consts::FRAC_PI_2 - 0.05,
                );
            }

            update_camera(world, state, input, dt);

            state.physics_acc += dt;
            let fixed_dt = state.physics_dt;
            state.physics_acc = state.physics_acc.min(fixed_dt * 8.0);
            
            while state.physics_acc >= fixed_dt {
                step_physics(world, fixed_dt);
                update_ropes(world, state);
                state.physics_acc -= fixed_dt;
            }
        })
        .set_ui(|_world, state, ctx| {
            gizmo::egui::Window::new("FPS")
                .anchor(gizmo::egui::Align2::LEFT_TOP, gizmo::egui::vec2(10.0, 10.0))
                .title_bar(false)
                .resizable(false)
                .frame(gizmo::egui::Frame::window(&ctx.style()).fill(gizmo::egui::Color32::from_black_alpha(150)))
                .show(ctx, |ui| {
                    ui.label(
                        gizmo::egui::RichText::new(format!("FPS: {:.0}", state.fps))
                            .color(gizmo::egui::Color32::WHITE)
                            .strong()
                            .size(24.0),
                    );
                });
        })
        .set_render(|world, _state, encoder, view, renderer, _light_time| {
            gizmo::default_systems::default_render_pass(world, encoder, view, renderer);
        })
        .run();
}

fn setup_scene(world: &mut World, renderer: &gizmo::renderer::Renderer) -> CradleGame {
    println!("Newton Sarkacı kuruluyor: {} top...", BALL_COUNT);

    world.insert_resource(PhysicsConfig {
        ground_y: GROUND_Y,
        max_linear_velocity: 100.0,
        max_angular_velocity: 100.0,
        deterministic_simulation: false,
        ..Default::default()
    });
    world.insert_resource(JointWorld::new());
    world.insert_resource(PhysicsSolverState::new());

    let mut asset_manager = AssetManager::new();

    let tex = asset_manager.create_white_texture(
        &renderer.device,
        &renderer.queue,
        &renderer.scene.texture_bind_group_layout,
    );

    let ground_mesh = AssetManager::create_plane(&renderer.device, 200.0);
    let ground = world.spawn();
    world.add_component(
        ground,
        Transform::new(Vec3::new(0.0, GROUND_Y, 0.0))
            .with_scale(Vec3::new(1.0, 1.0, 1.0)),
    );
    world.add_component(ground, ground_mesh);
    world.add_component(
        ground,
        Material::new(tex.clone()).with_pbr(Vec4::new(0.55, 0.5, 0.45, 1.0), 0.9, 0.0),
    );
    world.add_component(ground, MeshRenderer::new());
    world.add_component(ground, RigidBody::new_static());
    world.add_component(ground, Collider::new_aabb(100.0, 0.05, 100.0));

    let sun = world.spawn();
    world.add_component(
        sun,
        Transform::new(Vec3::new(30.0, 80.0, 40.0)).with_rotation(
            Quat::from_axis_angle(Vec3::new(1.0, 0.3, 0.0).normalize(), -0.8),
        ),
    );
    world.add_component(
        sun,
        DirectionalLight::new(Vec3::new(1.0, 0.97, 0.90), 2.5, true),
    );

    // Ana asma direkleri (Ön ve Arka)
    let beam_mesh = AssetManager::create_cube(&renderer.device);
    let z_offset = 1.5;
    
    let beam_entity_back = world.spawn();
    world.add_component(
        beam_entity_back, 
        Transform::new(Vec3::new(0.0, HINGE_HEIGHT, -z_offset))
            .with_scale(Vec3::new(BALL_COUNT as f32 * BALL_RADIUS + 0.5, 0.1, 0.1))
    );
    world.add_component(beam_entity_back, beam_mesh.clone());
    world.add_component(beam_entity_back, Material::new(tex.clone()).with_pbr(Vec4::new(0.2, 0.2, 0.2, 1.0), 0.5, 0.5));
    world.add_component(beam_entity_back, MeshRenderer::new());
    world.add_component(beam_entity_back, RigidBody::new_static());

    let beam_entity_front = world.spawn();
    world.add_component(
        beam_entity_front, 
        Transform::new(Vec3::new(0.0, HINGE_HEIGHT, z_offset))
            .with_scale(Vec3::new(BALL_COUNT as f32 * BALL_RADIUS + 0.5, 0.1, 0.1))
    );
    world.add_component(beam_entity_front, beam_mesh.clone());
    world.add_component(beam_entity_front, Material::new(tex.clone()).with_pbr(Vec4::new(0.2, 0.2, 0.2, 1.0), 0.5, 0.5));
    world.add_component(beam_entity_front, MeshRenderer::new());
    world.add_component(beam_entity_front, RigidBody::new_static());

    let mut game = CradleGame::new();
    let ball_mesh = AssetManager::create_sphere(&renderer.device, 1.0, 32, 32);

    // Newton Sarkacı momentum problemi ve CCS Çatışması:
    // PGS çözücüde her topun tam anında bağımsız 1.0 restitution verebilmesi için
    // gözle görülmeyen 1 milimetrelik bir boşluğa ihtiyaç vardır.
    // gap=0.015 kullanınca eğrisel sapmadan dolay enerji asimetrisi oldu. gap=0.001 tam idealdir.
    let gap = 0.001_f32;
    let diameter = (BALL_RADIUS * 2.0) + gap;
    let start_x = -((BALL_COUNT as f32 - 1.0) / 2.0) * diameter;
    for i in 0..BALL_COUNT {
        let x = start_x + (i as f32) * diameter;
        let dist_len = 4.0_f32;
        let dy = (dist_len * dist_len - z_offset * z_offset).sqrt();
        let ball_y = HINGE_HEIGHT - dy;

        // Top ekle
        let ball = world.spawn();
        world.add_component(
            ball,
            Transform::new(Vec3::new(x, ball_y, 0.0))
                .with_scale(Vec3::splat(BALL_RADIUS)),
        );
        world.add_component(ball, ball_mesh.clone());
        
        let color = if i == 0 || i == BALL_COUNT - 1 {
            Vec4::new(0.9, 0.1, 0.1, 1.0)
        } else {
            Vec4::new(0.8, 0.8, 0.8, 1.0)
        };
        
        let ball_tex = asset_manager.load_material_texture(
            &renderer.device,
            &renderer.queue,
            &renderer.scene.texture_bind_group_layout,
            "tut/assets/domino_real.png",
        ).unwrap_or_else(|_| tex.clone());
        
        world.add_component(ball, Material::new(ball_tex).with_pbr(color, 1.0, 0.0));
        world.add_component(ball, MeshRenderer::new());

        let mut rb = RigidBody::new(BALL_MASS, 1.0, 0.0, true);
        rb.ccd_enabled = false; // Sürekli çarpışma denetimi (CCD) kısıtlamaları (joints) bozabilir. Bu senaryoda gerek yok.
        rb.calculate_sphere_inertia(BALL_RADIUS);
        world.add_component(ball, rb);
        world.add_component(ball, Velocity::new(Vec3::ZERO));
        world.add_component(ball, Collider::new_sphere(BALL_RADIUS));
        world.add_component(ball, EntityName(format!("Ball_{}", i)));

        game.ball_ids.push(ball.id());

        // İp (Rope) arka
        let rope_back = world.spawn();
        world.add_component(rope_back, Transform::new(Vec3::ZERO));
        world.add_component(rope_back, beam_mesh.clone());
        world.add_component(rope_back, Material::new(tex.clone()).with_pbr(Vec4::new(0.9, 0.9, 0.9, 1.0), 0.8, 0.2));
        world.add_component(rope_back, MeshRenderer::new());
        world.add_component(rope_back, EntityName(format!("Rope_Back_{}", i)));

        game.ropes.push(RopeData {
            ball_id: ball.id(),
            rope_id: rope_back.id(),
            pivot: Vec3::new(x, HINGE_HEIGHT, -z_offset),
        });

        // İp (Rope) ön
        let rope_front = world.spawn();
        world.add_component(rope_front, Transform::new(Vec3::ZERO));
        world.add_component(rope_front, beam_mesh.clone());
        world.add_component(rope_front, Material::new(tex.clone()).with_pbr(Vec4::new(0.9, 0.9, 0.9, 1.0), 0.8, 0.2));
        world.add_component(rope_front, MeshRenderer::new());
        world.add_component(rope_front, EntityName(format!("Rope_Front_{}", i)));

        game.ropes.push(RopeData {
            ball_id: ball.id(),
            rope_id: rope_front.id(),
            pivot: Vec3::new(x, HINGE_HEIGHT, z_offset),
        });

        // Constraint (Mesafe İpi)
        // Fiziksel olarak topun ağırlık merkezi (Vec3::ZERO) hedeflenir ki sarkarken tork yaratmasın ve takla atmasın.
        // Görsel ipler ise update_ropes içerisinde topun "yüzeyine" gidecek.
        let anchor_b = Vec3::ZERO; 
        
        // anchor_a'lar beam'e LOCAL koordinattadır! Beam'ler zaten z=-1.5 ve z=1.5'te duruyor!
        // O yüzden local z ekseni 0.0 olmalı.
        let anchor_a_back = Vec3::new(x, 0.0, 0.0);
        let mut joint_back = Joint::distance(beam_entity_back.id(), ball.id(), anchor_a_back, anchor_b, dist_len);
        joint_back.damping = 0.0; 
        
        let anchor_a_front = Vec3::new(x, 0.0, 0.0);
        let mut joint_front = Joint::distance(beam_entity_front.id(), ball.id(), anchor_a_front, anchor_b, dist_len);
        joint_front.damping = 0.0; 
        
        let mut jw = world.get_resource_mut::<JointWorld>().expect("ECS Aliasing Error").unwrap();
        jw.add(joint_back);
        jw.add(joint_front);
    }

    world.insert_resource(asset_manager);

    println!("Sahne hazır: {} top", BALL_COUNT);
    game
}

fn dir_to_quat(dir: Vec3) -> Quat {
    let dir_norm = dir.normalize_or_zero();
    let up = Vec3::new(0.0, 1.0, 0.0);
    let dot = up.dot(dir_norm);
    if dot < -0.9999 {
        Quat::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), std::f32::consts::PI)
    } else if dot > 0.9999 {
        Quat::IDENTITY
    } else {
        let axis = up.cross(dir_norm);
        let s = (2.0 * (1.0 + dot)).sqrt();
        let invs = 1.0 / s;
        Quat::from_xyzw(
            axis.x * invs,
            axis.y * invs,
            axis.z * invs,
            s * 0.5,
        ).normalize()
    }
}

fn update_ropes(world: &mut World, game: &CradleGame) {
    if let Some(mut transforms) = world.borrow_mut::<Transform>().expect("ECS Aliasing Error") {
        let mut updates = Vec::new();
        
        for rope in &game.ropes {
            if let Some(t_ball) = transforms.get(rope.ball_id) {
                // Ball'un tepesindeki noktanın dünya konumu
                let anchor_local = Vec3::new(0.0, BALL_RADIUS, 0.0);
                let anchor_world = t_ball.position + t_ball.rotation * anchor_local;
                
                let pivot = rope.pivot;
                let dir = anchor_world - pivot;
                let dist = dir.length();
                let mid = pivot + dir * 0.5;

                let rot = dir_to_quat(dir);
                
                updates.push((rope.rope_id, mid, dist, rot));
            }
        }

        for (rope_id, mid, dist, rot) in updates {
            if let Some(t_rope) = transforms.get_mut(rope_id) {
                t_rope.position = mid;
                t_rope.rotation = rot;
                t_rope.scale = Vec3::new(0.015, dist * 0.5, 0.015);
                t_rope.update_local_matrix();
            }
        }
    }
}

fn trigger_cradle(world: &mut World, game: &mut CradleGame) {
    println!("Sarkaç bırakıldı!");

    if let (Some(mut transforms), Some(mut vels), Some(mut rbs)) = (
        world.borrow_mut::<Transform>().expect("ECS Aliasing Error"),
        world.borrow_mut::<Velocity>().expect("ECS Aliasing Error"),
        world.borrow_mut::<RigidBody>().expect("ECS Aliasing Error")
    ) {
        if let Some(&first_id) = game.ball_ids.first() {
            if let Some(t) = transforms.get_mut(first_id) {
                let gap = 0.0;
                let diameter = (BALL_RADIUS * 2.0) + gap;
                let start_x = -((BALL_COUNT as f32 - 1.0) / 2.0) * diameter;
                
                let z_offset = 1.5_f32;
                let dist_len = 4.0_f32;
                let dy = (dist_len * dist_len - z_offset * z_offset).sqrt();
                
                // Topu 90 derece (tamamen yatay) sola kaldırıyoruz:
                t.position = Vec3::new(start_x - dy, HINGE_HEIGHT, 0.0);
                t.update_local_matrix();
            }
            if let Some(v) = vels.get_mut(first_id) {
                // Hız yapay olarak verilmez, yerçekimi serbest düşüşü sağlar
                v.linear = Vec3::ZERO;
                v.angular = Vec3::ZERO;
            }
            if let Some(rb) = rbs.get_mut(first_id) {
                rb.wake_up();
                rb.sleep_timer = 0.0;
            }
        }
    }
}

fn reset_cradle(world: &mut World, game: &mut CradleGame) {
    println!("Sarkaç sıfırlanıyor...");
    let gap = 0.001_f32;
    let diameter = (BALL_RADIUS * 2.0) + gap;
    let start_x = -((BALL_COUNT as f32 - 1.0) / 2.0) * diameter;

    if let (Some(mut transforms), Some(mut vels), Some(mut rbs)) = (
        world.borrow_mut::<Transform>().expect("ECS Aliasing Error"),
        world.borrow_mut::<Velocity>().expect("ECS Aliasing Error"),
        world.borrow_mut::<RigidBody>().expect("ECS Aliasing Error")
    ) {
        for (i, &ball_id) in game.ball_ids.iter().enumerate() {
            let x = start_x + (i as f32) * diameter;
            
            let z_offset = 1.5_f32;
            let dist_len = 4.0_f32;
            let dy = (dist_len * dist_len - z_offset * z_offset).sqrt();
            let ball_y = HINGE_HEIGHT - dy;
            
            if let Some(t) = transforms.get_mut(ball_id) {
                t.position = Vec3::new(x, ball_y, 0.0);
                t.rotation = Quat::IDENTITY;
                t.update_local_matrix();
            }
            if let Some(v) = vels.get_mut(ball_id) {
                v.linear = Vec3::ZERO;
                v.angular = Vec3::ZERO;
            }
            if let Some(rb) = rbs.get_mut(ball_id) {
                rb.wake_up();
                rb.sleep_timer = 0.0;
            }
        }
    }
    game.triggered = false;
}

fn step_physics(world: &mut World, dt: f32) {
    gizmo::physics::integration::physics_apply_forces_system(world, dt);
    gizmo::physics::system::physics_collision_system(world, dt);
    gizmo::physics::integration::physics_movement_system(world, dt);
}

fn update_camera(
    world: &mut World,
    state: &mut CradleGame,
    input: &gizmo::core::input::Input,
    dt: f32,
) {
    let fx = state.cam_yaw.cos() * state.cam_pitch.cos();
    let fy = state.cam_pitch.sin();
    let fz = state.cam_yaw.sin() * state.cam_pitch.cos();
    let fwd = Vec3::new(fx, fy, fz).normalize();
    let right = fwd.cross(Vec3::new(0.0, 1.0, 0.0)).normalize();
 
    let speed = state.cam_speed * dt;
 
    if input.is_key_pressed(KeyCode::KeyW as u32) { state.cam_pos += fwd * speed; }
    if input.is_key_pressed(KeyCode::KeyS as u32) { state.cam_pos -= fwd * speed; }
    if input.is_key_pressed(KeyCode::KeyA as u32) { state.cam_pos -= right * speed; }
    if input.is_key_pressed(KeyCode::KeyD as u32) { state.cam_pos += right * speed; }
    if input.is_key_pressed(KeyCode::KeyQ as u32) { state.cam_pos.y -= speed; }
    if input.is_key_pressed(KeyCode::KeyE as u32) { state.cam_pos.y += speed; }
 
    if let Some(mut trans) = world.borrow_mut::<Transform>().expect("ECS Aliasing Error") {
        if let Some(t) = trans.get_mut(state.cam_id) {
            t.position = state.cam_pos;
            t.rotation = pitch_yaw_quat(state.cam_pitch, state.cam_yaw);
            t.update_local_matrix();
        }
    }
    if let Some(mut cams) = world.borrow_mut::<Camera>().expect("ECS Aliasing Error") {
        if let Some(c) = cams.get_mut(state.cam_id) {
            c.yaw = state.cam_yaw;
            c.pitch = state.cam_pitch;
        }
    }
}

fn pitch_yaw_quat(pitch: f32, yaw: f32) -> Quat {
    let q_yaw = Quat::from_axis_angle(Vec3::new(0.0, 1.0, 0.0), yaw);
    let q_pitch = Quat::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), pitch);
    q_yaw * q_pitch
}
