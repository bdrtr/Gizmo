use gizmo::prelude::*;
use gizmo::renderer::gpu_physics::GpuJoint;

struct KumasGame {
    cam_id: Entity,
    cam_yaw: f32,
    cam_pitch: f32,
    cam_pos: Vec3,
    cam_speed: f32,
    fps_timer: f32,
    frames: u32,
    fps: f32,
}

impl KumasGame {
    fn new() -> Self {
        Self {
            cam_id: Entity::new(0, 0),
            cam_yaw: std::f32::consts::FRAC_PI_4,
            cam_pitch: -0.3,
            cam_pos: Vec3::new(40.0, 30.0, 40.0),
            cam_speed: 30.0,
            fps_timer: 0.0,
            frames: 0,
            fps: 0.0,
        }
    }
}

fn main() {
    // 30x30 grid = 900 node (küçük kutucuk)
    App::<KumasGame>::new("Gizmo — Faz 4.1 XPBD Kumaş Simülasyonu", 1600, 900)
        .set_setup(|world, renderer| {
            // GPU fiziği varsayılan kapalı: bu demo onunla koşuyor. Bir kez, kurulumda.
            renderer.enable_gpu_physics();
            println!("##################################################");
            println!("    XPBD Kumaş Simülasyonu Başlıyor...");
            println!("##################################################");

            let mut game = KumasGame::new();

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
                    500.0,
                    game.cam_yaw,
                    game.cam_pitch,
                    true,
                ),
            );
            game.cam_id = cam_entity;

            // Işık
            let sun = world.spawn();
            world.add_component(
                sun,
                Transform::new(Vec3::new(100.0, 100.0, 100.0)).with_rotation(
                    Quat::from_axis_angle(Vec3::new(1.0, 0.0, 1.0).normalize(), -1.0),
                ),
            );
            world.add_component(
                sun,
                DirectionalLight::new(
                    Vec3::new(1.0, 1.0, 0.95),
                    3.0,
                    gizmo::renderer::components::LightRole::Sun,
                ),
            );

            // Kumaş Ayarları
            let grid_size_x = 40;
            let grid_size_z = 40;
            let spacing = 1.0;
            let particle_mass = 0.1;

            let mut nodes = Vec::new();

            // Parçacıkları (nodes) oluştur
            for z in 0..grid_size_z {
                for x in 0..grid_size_x {
                    let pos = Vec3::new(
                        (x as f32 - grid_size_x as f32 / 2.0) * spacing,
                        40.0,
                        (z as f32 - grid_size_z as f32 / 2.0) * spacing,
                    );

                    let entity = world.spawn();
                    let is_pinned =
                        z == 0 && (x == 0 || x == grid_size_x / 2 || x == grid_size_x - 1);

                    world.add_component(entity, Transform::new(pos));
                    world.add_component(
                        entity,
                        RigidBody::new(if is_pinned { 0.0 } else { particle_mass }, true),
                    );
                    world.add_component(entity, Velocity::default());
                    world.add_component(entity, Collider::box_collider(Vec3::splat(spacing * 0.4)));

                    nodes.push(entity.id());
                }
            }

            // GpuPhysicsSystem'e joint'leri eklemek için bir update_system gerekiyor.
            // Fakat sistem her render frame'inde submit yapıyor ve entity id'leri
            // GpuBox array indekslerine eşleniyor (GpuPhysicsLink).
            // Şu anda joint'leri başlangıçta direkt GPU'ya gönderemeyiz çünkü
            // GpuPhysicsLink id'leri ancak ilk render pass sonrası belli oluyor!

            // Bu yüzden "joint" constraintlerini oluşturmayı bir "Startup System" içine
            // veya 1 frame sonra çalışacak bir flag içine koymalıyız.

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

            // Kamera Hareketi
            let mut speed = state.cam_speed;
            if input.is_key_pressed(KeyCode::ShiftLeft as u32) {
                speed *= 3.0;
            }
            // Tuşlar ve sol çubuk `Input::move_axis` ile tek yönde birleşiyor; dikey eksen
            // (Q/E) ayrı kalıyor çünkü çubukta karşılığı yok. Aşağıdaki normalize KIRPMAYA
            // döndü: normalize, yarım yatırılmış çubuğu tam hıza çıkarır ve çubuğun tek
            // kattığı şeyi — miktarı — yok ederdi. Tuşlarda davranış birebir aynı (herhangi
            // bir tuş bileşimi zaten 1 veya daha uzun).
            let (mx, my) = input.move_axis();
            let mut cam_move = Vec3::new(mx, 0.0, -my);
            if input.is_key_pressed(KeyCode::KeyQ as u32) {
                cam_move.y -= 1.0;
            }
            if input.is_key_pressed(KeyCode::KeyE as u32) {
                cam_move.y += 1.0;
            }

            let len = cam_move.length();
            if len > 1.0 {
                cam_move /= len;
            }
            cam_move *= speed * dt;

            if input.is_mouse_button_pressed(1) {
                let mouse_delta = input.mouse_delta();
                state.cam_yaw -= mouse_delta.0 * 0.002;
                state.cam_pitch -= mouse_delta.1 * 0.002;
                state.cam_pitch = state.cam_pitch.clamp(-1.5, 1.5);
            }

            if let Some(mut tr) = world.query_mut::<&mut Transform>().get_mut(state.cam_id) {
                let rot = pitch_yaw_quat(state.cam_pitch, state.cam_yaw);
                tr.rotation = rot;
                let forward = rot * Vec3::new(0.0, 0.0, -1.0);
                let right = rot * Vec3::new(1.0, 0.0, 0.0);
                let up = Vec3::new(0.0, 1.0, 0.0);
                tr.position += right * cam_move.x + up * cam_move.y - forward * cam_move.z;
                state.cam_pos = tr.position;
            }
        })
        // Kumaşın yayları senkron noktasında, oyun dünyasından okunarak yükleniyor.
        .set_render_stage(RenderWorld::new().with_settings(load_cloth_joints))
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Kumaşın yaylarını GPU fiziğine bir kez yükler: düğümler kaydedildikten sonra — ilk karenin
/// hazırlığı (`prepare_frame`) onlara `GpuPhysicsLink` veriyor — ve GPU'da henüz eklem yokken. Her
/// senkron noktasında çağrılıyor; oyun dünyasını okuyor, renderer'a yazıyor.
fn load_cloth_joints(game: &World, renderer: &mut Renderer) {
    let Some(physics) = renderer.gpu_physics.as_mut() else { return };
    if physics.joint_count != 0 {
        return;
    }
    let grid_x = 40;
    let grid_z = 40;
    let mut node_gpu_ids = vec![0; grid_x * grid_z];
    let mut found = false;
    for (link, trans) in game.query::<(&gizmo::physics::GpuPhysicsLink, &Transform)>().iter() {
        if (trans.position.y - 40.0).abs() < 1.0 {
            let x = ((trans.position.x + (grid_x as f32 / 2.0)) / 1.0).round() as i32;
            let z = ((trans.position.z + (grid_z as f32 / 2.0)) / 1.0).round() as i32;
            if x >= 0 && x < grid_x as i32 && z >= 0 && z < grid_z as i32 {
                node_gpu_ids[(z * grid_x as i32 + x) as usize] = link.id;
                found = true;
            }
        }
    }
    if !found {
        return;
    }

    let stiffness = 300.0;
    let damping = 0.5;

    for z in 0..grid_z {
        for x in 0..grid_x {
            let idx = z * grid_x + x;
            let id_a = node_gpu_ids[idx];

            // Structural
            if x < grid_x - 1 {
                let id_b = node_gpu_ids[idx + 1];
                physics.add_joint(
                    &renderer.queue,
                    GpuJoint::spring(
                        id_a,
                        id_b,
                        [0.5, 0.0, 0.0],
                        [-0.5, 0.0, 0.0],
                        stiffness,
                        damping,
                    ),
                );
            }
            if z < grid_z - 1 {
                let id_b = node_gpu_ids[idx + grid_x];
                physics.add_joint(
                    &renderer.queue,
                    GpuJoint::spring(
                        id_a,
                        id_b,
                        [0.0, 0.0, 0.5],
                        [0.0, 0.0, -0.5],
                        stiffness,
                        damping,
                    ),
                );
            }

            // Shear
            if x < grid_x - 1 && z < grid_z - 1 {
                let id_b = node_gpu_ids[idx + grid_x + 1];
                physics.add_joint(
                    &renderer.queue,
                    GpuJoint::spring(
                        id_a,
                        id_b,
                        [0.5, 0.0, 0.5],
                        [-0.5, 0.0, -0.5],
                        stiffness * 0.5,
                        damping,
                    ),
                );

                let id_c = node_gpu_ids[idx + grid_x];
                let id_d = node_gpu_ids[idx + 1];
                physics.add_joint(
                    &renderer.queue,
                    GpuJoint::spring(
                        id_c,
                        id_d,
                        [0.5, 0.0, -0.5],
                        [-0.5, 0.0, 0.5],
                        stiffness * 0.5,
                        damping,
                    ),
                );
            }
        }
    }

    println!(
        "{} Joint başarıyla GPU'ya yüklendi! XPBD aktif.",
        physics.joint_count
    );
    physics.update_params(&renderer.queue, 1.0 / 60.0, [0.0, -9.81, 0.0]);
}

fn pitch_yaw_quat(pitch: f32, yaw: f32) -> Quat {
    let q_yaw = Quat::from_axis_angle(Vec3::new(0.0, 1.0, 0.0), yaw);
    let q_pitch = Quat::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), pitch);
    q_yaw * q_pitch
}
