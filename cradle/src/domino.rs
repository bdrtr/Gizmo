use gizmo::prelude::*;
use gizmo::physics::components::{PhysicsConfig, RigidBody, Velocity};
use gizmo::physics::shape::{Collider, ColliderShape, Aabb, Sphere};
use gizmo::physics::system::PhysicsSolverState;
use gizmo::renderer::asset::AssetManager;
use gizmo::renderer::components::{DirectionalLight, MeshRenderer};

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

struct DominoGame {
    cam_id: u32,
    cam_yaw: f32,
    cam_pitch: f32,
    cam_pos: Vec3,
    cam_speed: f32,
    physics_acc: f32,
    physics_dt: f32,
    fps_timer: f32,
    frames: u32,
    fps: f32,
}

impl DominoGame {
    fn new() -> Self {
        Self {
            cam_id: 0,
            cam_yaw: -std::f32::consts::FRAC_PI_2,
            cam_pitch: -0.2,
            cam_pos: Vec3::new(40.0, 15.0, 45.0),
            cam_speed: 30.0,
            physics_acc: 0.0,
            physics_dt: 1.0 / 60.0,
            fps_timer: 0.0,
            frames: 0,
            fps: 0.0,
        }
    }
}

fn main() {
    App::<DominoGame>::new("Gizmo — Domino Reaksiyonu", 1600, 900)
        .set_setup(|world, renderer| {
            let mut game = setup_scene(world, renderer);
            
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

fn setup_scene(world: &mut World, renderer: &gizmo::renderer::Renderer) -> DominoGame {
    println!("Domino Zinciri kuruluyor...");

    world.insert_resource(PhysicsConfig {
        ground_y: -0.5,
        max_linear_velocity: 200.0,
        max_angular_velocity: 100.0,
        deterministic_simulation: false,
        solver_iterations: 32,
        ..Default::default()
    });
    world.insert_resource(gizmo::physics::constraints::JointWorld::new());
    world.insert_resource(PhysicsSolverState::new());

    let mut asset_manager = AssetManager::new();

    let tex = asset_manager.create_white_texture(
        &renderer.device,
        &renderer.queue,
        &renderer.scene.texture_bind_group_layout,
    );

    // Zemin
    let ground_mesh = AssetManager::create_plane(&renderer.device, 200.0);
    let ground = world.spawn();
    world.add_component(
        ground,
        Transform::new(Vec3::new(0.0, -0.5, 0.0))
            .with_scale(Vec3::new(10.0, 1.0, 120.0)),
    );
    world.add_component(ground, ground_mesh);
    world.add_component(
        ground,
        Material::new(tex.clone()).with_pbr(Vec4::new(0.55, 0.5, 0.45, 1.0), 0.9, 0.0),
    );
    world.add_component(ground, MeshRenderer::new());
    world.add_component(ground, RigidBody::new_static());
    world.add_component(ground, Collider { shape: ColliderShape::Aabb(Aabb { half_extents: Vec3::new(100.0, 0.5, 120.0) }) });

    // Işık
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

    let num_dominoes = 100;
    let spacing = 0.8;
    let (dx, dy, dz) = (0.2, 1.0, 0.1);
    
    let domino_mesh = AssetManager::create_cube(&renderer.device);

    for i in 0..num_dominoes {
        let domino = world.spawn();
        let pos = Vec3::new(0.0, dy - 0.5, i as f32 * spacing); // Y adjustment for floor
        world.add_component(domino, Transform::new(pos).with_scale(Vec3::new(dx, dy, dz)));
        
        let color = if i % 2 == 0 { Vec4::new(0.8, 0.1, 0.1, 1.0) } else { Vec4::new(0.9, 0.9, 0.9, 1.0) };
        world.add_component(domino, Material::new(tex.clone()).with_pbr(color, 1.0, 0.0));
        world.add_component(domino, domino_mesh.clone());
        world.add_component(domino, MeshRenderer::new());

        let mut rb = RigidBody::new(0.5, 0.1, 0.3, true);
        rb.calculate_box_inertia(dx * 2.0, dy * 2.0, dz * 2.0);
        world.add_component(domino, rb);
        world.add_component(
            domino,
            Collider { shape: ColliderShape::Aabb(Aabb { half_extents: Vec3::new(dx, dy, dz) }) },
        );
        world.add_component(domino, Velocity::new(Vec3::ZERO));
    }

    // Heavy Ball
    let heavy_ball = world.spawn();
    world.add_component(heavy_ball, Transform::new(Vec3::new(0.0, dy - 0.5, -1.5)).with_scale(Vec3::splat(0.5)));
    let ball_mesh = AssetManager::create_sphere(&renderer.device, 1.0, 32, 32);
    world.add_component(heavy_ball, ball_mesh);
    world.add_component(heavy_ball, Material::new(tex.clone()).with_pbr(Vec4::new(0.2, 0.2, 0.2, 1.0), 0.8, 0.5));
    world.add_component(heavy_ball, MeshRenderer::new());

    let mut ball_rb = RigidBody::new(50.0, 0.2, 0.5, true);
    let r = 0.5;
    let inertia = (2.0 / 5.0) * ball_rb.mass * (r * r);
    ball_rb.local_inertia = Vec3::new(inertia, inertia, inertia);
    ball_rb.inverse_inertia_local = gizmo_math::Mat3::from_diagonal(Vec3::splat(1.0 / inertia));

    world.add_component(heavy_ball, ball_rb);
    world.add_component(
        heavy_ball,
        Collider { shape: ColliderShape::Sphere(Sphere { radius: 0.5 }) },
    );
    world.add_component(heavy_ball, Velocity::new(Vec3::new(0.0, 0.0, 25.0)));

    world.insert_resource(asset_manager);
    println!("Saha hazır!");
    DominoGame::new()
}

fn step_physics(world: &mut World, dt: f32) {
    gizmo::physics::integration::physics_apply_forces_system(world, dt);
    gizmo::physics::system::physics_collision_system(world, dt);
    gizmo::physics::integration::physics_movement_system(world, dt);
}

fn update_camera(
    world: &mut World,
    state: &mut DominoGame,
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
 
    if let Ok(mut trans) = world.borrow_mut::<Transform>() {
        if let Some(t) = trans.get_mut(state.cam_id) {
            t.position = state.cam_pos;
            t.rotation = pitch_yaw_quat(state.cam_pitch, state.cam_yaw);
            t.update_local_matrix();
        }
    }
    if let Ok(mut cams) = world.borrow_mut::<Camera>() {
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
