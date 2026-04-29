use gizmo::physics::components::{RigidBody, Velocity};
use gizmo::prelude::*;
use gizmo::renderer::asset::AssetManager;
use gizmo::renderer::components::{DirectionalLight, MeshRenderer};

const DOMINO_COUNT: usize = 1000;

// ─── Domino boyutları (yarı-uzunluklar) ───
// Gerçekçi domino oranları: genişlik:yükseklik:kalınlık = 1:2:0.4
const HX: f32 = 0.10; // genişlik  → 0.20 m
const HY: f32 = 0.50; // yükseklik → 1.00 m
const HZ: f32 = 0.06; // kalınlık  → 0.12 m

// Standart aralık: taş yüksekliğinin ~%70'i yeterli olmalı
const GAP: f32 = 0.35;

const GROUND_Y: f32 = 0.0;
const GROUND_HALF_Y: f32 = 0.05; // zemin collider yarı yüksekliği (Collider::new_aabb ikinci parametre)
const DOMINO_MASS: f32 = 0.3; // Biraz daha ağır = daha kararlı çarpışma
const BALL_MASS: f32 = 1.0;
const BALL_RADIUS: f32 = 0.20;

struct DominoGame {
    domino_ids: Vec<u32>,
    ball_id: u32,
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

impl DominoGame {
    fn new() -> Self {
        Self {
            domino_ids: Vec::new(),
            ball_id: 0,
            cam_id: 0,
            cam_yaw: -std::f32::consts::FRAC_PI_2,
            cam_pitch: -0.4,
            cam_pos: Vec3::new(0.0, 8.0, 20.0),
            cam_speed: 15.0,
            triggered: false,
            physics_acc: 0.0,
            physics_dt: 1.0 / 60.0, // 60 Hz — performans/kalite dengesi
            fps_timer: 0.0,
            frames: 0,
            fps: 0.0,
        }
    }
}

fn main() {
    App::<DominoGame>::new("Gizmo — Domino Oyunu (500 taş)", 1600, 900)
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
            // FPS
            state.fps_timer += dt;
            state.frames += 1;
            if state.fps_timer >= 1.0 {
                state.fps = state.frames as f32 / state.fps_timer;
                state.frames = 0;
                state.fps_timer = 0.0;
            }

            // Space → trigger
            if input.is_key_pressed(KeyCode::Space as u32) && !state.triggered {
                trigger_first_domino(world, state);
                state.triggered = true;
            }

            // Mouse camera rotation
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

            // Fizik sub-stepping
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
                .frame(
                    gizmo::egui::Frame::window(&ctx.style())
                        .fill(gizmo::egui::Color32::from_black_alpha(150)),
                )
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
    println!("Domino sahne kuruluyor: {} taş…", DOMINO_COUNT);

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
        Transform::new(Vec3::new(0.0, GROUND_Y, 0.0)).with_scale(Vec3::new(1.0, 1.0, 1.0)),
    );
    world.add_component(ground, ground_mesh);
    world.add_component(
        ground,
        Material::new(tex.clone()).with_pbr(Vec4::new(0.55, 0.5, 0.45, 1.0), 0.9, 0.0),
    );
    world.add_component(ground, MeshRenderer::new());
    world.add_component(ground, RigidBody::new_static());
    world.add_component(ground, Collider::box_collider(Vec3::new(100.0, 0.05, 100.0)));

    // Güneş ışığı
    let sun = world.spawn();
    world.add_component(
        sun,
        Transform::new(Vec3::new(30.0, 80.0, 40.0)).with_rotation(Quat::from_axis_angle(
            Vec3::new(1.0, 0.3, 0.0).normalize(),
            -0.8,
        )),
    );
    world.add_component(
        sun,
        DirectionalLight::new(Vec3::new(1.0, 0.97, 0.90), 2.5, gizmo::renderer::components::LightRole::Sun),
    );

    let domino_mesh = AssetManager::create_cube(&renderer.device);
    let mut game = DominoGame::new();
    let positions = spiral_positions(DOMINO_COUNT);

    // Domino texture yükle
    let domino_tex = asset_manager
        .load_material_texture(
            &renderer.device,
            &renderer.queue,
            &renderer.scene.texture_bind_group_layout,
            "tut/assets/domino_real.png",
        )
        .unwrap_or_else(|e| {
            eprintln!(
                "Domino texture yüklenemedi: {}. Beyaz fallback kullanılıyor.",
                e
            );
            tex.clone()
        });

    for (i, pos) in positions.iter().enumerate() {
        let t = i as f32 / DOMINO_COUNT as f32;
        let color = Vec4::new(1.0, t * 0.85, 0.05, 1.0);

        // pos.x = world X, pos.y = world Z, pos.z = rotation angle around Y
        let transform = Transform::new(Vec3::new(
            pos.x,
            GROUND_Y + GROUND_HALF_Y + HY, // zemin üst yüzeyi + yarı yükseklik
            pos.y,
        ))
        .with_rotation(Quat::from_axis_angle(Vec3::new(0.0, 1.0, 0.0), pos.z))
        .with_scale(Vec3::new(HX, HY, HZ));

        let entity = world.spawn();

        world.add_component(entity, transform);
        world.add_component(entity, domino_mesh.clone());
        world.add_component(
            entity,
            Material::new(domino_tex.clone()).with_pbr(color, 0.7, 0.02),
        );
        world.add_component(entity, MeshRenderer::new());

        let mut rb = RigidBody::new(DOMINO_MASS, 0.05, 0.6, true);
        rb.ccd_enabled = false;
        rb.is_sleeping = true; // Başlangıçta uyku — sadece çarpışma gelince uyanır
        rb.calculate_box_inertia(HX * 2.0, HY * 2.0, HZ * 2.0);
        world.add_component(entity, rb);
        world.add_component(entity, Velocity::new(Vec3::ZERO));

        world.add_component(entity, Collider::box_collider(Vec3::new(HX, HY, HZ)));
        world.add_component(entity, EntityName(format!("Domino_{}", i)));

        game.domino_ids.push(entity.id());
    }

    // İtme topu — ilk dominonun arkasına yerleştir
    let ball_mesh = AssetManager::create_sphere(&renderer.device, 1.0, 12, 12);
    let ball = world.spawn();
    let first_pos = positions[0];
    // pos.z = atan2(dx,dz) ile hesaplanan açı
    // Quat::from_axis_angle(Y, angle) ile local +Z → tangent yönüne döner
    // Yani topun ileri yönü: (sin(angle), 0, cos(angle))
    let angle = first_pos.z;
    let tangent = Vec3::new(angle.sin(), 0.0, angle.cos());

    world.add_component(
        ball,
        Transform::new(Vec3::new(
            first_pos.x - tangent.x * 1.5,
            GROUND_Y + GROUND_HALF_Y + BALL_RADIUS,
            first_pos.y - tangent.z * 1.5,
        ))
        .with_scale(Vec3::splat(BALL_RADIUS)),
    );
    world.add_component(ball, ball_mesh);
    world.add_component(
        ball,
        Material::new(tex.clone()).with_pbr(Vec4::new(0.1, 0.4, 1.0, 1.0), 0.2, 0.8),
    );
    world.add_component(ball, MeshRenderer::new());

    let mut ball_rb = RigidBody::new(BALL_MASS, 0.5, 0.3, true);
    ball_rb.ccd_enabled = true;
    ball_rb.is_sleeping = true; // Space'e basılana kadar uyku
    ball_rb.calculate_sphere_inertia(BALL_RADIUS);
    world.add_component(ball, ball_rb);
    world.add_component(ball, Velocity::new(Vec3::ZERO));
    world.add_component(ball, Collider::sphere(BALL_RADIUS));
    world.add_component(ball, EntityName("İtme Topu".into()));

    game.ball_id = ball.id();
    world.insert_resource(asset_manager);

    println!("Sahne hazır: {} domino taşı + 1 top", DOMINO_COUNT);
    game
}

/// Arşimet sarmalı üzerinde sabit aralıkla domino konumları üretir.
/// Her Vec3: (x, z_world, rotation_angle)
fn spiral_positions(count: usize) -> Vec<Vec3> {
    let mut out = Vec::with_capacity(count);

    let a = 2.0_f32; // başlangıç yarıçapı
    let b = 0.25_f32; // sarmal büyüme oranı (daha sıkı halkalar)

    let mut theta = 0.0_f32;
    for _ in 0..count {
        let r = a + b * theta;
        let x = theta.cos() * r;
        let z = theta.sin() * r;

        // Teğet yönü (analitik türev):
        // dx/dθ = -r·sin(θ) + b·cos(θ)
        // dz/dθ =  r·cos(θ) + b·sin(θ)
        let tx = -r * theta.sin() + b * theta.cos();
        let tz = r * theta.cos() + b * theta.sin();
        let angle = tx.atan2(tz);

        out.push(Vec3::new(x, z, angle));

        // Sabit yay uzunluğu adımı: ds = GAP
        // ds² = (dx/dθ)² + (dz/dθ)² = r² + b²  (Arşimet spirali özelliği)
        // dθ = ds / sqrt(r² + b²)
        let arc_speed = (r * r + b * b).sqrt();
        let d_theta = GAP / arc_speed;
        theta += d_theta;
    }

    out
}

fn trigger_first_domino(world: &mut World, game: &DominoGame) {
    println!("İlk domino taşı tetiklendi!");

    // İlk dominonun teğet yönünü al
    let angle = {
        let positions = spiral_positions(1);
        positions[0].z
    };
    let fwd = Vec3::new(angle.sin(), 0.0, angle.cos());

    let mut vels = world.borrow_mut::<Velocity>();
    let mut rbs = world.borrow_mut::<RigidBody>();
    {
        // Topa hız ver — ilk dominoya doğru
        if let Some(v) = vels.get_mut(game.ball_id) {
            v.linear = fwd * 4.0; // Hafif itme, çok güçlü → yığılma
        }
        if let Some(rb) = rbs.get_mut(game.ball_id) {
            rb.wake_up();
        }

        // İlk dominoyu da dürt — teğet yönünde devrilecek şekilde
        if let Some(first_id) = game.domino_ids.first() {
            if let Some(v) = vels.get_mut(*first_id) {
                // Y × fwd = sağ yöndeki eksen → taşı ileri doğru devirir
                let pitch_axis = Vec3::new(0.0, 1.0, 0.0).cross(fwd);
                if pitch_axis.length_squared() > 0.001 {
                    v.angular = pitch_axis.normalize() * 5.0;
                }
            }
            if let Some(rb) = rbs.get_mut(*first_id) {
                rb.wake_up();
            }
        }
    }
}

fn step_physics(_world: &mut World, _dt: f32) {}

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

    if input.is_key_pressed(KeyCode::KeyW as u32) {
        state.cam_pos += fwd * speed;
    }
    if input.is_key_pressed(KeyCode::KeyS as u32) {
        state.cam_pos -= fwd * speed;
    }
    if input.is_key_pressed(KeyCode::KeyA as u32) {
        state.cam_pos -= right * speed;
    }
    if input.is_key_pressed(KeyCode::KeyD as u32) {
        state.cam_pos += right * speed;
    }
    if input.is_key_pressed(KeyCode::KeyQ as u32) {
        state.cam_pos.y -= speed;
    }
    if input.is_key_pressed(KeyCode::KeyE as u32) {
        state.cam_pos.y += speed;
    }

    let mut trans = world.borrow_mut::<Transform>();
    {
        if let Some(t) = trans.get_mut(state.cam_id) {
            t.position = state.cam_pos;
            t.rotation = pitch_yaw_quat(state.cam_pitch, state.cam_yaw);
            t.update_local_matrix();
        }
    }
    let mut cams = world.borrow_mut::<Camera>();
    {
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
