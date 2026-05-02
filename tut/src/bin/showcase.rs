use gizmo::prelude::*;
use gizmo::renderer::asset::AssetManager;
use gizmo::renderer::components::{DirectionalLight, MeshRenderer, Decal};

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
    // Post Processing Cinematics
    bloom_intensity: f32,
    exposure: f32,
    chromatic_aberration: f32,
    vignette_intensity: f32,
    film_grain_intensity: f32,
    dof_focus_dist: f32,
    dof_focus_range: f32,
    dof_blur_size: f32,
    light_1_id: u32,
    light_2_id: u32,
    light_3_id: u32,
    cube_mesh: Option<Mesh>,
    tex: Option<std::sync::Arc<wgpu::BindGroup>>,
    last_shot_time: f32,
    last_push_time: f32,
    request_force_push: bool,
    request_force_pull: bool,
    request_grab: bool,
    grabbed_object_id: std::cell::Cell<Option<usize>>,
    particle_spawn_rate: usize,
    particle_color_r: f32,
    particle_color_g: f32,
    particle_color_b: f32,
    particle_velocity_y: f32,
    gravity_y: f32,
    time_scale: f32,
    time_of_day: f32,
    cam_velocity_y: f32,
    fly_mode: bool,
}

impl ShowcaseDemo {
    fn new() -> Self {
        Self {
            cam_id: 0,
            cam_yaw: -1.57, // -PI/2 (Look straight at -Z)
            cam_pitch: -0.2,
            cam_pos: Vec3::new(0.0, 5.0, 20.0),
            cam_speed: 10.0,
            fps_timer: 0.0,
            frames: 0,
            fps: 0.0,
            time: 0.0,
            sun_id: 0,
            bloom_intensity: 0.3,
            exposure: 1.0,
            chromatic_aberration: 0.0,
            vignette_intensity: 0.0,
            film_grain_intensity: 0.0,
            dof_focus_dist: 20.0, // Odak noktası (20 birim uzakta, sütunun olduğu yer)
            dof_focus_range: 15.0, // Odak alanının genişliği
            dof_blur_size: 4.0,   // Max bulanıklık çapı
            light_1_id: 0,
            light_2_id: 0,
            light_3_id: 0,
            cube_mesh: None,
            tex: None,
            last_shot_time: 0.0,
            last_push_time: 0.0,
            request_force_push: false,
            request_force_pull: false,
            request_grab: false,
            grabbed_object_id: std::cell::Cell::new(None),
            particle_spawn_rate: 100,
            particle_color_r: 1.0,
            particle_color_g: 0.4,
            particle_color_b: 0.1,
            particle_velocity_y: 10.0,
            gravity_y: -9.81,
            time_scale: 1.0,
            time_of_day: 17.5,
            cam_velocity_y: 0.0,
            fly_mode: false, // Varsayılan olarak yürüme modu
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
            world.add_component(pillar, cube_mesh.clone());
            world.add_component(
                pillar,
                Material::new(tex.clone()).with_pbr(Vec4::new(0.8, 0.8, 0.8, 1.0), 0.2, 0.0), // Beyazımsı yapalım ışıklar belli olsun
            );
            world.add_component(pillar, MeshRenderer::new());
            world.add_component(pillar, Collider::box_collider(Vec3::new(1.0, 5.0, 1.0)));
            world.add_component(pillar, RigidBody::new(0.0, 0.5, 0.5, false));

            // GPU Physics için Zemin
            world.add_component(ground, Collider::box_collider(Vec3::new(100.0, 0.1, 100.0)));
            world.add_component(ground, RigidBody::new(0.0, 0.5, 0.5, false));

            // --- 500 GPU Physics Küpü (Lavın İçinden Dökülen Taşlar) ---
            use rand::Rng;
            let mut rng = rand::thread_rng();
            for _i in 0..500 {
                let box_ent = world.spawn();
                let px = rng.gen_range(-5.0..5.0);
                let py = rng.gen_range(10.0..30.0);
                let pz = rng.gen_range(-25.0..-15.0);
                world.add_component(box_ent, Transform::new(Vec3::new(px, py, pz)).with_scale(Vec3::new(0.5, 0.5, 0.5)));
                world.add_component(box_ent, cube_mesh.clone());
                world.add_component(
                    box_ent,
                    Material::new(tex.clone()).with_pbr(Vec4::new(0.2, 0.2, 0.2, 1.0), 0.8, 0.0), // Siyah kömür gibi taşlar
                );
                world.add_component(box_ent, MeshRenderer::new());
                world.add_component(box_ent, Collider::box_collider(Vec3::new(0.25, 0.25, 0.25)));
                world.add_component(box_ent, RigidBody::new(1.0, 0.5, 0.5, true));
            }

            // --- Dinamik Nokta Işıklar (Point Lights) ---
            let l1 = world.spawn();
            world.add_component(l1, Transform::new(Vec3::new(0.0, 5.0, -20.0)));
            world.add_component(l1, gizmo::renderer::components::PointLight::new(Vec3::new(1.0, 0.0, 0.0), 50.0, 10.0));
            game.light_1_id = l1.id();

            let l2 = world.spawn();
            world.add_component(l2, Transform::new(Vec3::new(0.0, 5.0, -20.0)));
            world.add_component(l2, gizmo::renderer::components::PointLight::new(Vec3::new(0.0, 1.0, 0.0), 50.0, 10.0));
            game.light_2_id = l2.id();

            let l3 = world.spawn();
            world.add_component(l3, Transform::new(Vec3::new(0.0, 5.0, -20.0)));
            world.add_component(l3, gizmo::renderer::components::PointLight::new(Vec3::new(0.0, 0.5, 1.0), 50.0, 10.0));
            game.light_3_id = l3.id();

            game.cube_mesh = Some(cube_mesh);
            game.tex = Some(tex);

            // Decal (Çıkartma) Testi
            if let Some(decal_state) = renderer.decal.as_ref() {
                let decal_tex = asset_manager.create_checkerboard_texture(
                    &renderer.device,
                    &renderer.queue,
                    &decal_state.decal_tex_bgl,
                );

                let decal_entity = world.spawn();
                world.add_component(
                    decal_entity,
                    Transform::new(Vec3::new(0.0, 0.0, -10.0))
                        .with_scale(Vec3::new(8.0, 8.0, 8.0)), // 8x8x8 volume
                );
                world.add_component(
                    decal_entity,
                    Decal::new(decal_tex, Vec4::new(1.0, 0.2, 0.2, 1.0)), // Kırmızımtırak renk
                );
            }

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

            if let Some(mut time_res) = world.get_resource_mut::<gizmo::core::time::Time>() {
                time_res.set_time_scale(state.time_scale);
            }

            // Güneşi sabitleyelim ki sunset açısı bozulmasın
            // if let Some(mut trans) = world.borrow_mut::<Transform>().get_mut(state.sun_id) {
            //     let sun_angle = (state.time * 0.1).sin() * 0.5 - 0.5;
            //     trans.rotation = Quat::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), sun_angle)
            //         * Quat::from_axis_angle(Vec3::new(0.0, 1.0, 0.0), state.time * 0.2);
            //     trans.update_local_matrix();
            // }

            // Day/Night Cycle (Zaman Akışı)
            let hour = state.time_of_day;
            let sun_angle = (hour - 6.0) / 12.0 * std::f32::consts::PI; // 06:00 = 0, 12:00 = PI/2, 18:00 = PI
            let is_day = hour >= 6.0 && hour <= 18.0;

            if let Some(trans) = world.borrow_mut::<Transform>().get_mut(state.sun_id) {
                // Güneşin dönüşünü ayarla
                trans.rotation = Quat::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), -sun_angle);
                trans.update_local_matrix();
            }

            if let Some(light) = world.borrow_mut::<DirectionalLight>().get_mut(state.sun_id) {
                if !is_day {
                    // Gece: Ay ışığı (mavi/karanlık)
                    light.color = Vec3::new(0.1, 0.2, 0.4);
                    light.intensity = 0.5;
                } else {
                    // Gündüz
                    let t = (hour - 12.0).abs() / 6.0; // 12:00 = 0.0 (Öğle), 06:00 veya 18:00 = 1.0 (Gün doğumu/batımı)
                    
                    // Renk interpolasyonu (Öğle: Beyaz, Gün Batımı: Turuncu)
                    let noon_color = Vec3::new(1.0, 1.0, 0.9);
                    let sunset_color = Vec3::new(1.0, 0.5, 0.1);
                    
                    light.color = noon_color.lerp(sunset_color, t);
                    // Yoğunluk: Öğlen en yüksek, gün batımında düşük
                    light.intensity = (1.0 - t * 0.8) * 5.0; 
                }
            }

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

            // Dinamik ışıkları pillar etrafında döndürelim (Pillar konumu: 0.0, 5.0, -20.0)
            let pillar_pos = Vec3::new(0.0, 5.0, -20.0);
            let orbit_radius = 5.0;
            let speed = 2.0;

            if let Some(trans) = world.borrow_mut::<Transform>().get_mut(state.light_1_id) {
                let angle = state.time * speed;
                trans.position = pillar_pos + Vec3::new(angle.cos() * orbit_radius, (angle * 2.0).sin() * 2.0, angle.sin() * orbit_radius);
                trans.update_local_matrix();
            }
            if let Some(trans) = world.borrow_mut::<Transform>().get_mut(state.light_2_id) {
                let angle = state.time * speed + 2.094; // 120 derece ofset
                trans.position = pillar_pos + Vec3::new(angle.cos() * orbit_radius, (angle * 2.0).sin() * 2.0, angle.sin() * orbit_radius);
                trans.update_local_matrix();
            }
            if let Some(trans) = world.borrow_mut::<Transform>().get_mut(state.light_3_id) {
                let angle = state.time * speed + 4.188; // 240 derece ofset
                trans.position = pillar_pos + Vec3::new(angle.cos() * orbit_radius, (angle * 2.0).sin() * 2.0, angle.sin() * orbit_radius);
                trans.update_local_matrix();
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

            // Eğer fly_mode kapalıysa, ileri/geri hareketleri XZ düzleminde yap (Y'yi değiştirme)
            let move_fwd = if state.fly_mode {
                fwd
            } else {
                let mut flat = Vec3::new(fwd.x, 0.0, fwd.z);
                if flat.length_squared() > 0.001 {
                    flat = flat.normalize();
                }
                flat
            };

            if input.is_key_pressed(KeyCode::KeyW as u32) {
                state.cam_pos += move_fwd * speed * sprint;
            }
            if input.is_key_pressed(KeyCode::KeyS as u32) {
                state.cam_pos -= move_fwd * speed * sprint;
            }
            if input.is_key_pressed(KeyCode::KeyA as u32) {
                state.cam_pos -= right * speed * sprint;
            }
            if input.is_key_pressed(KeyCode::KeyD as u32) {
                state.cam_pos += right * speed * sprint;
            }

            if state.fly_mode {
                if input.is_key_pressed(KeyCode::KeyQ as u32) {
                    state.cam_pos.y -= speed * sprint;
                }
                if input.is_key_pressed(KeyCode::KeyE as u32) {
                    state.cam_pos.y += speed * sprint;
                }
            } else {
                // FPS Yürüme/Zıplama Mekaniği (Yerçekimi)
                let is_grounded = state.cam_pos.y <= 2.0;
                if is_grounded {
                    state.cam_pos.y = 2.0;
                    state.cam_velocity_y = 0.0;
                    
                    // Zıplama
                    if input.is_key_pressed(KeyCode::Space as u32) {
                        state.cam_velocity_y = 6.0; // Zıplama hızı
                    }
                } else {
                    // Havadayken yerçekimi uygula
                    state.cam_velocity_y += state.gravity_y * dt;
                }
                
                state.cam_pos.y += state.cam_velocity_y * dt;
            }

            {
                let mut trans = world.borrow_mut::<Transform>();
                if let Some(t) = trans.get_mut(state.cam_id) {
                    t.position = state.cam_pos;
                    t.rotation = pitch_yaw_quat(state.cam_pitch, state.cam_yaw);
                    t.update_local_matrix();
                }
            }

            // Atış Mekaniği (Mermi atma)
            if input.is_mouse_button_pressed(0) && state.time - state.last_shot_time > 0.1 {
                state.last_shot_time = state.time;
                
                // Mermi mermer ateşle
                let bullet = world.spawn();
                world.add_component(bullet, Transform::new(state.cam_pos).with_scale(Vec3::new(0.5, 0.5, 0.5)));
                if let Some(m) = &state.cube_mesh {
                    world.add_component(bullet, m.clone());
                }
                if let Some(t) = &state.tex {
                    world.add_component(
                        bullet,
                        Material::new(t.clone()).with_pbr(Vec4::new(1.0, 0.2, 0.0, 1.0), 0.5, 0.0), 
                    );
                }
                world.add_component(bullet, MeshRenderer::new());
                world.add_component(bullet, Collider::box_collider(Vec3::new(0.25, 0.25, 0.25)));
                let rb = RigidBody::new(5.0, 0.5, 0.5, true);
                let speed = 40.0;
                let velocity = fwd * speed;
                let vel_comp = gizmo::physics::Velocity {
                    linear: velocity,
                    angular: Vec3::new(0.0, 0.0, 0.0),
                };
                world.add_component(bullet, rb);
                world.add_component(bullet, vel_comp);
                
                // Mermiye Dinamik Işık Ekle (Gece karanlığında harika görünür)
                world.add_component(
                    bullet,
                    gizmo::renderer::components::PointLight::new(
                        Vec3::new(1.0, 0.3, 0.0), // Merminin parlak turuncu rengi
                        10.0, // Yoğunluk
                        15.0  // Yarıçap (15 metre aydınlatır)
                    )
                );
            }

            // Güç İtişi Mekaniği (Sağ Tık / Jedi Push) - Sadece Flag ayarla
            if input.is_mouse_button_pressed(1) && state.time - state.last_push_time > 0.5 {
                state.last_push_time = state.time;
                state.request_force_push = true;
            } else {
                state.request_force_push = false;
            }

            // Güç Çekimi Mekaniği (F Tuşu / Jedi Pull) - Basılı tutulduğunda çalışır
            if input.is_key_pressed(KeyCode::KeyF as u32) {
                state.request_force_pull = true;
            } else {
                state.request_force_pull = false;
            }

            // Gravity Gun / Kavrama Mekaniği (E Tuşu)
            if input.is_key_pressed(KeyCode::KeyE as u32) {
                state.request_grab = true;
            } else {
                state.request_grab = false;
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
                    ui.separator();
                    ui.label(gizmo::egui::RichText::new("Kamera Modu").strong());
                    ui.checkbox(&mut state.fly_mode, "Uçuş Modu (Hayalet)");
                    ui.separator();
                    ui.label(gizmo::egui::RichText::new("Atmosfer ve Zaman").strong());
                    ui.add(gizmo::egui::Slider::new(&mut state.time_of_day, 0.0..=24.0).text("Günün Saati"));
                    ui.separator();
                    ui.label(gizmo::egui::RichText::new("Zaman Kontrolü (Matrix Etkisi)").strong());
                    ui.add(gizmo::egui::Slider::new(&mut state.time_scale, 0.0..=2.0).text("Zaman Ölçeği (Ağır Çekim)"));
                    ui.separator();
                    ui.label(gizmo::egui::RichText::new("Sinematik Post-Process").strong());
                    ui.add(gizmo::egui::Slider::new(&mut state.bloom_intensity, 0.0..=2.0).text("Bloom Yoğunluğu"));
                    ui.add(gizmo::egui::Slider::new(&mut state.exposure, 0.1..=5.0).text("Pozlama (Exposure)"));
                    ui.add(gizmo::egui::Slider::new(&mut state.chromatic_aberration, 0.0..=5.0).text("Renk Sapması (Chromatic Ab.)"));
                    ui.add(gizmo::egui::Slider::new(&mut state.vignette_intensity, 0.0..=2.0).text("Vignette"));
                    ui.add(gizmo::egui::Slider::new(&mut state.film_grain_intensity, 0.0..=1.0).text("Film Greni"));
                    ui.separator();
                    ui.label(gizmo::egui::RichText::new("Alan Derinliği (DoF)").strong());
                    ui.add(gizmo::egui::Slider::new(&mut state.dof_focus_dist, 0.1..=100.0).text("Odak Uzaklığı"));
                    ui.add(gizmo::egui::Slider::new(&mut state.dof_focus_range, 0.1..=50.0).text("Odak Aralığı"));
                    ui.add(gizmo::egui::Slider::new(&mut state.dof_blur_size, 0.0..=10.0).text("Bulanıklık Miktarı"));
                    ui.separator();
                    ui.label(gizmo::egui::RichText::new("Parçacık Efekti (GPU Particles)").strong());
                    ui.add(gizmo::egui::Slider::new(&mut state.particle_spawn_rate, 0..=1000).text("Üretim Hızı (Kare Başına)"));
                    ui.add(gizmo::egui::Slider::new(&mut state.particle_velocity_y, 0.0..=50.0).text("Fışkırma Gücü (Y)"));
                    ui.add(gizmo::egui::Slider::new(&mut state.particle_color_r, 0.0..=1.0).text("Renk (Kırmızı)"));
                    ui.add(gizmo::egui::Slider::new(&mut state.particle_color_g, 0.0..=1.0).text("Renk (Yeşil)"));
                    ui.add(gizmo::egui::Slider::new(&mut state.particle_color_b, 0.0..=1.0).text("Renk (Mavi)"));
                    ui.separator();
                    ui.label(gizmo::egui::RichText::new("Fizik Motoru (GPU Physics)").strong());
                    ui.add(gizmo::egui::Slider::new(&mut state.gravity_y, -20.0..=20.0).text("Yerçekimi (Y)"));
                });

            // Ekranın ortasına crosshair (Hedef Göstergesi) çiz
            let screen_rect = ctx.screen_rect();
            let center = screen_rect.center();
            
            let painter = ctx.layer_painter(gizmo::egui::LayerId::new(
                gizmo::egui::Order::Foreground,
                gizmo::egui::Id::new("crosshair"),
            ));
            
            let stroke = gizmo::egui::Stroke::new(1.5, gizmo::egui::Color32::from_rgba_unmultiplied(255, 255, 255, 200));
            
            // Merkez nokta
            painter.circle_filled(center, 2.0, gizmo::egui::Color32::from_rgba_unmultiplied(255, 255, 255, 220));
            painter.circle_stroke(center, 2.5, gizmo::egui::Stroke::new(1.0, gizmo::egui::Color32::BLACK));
            
            // Artı işaretleri
            let size = 8.0;
            let gap = 4.0;
            
            // Sol
            painter.line_segment([
                gizmo::egui::pos2(center.x - size - gap, center.y),
                gizmo::egui::pos2(center.x - gap, center.y),
            ], stroke);
            // Sağ
            painter.line_segment([
                gizmo::egui::pos2(center.x + gap, center.y),
                gizmo::egui::pos2(center.x + size + gap, center.y),
            ], stroke);
            // Üst
            painter.line_segment([
                gizmo::egui::pos2(center.x, center.y - size - gap),
                gizmo::egui::pos2(center.x, center.y - gap),
            ], stroke);
            // Alt
            painter.line_segment([
                gizmo::egui::pos2(center.x, center.y + gap),
                gizmo::egui::pos2(center.x, center.y + size + gap),
            ], stroke);
        })
        .set_render(|world, state, encoder, view, renderer, _light_time| {
            // Post Processing Parametrelerini Güncelle
            renderer.update_post_process(
                &renderer.queue,
                gizmo::renderer::PostProcessUniforms {
                    bloom_intensity: state.bloom_intensity,
                    bloom_threshold: 1.0,
                    exposure: state.exposure,
                    chromatic_aberration: state.chromatic_aberration,
                    vignette_intensity: state.vignette_intensity,
                    film_grain_intensity: state.film_grain_intensity,
                    dof_focus_dist: state.dof_focus_dist,
                    dof_focus_range: state.dof_focus_range,
                    dof_blur_size: state.dof_blur_size,
                    _padding: [0.0; 3],
                },
            );

            // GPU Sistemlerini Gönder ve Oku (Fizik motoru çalışsın)
            if let Some(physics) = &renderer.gpu_physics {
                physics.update_params(&renderer.queue, 0.016 * state.time_scale, [0.0, state.gravity_y, 0.0]);
                
                // Force Push (Jedi Itişi) Uygula
                if state.request_force_push {
                    let push_radius = 20.0; // 20 metre çapındaki objeler etkilensin
                    let push_force = 60.0;
                    
                    if let Some(gpu_data) = physics.poll_readback_data(&renderer.device) {
                        for (idx, mut box_data) in gpu_data.into_iter().enumerate() {
                            let pos = Vec3::new(box_data.position[0], box_data.position[1], box_data.position[2]);
                            let dir = pos - state.cam_pos;
                            let dist = dir.length();
                            if dist > 0.1 && dist < push_radius {
                                let strength = (1.0 - (dist / push_radius)).powi(2);
                                let force = dir.normalize() * push_force * strength;
                                box_data.velocity[0] += force.x;
                                box_data.velocity[1] += force.y;
                                box_data.velocity[2] += force.z;
                                
                                box_data.angular_velocity[0] += push_force * strength * 0.1;
                                box_data.angular_velocity[1] += push_force * strength * 0.1;
                                box_data.angular_velocity[2] += push_force * strength * 0.1;

                                // Uyandır
                                box_data.sleep_counter = 0;
                                box_data.state = 0;
                                
                                physics.update_box(&renderer.queue, idx as u32, &box_data);
                            }
                        }
                    }
                }

                // Force Pull (Jedi Çekişi) Uygula
                if state.request_force_pull {
                    let pull_force = 120.0; // Güçlü çekim
                    
                    let fx = state.cam_yaw.cos() * state.cam_pitch.cos();
                    let fy = state.cam_pitch.sin();
                    let fz = state.cam_yaw.sin() * state.cam_pitch.cos();
                    let fwd = Vec3::new(fx, fy, fz).normalize();
                    
                    // Kameranın 4 birim önü
                    let target_pos = state.cam_pos + fwd * 4.0; 

                    if let Some(gpu_data) = physics.poll_readback_data(&renderer.device) {
                        for (idx, mut box_data) in gpu_data.into_iter().enumerate() {
                            let pos = Vec3::new(box_data.position[0], box_data.position[1], box_data.position[2]);
                            let to_obj_dir = pos - state.cam_pos;
                            let dist_to_cam = to_obj_dir.length();
                            
                            // Sadece kameranın önündeki objeleri çek (yaklaşık 60 derece görüş açısı)
                            let is_in_front = dist_to_cam > 0.1 && (to_obj_dir.normalize().dot(fwd) > 0.7);
                            
                            if is_in_front && dist_to_cam < 50.0 {
                                let dir_to_target = target_pos - pos;
                                let dist_to_target = dir_to_target.length();
                                
                                // Hedef noktaya (kameranın önüne) uzaksa sert çek
                                let force_mag = if dist_to_target > 3.0 { pull_force } else { pull_force * 0.1 };
                                let force = dir_to_target.normalize() * force_mag;
                                
                                // Yerdeyse, sürtünmeden kurtulması için havaya fırlatma etkisi (Lift)
                                let lift = if box_data.velocity[1].abs() < 1.0 { 30.0 } else { 0.0 };

                                // Hızlarına doğrudan ekleme yapıyoruz
                                box_data.velocity[0] += force.x * 0.016;
                                box_data.velocity[1] += (force.y + lift) * 0.016;
                                box_data.velocity[2] += force.z * 0.016;
                                
                                // SADECE hedefe ulaştıklarında fren ve yerçekimi sıfırlaması uygula ki orada havada asılı kalsınlar
                                if dist_to_target < 4.0 {
                                    box_data.velocity[0] *= 0.8;
                                    box_data.velocity[1] = box_data.velocity[1] * 0.8 + (9.81 * 0.016); // Yerçekimine karşı gelip havada tutar
                                    box_data.velocity[2] *= 0.8;
                                }
                                
                                // Biraz dönme efekti ekleyelim
                                box_data.angular_velocity[0] += 0.1;
                                box_data.angular_velocity[1] += 0.1;
                                
                                // Uyandır
                                box_data.sleep_counter = 0;
                                box_data.state = 0;
                                
                                physics.update_box(&renderer.queue, idx as u32, &box_data);
                            }
                        }
                    }
                }

                // Kavrama (Grabbing / Gravity Gun) Uygula
                let mut new_grabbed_id = state.grabbed_object_id.get();
                
                if state.request_grab {
                    let fx = state.cam_yaw.cos() * state.cam_pitch.cos();
                    let fy = state.cam_pitch.sin();
                    let fz = state.cam_yaw.sin() * state.cam_pitch.cos();
                    let fwd = Vec3::new(fx, fy, fz).normalize();
                    let target_pos = state.cam_pos + fwd * 4.0; // Tutulan obje kameranın önünde duracak

                    if let Some(gpu_data) = physics.poll_readback_data(&renderer.device) {
                        
                        // Henüz bir şey tutulmuyorsa, nişangahtaki (çok dar bir koni) en yakın objeyi bul
                        if state.grabbed_object_id.get().is_none() {
                            let mut closest_idx = None;
                            let mut min_dist = 25.0; // Maksimum tutma mesafesi (25 metre)
                            
                            for (idx, box_data) in gpu_data.iter().enumerate() {
                                let pos = Vec3::new(box_data.position[0], box_data.position[1], box_data.position[2]);
                                let to_obj_dir = pos - state.cam_pos;
                                let dist = to_obj_dir.length();
                                
                                if dist > 0.1 && dist < min_dist {
                                    // Çok dar bir açı, sadece crosshair'in tam üzerinde olanlar (0.98 çok keskin bir açıdır)
                                    if to_obj_dir.normalize().dot(fwd) > 0.98 {
                                        min_dist = dist;
                                        closest_idx = Some(idx);
                                    }
                                }
                            }
                            new_grabbed_id = closest_idx;
                        }

                        // Eğer elimizde bir obje varsa, onu target_pos'a kilitle
                        if let Some(idx) = new_grabbed_id {
                            if idx < gpu_data.len() {
                                let mut box_data = gpu_data[idx].clone();
                                let pos = Vec3::new(box_data.position[0], box_data.position[1], box_data.position[2]);
                                
                                let dir_to_target = target_pos - pos;
                                let dist = dir_to_target.length();
                                
                                // Çok uzaktaysa (duvar arkasına sıkıştıysa vs) bağlantıyı kopar
                                if dist > 15.0 {
                                    new_grabbed_id = None;
                                } else {
                                    // Hedefe sıkıca çek
                                    box_data.velocity[0] = dir_to_target.x * 12.0;
                                    box_data.velocity[1] = dir_to_target.y * 12.0 + 9.81; // Yerçekimini kesinlikle nötrle
                                    box_data.velocity[2] = dir_to_target.z * 12.0;
                                    
                                    // Dönmesini sönümleyerek elde stabil tut
                                    box_data.angular_velocity[0] *= 0.8;
                                    box_data.angular_velocity[1] *= 0.8;
                                    box_data.angular_velocity[2] *= 0.8;
                                    
                                    box_data.sleep_counter = 0;
                                    box_data.state = 0; // Uyandır
                                    
                                    physics.update_box(&renderer.queue, idx as u32, &box_data);
                                }
                            } else {
                                new_grabbed_id = None;
                            }
                        }
                    }
                } else {
                    // E tuşu bırakıldığında objeyi düşür
                    new_grabbed_id = None;
                }
                
                state.grabbed_object_id.set(new_grabbed_id);
            }
            gizmo::default_systems::gpu_physics_submit_system(world, renderer);
            gizmo::default_systems::gpu_physics_readback_system(world, renderer);
            
            // SPH Sıvı Simülasyonu
            if let Some(fluid) = &mut renderer.gpu_fluid {
                fluid.update_parameters(
                    &renderer.queue, 
                    [0.0, 0.0, 0.0], // mouse_pos
                    [0.0, 0.0, 0.0], // mouse_dir
                    false, // mouse_active
                    &[], // colliders (will be updated by gpu_fluid_coupling_system)
                    state.time,
                    fluid.num_particles
                );
            }
            gizmo::default_systems::gpu_fluid_coupling_system(world, renderer);
            
            // --- GPU Particle Spawner (Ateş / Sihir Şelalesi) ---
            if let Some(particles) = &renderer.gpu_particles {
                use rand::Rng;
                let mut rng = rand::thread_rng();
                let mut new_particles = Vec::new();
                for _ in 0..state.particle_spawn_rate {
                    let p_x = rng.gen_range(-1.0..1.0);
                    let p_z = rng.gen_range(-1.0..1.0);
                    let v_x = rng.gen_range(-3.0..3.0);
                    let v_y = rng.gen_range((state.particle_velocity_y - 5.0).max(0.0)..(state.particle_velocity_y + 5.0)); // Yukarı doğru fışkırma
                    let v_z = rng.gen_range(-3.0..3.0);
                    let life = rng.gen_range(1.0..3.0);
                    
                    new_particles.push(gizmo::renderer::gpu_particles::GpuParticle {
                        position: [p_x, 5.0, -20.0 + p_z], // Sütunun içinden çıksın
                        life: 0.0, // Yeni doğmuş
                        velocity: [v_x, v_y, v_z],
                        max_life: life,
                        color: [state.particle_color_r, state.particle_color_g, state.particle_color_b, 1.0], // Parlak renkli
                        size_start: 0.5,
                        size_end: 0.01,
                        _padding: [0.0; 2],
                    });
                }
                particles.spawn_particles(&renderer.queue, &new_particles);
            }

            gizmo::default_systems::default_render_pass(world, encoder, view, renderer);
        })
        .run();
}

fn pitch_yaw_quat(pitch: f32, yaw: f32) -> Quat {
    let q_yaw = Quat::from_axis_angle(Vec3::new(0.0, 1.0, 0.0), yaw);
    let q_pitch = Quat::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), pitch);
    q_yaw * q_pitch
}
