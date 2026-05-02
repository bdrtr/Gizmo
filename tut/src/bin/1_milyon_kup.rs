use gizmo::prelude::*;

struct MillionKupGame {
    cam_id: u32,
    cam_yaw: f32,
    cam_pitch: f32,
    cam_pos: Vec3,
    cam_speed: f32,
    fps_timer: f32,
    frames: u32,
    fps: f32,
}

impl MillionKupGame {
    fn new() -> Self {
        Self {
            cam_id: 0,
            cam_yaw: -std::f32::consts::FRAC_PI_2,
            cam_pitch: -0.4,
            cam_pos: Vec3::new(0.0, 150.0, 300.0), // Kamerayı küp kalabalığından tamamen dışarı aldık
            cam_speed: 40.0,
            fps_timer: 0.0,
            frames: 0,
            fps: 0.0,
        }
    }
}

fn main() {
    run(50_000); // 1 Milyon tam OBB fizik iterasyonu GPU'yu kitler, 50k ile 60+ FPS alalım!
}

fn run(cube_count: u32) {
    App::<MillionKupGame>::new("Gizmo — 50 Bin Küp Simülasyonu", 1600, 900)
        .set_setup(move |world, _renderer| {
            println!("##################################################");
            println!("    {} GPU Küp Simülasyonu Başlıyor...", cube_count);
            println!("##################################################");

            let mut game = MillionKupGame::new();

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

            // Siyah ekranı önlemek için sahneye güçlü bir Güneş ekleyelim
            let sun = world.spawn();
            world.add_component(
                sun,
                Transform::new(Vec3::new(0.0, 500.0, 0.0))
                    .with_rotation(Quat::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), -1.0)),
            );
            world.add_component(
                sun,
                DirectionalLight::new(Vec3::new(1.0, 1.0, 0.95), 3.0, gizmo::renderer::components::LightRole::Sun),
            );

            game
        })
        .set_update(|world, state, dt, input| {
            // FPS Sayacı
            state.fps_timer += dt;
            state.frames += 1;
            if state.fps_timer >= 1.0 {
                state.fps = state.frames as f32 / state.fps_timer;
                println!("FPS: {:.1}", state.fps);
                state.frames = 0;
                state.fps_timer = 0.0;
            }

            // Kamera Hareketi (WASD + QE + Shift)
            let mut speed = state.cam_speed;
            if input.is_key_pressed(KeyCode::ShiftLeft as u32) {
                speed *= 3.0;
            }

            let mut cam_move = Vec3::ZERO;
            if input.is_key_pressed(KeyCode::KeyW as u32) {
                cam_move.z -= 1.0;
            }
            if input.is_key_pressed(KeyCode::KeyS as u32) {
                cam_move.z += 1.0;
            }
            if input.is_key_pressed(KeyCode::KeyA as u32) {
                cam_move.x -= 1.0;
            }
            if input.is_key_pressed(KeyCode::KeyD as u32) {
                cam_move.x += 1.0;
            }
            if input.is_key_pressed(KeyCode::KeyQ as u32) {
                cam_move.y -= 1.0;
            }
            if input.is_key_pressed(KeyCode::KeyE as u32) {
                cam_move.y += 1.0;
            }

            if cam_move.length_squared() > 0.0 {
                cam_move = cam_move.normalize() * speed * dt;
            }

            // Kamera Fare kontrolü
            let mouse_delta = input.mouse_delta();
            if input.is_mouse_button_pressed(1) {
                // 1 = Right Click typically in some engines, or just remove mouse click constraint. Let's just use 1.
                state.cam_yaw -= mouse_delta.0 * 0.002;
                state.cam_pitch -= mouse_delta.1 * 0.002;
                state.cam_pitch = state.cam_pitch.clamp(-1.5, 1.5);
            }

            // Transformu Güncelle
            if let Some(tr) = world.borrow_mut::<Transform>().get_mut(state.cam_id) {
                let rot = pitch_yaw_quat(state.cam_pitch, state.cam_yaw);
                tr.rotation = rot;

                let forward = rot * Vec3::new(0.0, 0.0, -1.0);
                let right = rot * Vec3::new(1.0, 0.0, 0.0);
                let up = Vec3::new(0.0, 1.0, 0.0);

                let movement = right * cam_move.x + up * cam_move.y - forward * cam_move.z;
                tr.position += movement;
                state.cam_pos = tr.position;
            }
        })
        .set_ui(|_world, state, ctx| {
            gizmo::egui::Area::new(gizmo::egui::Id::new("fps_counter"))
                .anchor(gizmo::egui::Align2::LEFT_TOP, [10.0, 10.0])
                .show(ctx, |ui| {
                    ui.label(
                        gizmo::egui::RichText::new(format!("FPS: {:.1}", state.fps))
                            .color(gizmo::egui::Color32::YELLOW)
                            .size(24.0)
                            .strong(),
                    );
                });
        })
        .set_render(|world, _state, encoder, view, renderer, _light_time| {
            if let Some(physics) = &mut renderer.gpu_physics {
                if !physics.debug_enabled {
                    physics.enable_debug(&renderer.device, 0);
                }
            }
            gizmo::systems::default_render_pass(world, encoder, view, renderer);
        })
        .run();
}

fn pitch_yaw_quat(pitch: f32, yaw: f32) -> Quat {
    let q_yaw = Quat::from_axis_angle(Vec3::new(0.0, 1.0, 0.0), yaw);
    let q_pitch = Quat::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), pitch);
    q_yaw * q_pitch
}
