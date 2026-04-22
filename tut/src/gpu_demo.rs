use gizmo::prelude::*;
use std::cell::Cell;

struct GpuPhysicsDemo {
    cam_id: u32,
    cam_yaw: f32,
    cam_pitch: f32,
    cam_pos: Vec3,
    cam_speed: f32,
    fps_timer: f32,
    frames: u32,
    fps: f32,
    readback_timer: f32,
    should_request_readback: Cell<bool>,
}

impl GpuPhysicsDemo {
    fn new() -> Self {
        Self {
            cam_id: 0,
            cam_yaw: -std::f32::consts::FRAC_PI_2, // Look forward (-Z)
            cam_pitch: 0.2,                        // Look slightly down
            cam_pos: Vec3::new(0.0, 70.0, 80.0),   // Start high up to see 250,000 spheres falling!
            cam_speed: 60.0,
            fps_timer: 0.0,
            frames: 0,
            fps: 0.0,
            readback_timer: 0.0,
            should_request_readback: Cell::new(false),
        }
    }
}

fn main() {
    App::<GpuPhysicsDemo>::new("Gizmo GPU Physics — 250,000 Spheres", 1600, 900)
        .set_setup(|world, _renderer| {
            println!("🚀 GPU Physics İzole Ortamda Başlatılıyor...");
            let mut game = GpuPhysicsDemo::new();

            // Kamera Oluşturma
            let cam_entity = world.spawn();
            world.add_component(
                cam_entity,
                Transform::new(game.cam_pos).with_rotation(pitch_yaw_quat(game.cam_pitch, game.cam_yaw)),
            );
            world.add_component(
                cam_entity,
                Camera::new(
                    std::f32::consts::FRAC_PI_3,
                    0.1,
                    2000.0,
                    game.cam_yaw,
                    game.cam_pitch,
                    true,
                ),
            );
            world.add_component(cam_entity, EntityName("Gözlemci Kamerası".into()));
            game.cam_id = cam_entity.id();

            // Sadece Işık Ekleyelim (Küreler parlasın)
            let sun = world.spawn();
            world.add_component(
                sun,
                Transform::new(Vec3::new(0.0, 100.0, 0.0))
                    .with_rotation(Quat::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), -1.0)), // Güneş açısı
            );
            world.add_component(
                sun,
                gizmo::renderer::components::DirectionalLight::new(Vec3::new(1.0, 0.9, 0.9), 3.0, true),
            );

            game
        })
        .set_update(|world, state, dt, input| {
            // FPS Calculation
            state.fps_timer += dt;
            state.frames += 1;
            if state.fps_timer >= 1.0 {
                state.fps = state.frames as f32 / state.fps_timer;
                state.frames = 0;
                state.fps_timer = 0.0;
            }

            // GPU'dan CPU'ya saniyede 1 kez veri çekmeyi talep et
            state.readback_timer += dt;
            if state.readback_timer >= 1.0 {
                state.should_request_readback.set(true);
                state.readback_timer = 0.0;
            }

            // Free-cam Controls
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

            let mut trans = world.borrow_mut::<Transform>();
            if let Some(t) = trans.get_mut(state.cam_id) {
                t.position = state.cam_pos;
                t.rotation = pitch_yaw_quat(state.cam_pitch, state.cam_yaw);
                t.update_local_matrix();
            }
            
            let mut cams = world.borrow_mut::<Camera>();
            if let Some(c) = cams.get_mut(state.cam_id) {
                c.yaw = state.cam_yaw;
                c.pitch = state.cam_pitch;
            }
        })
        .set_ui(|_world, state, ctx| {
            gizmo::egui::Window::new("İstatistikler")
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
                    ui.label(
                        gizmo::egui::RichText::new("1.000.000 GPU Sphere")
                            .color(gizmo::egui::Color32::YELLOW)
                            .strong()
                            .size(16.0),
                    );
                });
        })
        .set_render(|world, _state, encoder, view, renderer, _light_time| {
            gizmo::default_systems::default_render_pass(world, encoder, view, renderer);
        })
        .run();
}

fn pitch_yaw_quat(pitch: f32, yaw: f32) -> Quat {
    let q_yaw = Quat::from_axis_angle(Vec3::new(0.0, 1.0, 0.0), yaw);
    let q_pitch = Quat::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), pitch);
    q_yaw * q_pitch
}
