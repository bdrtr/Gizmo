use gizmo::prelude::*;
use gizmo::renderer::asset::AssetManager;
use gizmo::renderer::components::{DirectionalLight, MeshRenderer};

struct ShowcaseDemo {
    cam_id: u32,
    cam_yaw: f32,
    cam_pitch: f32,
    cam_pos: Vec3,
    cam_speed: f32,
    fps_timer: f32,
    frames: u32,
    fps: f32,
    time: f32,
    sun_id: u32,
}

impl ShowcaseDemo {
    fn new() -> Self {
        Self {
            cam_id: 0,
            cam_yaw: 2.5,
            cam_pitch: -0.2,
            cam_pos: Vec3::new(-10.0, 5.0, -10.0),
            cam_speed: 10.0,
            fps_timer: 0.0,
            frames: 0,
            fps: 0.0,
            time: 0.0,
            sun_id: 0,
        }
    }
}

fn main() {
    App::<ShowcaseDemo>::new("Gizmo — AAA Showcase (IBL, SSR, God Rays)", 1600, 900)
        .set_setup(|world, renderer| {
            println!("##################################################");
            println!("    AAA Showcase Başlıyor...");
            println!("##################################################");

            let mut game = ShowcaseDemo::new();
            let mut asset_manager = AssetManager::new();

            let tex = asset_manager.create_white_texture(
                &renderer.device,
                &renderer.queue,
                &renderer.scene.texture_bind_group_layout,
            );

            // Kamera
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
                    5000.0,
                    game.cam_yaw,
                    game.cam_pitch,
                    true,
                ),
            );
            world.add_component(cam_entity, EntityName("Kamera".into()));
            game.cam_id = cam_entity.id();

            // Güneş (Gün batımı açısı ve rengi)
            let sun = world.spawn();
            world.add_component(
                sun,
                Transform::new(Vec3::new(0.0, 50.0, 0.0))
                    .with_rotation(Quat::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), -0.3)), // Gün batımı açısı
            );
            world.add_component(
                sun,
                DirectionalLight::new(
                    Vec3::new(1.0, 0.7, 0.4), // Sıcak gün batımı rengi
                    3.0,
                    gizmo::renderer::components::LightRole::Sun,
                ),
            );
            game.sun_id = sun.id();

            let ground_tex = asset_manager.load_material_texture(
                &renderer.device,
                &renderer.queue,
                &renderer.scene.texture_bind_group_layout,
                "assets/grass.jpg",
            ).expect("Failed to load grass texture");

            // Yer düzlemi
            let ground_mesh = AssetManager::create_plane(&renderer.device, 200.0);
            let ground = world.spawn();
            world.add_component(
                ground,
                Transform::new(Vec3::new(0.0, 0.0, 0.0)).with_scale(Vec3::new(1.0, 1.0, 1.0)),
            );
            world.add_component(ground, ground_mesh);
            world.add_component(
                ground,
                Material::new(ground_tex)
                    .with_pbr(Vec4::new(1.0, 1.0, 1.0, 1.0), 0.9, 0.0) // Çimen için mat bir yüzey
                    .with_double_sided(true),
            );
            world.add_component(ground, MeshRenderer::new());

            // sGökyüzü (Skybox)
            let skybox = world.spawn();
            let sky_mesh = AssetManager::create_sphere(&renderer.device, 2000.0, 32, 32);
            world.add_component(
                skybox,
                Transform::new(Vec3::new(0.0, 0.0, 0.0)),
            );
            world.add_component(skybox, sky_mesh);
            world.add_component(
                skybox,
                Material::new(tex.clone())
                    .with_unlit(Vec4::new(1.0, 1.0, 1.0, 1.0)) // Beyaz (sky.wgsl için çarpan)
                    .with_skybox(), // Skybox olarak işaretle
            );
            world.add_component(skybox, MeshRenderer::new());

            // Referans için bir sütun ekleyelim
            let pillar = world.spawn();
            let cube_mesh = AssetManager::create_cube(&renderer.device);
            world.add_component(
                pillar,
                Transform::new(Vec3::new(0.0, 5.0, -20.0)).with_scale(Vec3::new(2.0, 10.0, 2.0)),
            );
            world.add_component(pillar, cube_mesh);
            world.add_component(
                pillar,
                Material::new(tex.clone()).with_pbr(Vec4::new(0.8, 0.2, 0.2, 1.0), 0.2, 0.0),
            );
            world.add_component(pillar, MeshRenderer::new());

            world.insert_resource(asset_manager);
            game
        })
        .set_update(|world, state, dt, input| {
            // FPS
            state.fps_timer += dt;
            state.frames += 1;
            state.time += dt;
            if state.fps_timer >= 1.0 {
                state.fps = state.frames as f32 / state.fps_timer;
                state.frames = 0;
                state.fps_timer = 0.0;
            }

            // Güneşi sabitleyelim ki sunset açısı bozulmasın
            // if let Some(mut trans) = world.borrow_mut::<Transform>().get_mut(state.sun_id) {
            //     let sun_angle = (state.time * 0.1).sin() * 0.5 - 0.5;
            //     trans.rotation = Quat::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), sun_angle)
            //         * Quat::from_axis_angle(Vec3::new(0.0, 1.0, 0.0), state.time * 0.2);
            //     trans.update_local_matrix();
            // }

            // Mouse camera rotation
            if input.is_mouse_button_pressed(gizmo::core::input::mouse::RIGHT) {
                let delta = input.mouse_delta();
                let sens = 0.0008_f32;
                state.cam_yaw -= delta.0 * sens;
                state.cam_pitch -= delta.1 * sens;
                state.cam_pitch = state.cam_pitch.clamp(
                    -std::f32::consts::FRAC_PI_2 + 0.05,
                    std::f32::consts::FRAC_PI_2 - 0.05,
                );
            }

            // Kamera Hareketi
            let fx = state.cam_yaw.cos() * state.cam_pitch.cos();
            let fy = state.cam_pitch.sin();
            let fz = state.cam_yaw.sin() * state.cam_pitch.cos();
            let fwd = Vec3::new(fx, fy, fz).normalize();
            let right = fwd.cross(Vec3::new(0.0, 1.0, 0.0)).normalize();

            let speed = state.cam_speed * dt;
            let mut sprint = 1.0;
            if input.is_key_pressed(KeyCode::ShiftLeft as u32) {
                sprint = 3.0;
            }

            if input.is_key_pressed(KeyCode::KeyW as u32) {
                state.cam_pos += fwd * speed * sprint;
            }
            if input.is_key_pressed(KeyCode::KeyS as u32) {
                state.cam_pos -= fwd * speed * sprint;
            }
            if input.is_key_pressed(KeyCode::KeyA as u32) {
                state.cam_pos -= right * speed * sprint;
            }
            if input.is_key_pressed(KeyCode::KeyD as u32) {
                state.cam_pos += right * speed * sprint;
            }
            if input.is_key_pressed(KeyCode::KeyQ as u32) {
                state.cam_pos.y -= speed * sprint;
            }
            if input.is_key_pressed(KeyCode::KeyE as u32) {
                state.cam_pos.y += speed * sprint;
            }

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
            gizmo::egui::Window::new("Gizmo AAA Engine")
                .anchor(gizmo::egui::Align2::LEFT_TOP, gizmo::egui::vec2(10.0, 10.0))
                .title_bar(false)
                .resizable(false)
                .frame(
                    gizmo::egui::Frame::window(&ctx.style())
                        .fill(gizmo::egui::Color32::from_black_alpha(200)),
                )
                .show(ctx, |ui| {
                    ui.label(
                        gizmo::egui::RichText::new(format!("FPS: {:.0}", state.fps))
                            .color(gizmo::egui::Color32::YELLOW)
                            .strong()
                            .size(24.0),
                    );
                    ui.separator();
                    ui.label("Eklentiler:");
                    ui.label("✅ Deferred PBR Pipeline");
                    ui.label("✅ CSM Shadows");
                    ui.label("✅ Screen Space Reflections (SSR)");
                    ui.label("✅ God Rays (Volumetric Lighting)");
                    ui.label("✅ Procedural IBL");
                    ui.label("✅ TAA & Bloom");
                });
        })
        .set_render(|world, _state, encoder, view, renderer, _light_time| {
            // Başka demolardan kalan otomatik GPU sistemlerini kapat:
            renderer.gpu_physics = None;
            renderer.gpu_fluid = None;
            gizmo::default_systems::default_render_pass(world, encoder, view, renderer);
        })
        .run();
}

fn pitch_yaw_quat(pitch: f32, yaw: f32) -> Quat {
    let q_yaw = Quat::from_axis_angle(Vec3::new(0.0, 1.0, 0.0), yaw);
    let q_pitch = Quat::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), pitch);
    q_yaw * q_pitch
}
