use gizmo::prelude::*;

struct FluidRigidDemo {
    cam_id: u32,
    cam_yaw: f32,
    cam_pitch: f32,
    cam_pos: Vec3,
    cam_speed: f32,
    fps_timer: f32,
    frames: u32,
    fps: f32,
    mouse_active: bool,
    mouse_pos: Vec3,
    mouse_dir: Vec3,
    total_time: f32,
    active_particles: u32,
}

impl FluidRigidDemo {
    fn new() -> Self {
        Self {
            cam_id: 0,
            cam_yaw: -std::f32::consts::FRAC_PI_2,
            cam_pitch: -0.2,
            cam_pos: Vec3::new(0.0, 4.0, 10.0),
            cam_speed: 5.0,
            fps_timer: 0.0,
            frames: 0,
            fps: 0.0,
            mouse_active: false,
            mouse_pos: Vec3::ZERO,
            mouse_dir: Vec3::ZERO,
            total_time: 0.0,
            active_particles: 30_000,
        }
    }
}

fn main() {
    App::<FluidRigidDemo>::new("Gizmo — Faz 7.1 Fluid-Rigid Coupling", 1600, 900)
        .set_setup(|world, _renderer| {
            println!("══════════════════════════════════════════");
            println!("   🌊 SPH Sıvı ve Katı Cisim Etkileşimi");
            println!("══════════════════════════════════════════");

            let mut state = FluidRigidDemo::new();

            // Kamera
            let cam = world.spawn();
            world.add_component(
                cam,
                Transform::new(state.cam_pos)
                    .with_rotation(pitch_yaw_quat(state.cam_pitch, state.cam_yaw)),
            );
            world.add_component(
                cam,
                Camera::new(
                    std::f32::consts::FRAC_PI_3,
                    0.1,
                    500.0,
                    state.cam_yaw,
                    state.cam_pitch,
                    true,
                ),
            );
            state.cam_id = cam.id();

            // Güneş
            let sun = world.spawn();
            world.add_component(
                sun,
                Transform::new(Vec3::new(0.0, 10.0, 5.0))
                    .with_rotation(Quat::from_axis_angle(Vec3::X, -0.8)),
            );
            world.add_component(
                sun,
                DirectionalLight::new(
                    Vec3::new(1.0, 0.95, 0.9),
                    3.5,
                    gizmo::renderer::components::LightRole::Sun,
                ),
            );

            world.insert_resource(gizmo::renderer::Gizmos::default());

            // --- FLUID KURULUMU ---
            let template = world.spawn();
            world.add_component(template, gizmo::renderer::components::FluidParticle);
            world.add_component(
                template,
                gizmo::renderer::components::FluidPhase {
                    phase: gizmo::renderer::components::FluidPhaseType::Water,
                },
            );
            world.add_component(
                template,
                gizmo::renderer::components::FluidHandle { gpu_index: 0 },
            );

            if let Some(clones) = world.clone_entity(template.id(), 100_000 - 1) {
                let mut handles = world.borrow_mut::<gizmo::renderer::components::FluidHandle>();
                for (i, clone_ent) in clones.into_iter().enumerate() {
                    if let Some(h) = handles.get_mut(clone_ent.id()) {
                        h.gpu_index = (i + 1) as u32;
                    }
                }
            }

            // --- RIGID BODY KURULUMU ---
            // Yere bir AABB koyalım (Zemin)
            let ground = world.spawn();
            world.add_component(ground, Transform::new(Vec3::new(0.0, 0.0, 0.0)));
            world.add_component(ground, Collider::box_collider(Vec3::new(2.5, 0.1, 2.5))); // Su tankının altı
            world.add_component(ground, RigidBody::new(0.0, 0.5, 0.5, false));

            // Havada dönen/sallanan birkaç küp ekleyelim
            for i in 0..5 {
                let box_ent = world.spawn();
                world.add_component(
                    box_ent,
                    Transform::new(Vec3::new(0.0, 2.0 + (i as f32) * 1.5, 0.0)),
                );
                world.add_component(box_ent, Collider::box_collider(Vec3::new(0.3, 0.3, 0.3)));
                world.add_component(box_ent, RigidBody::new(1.0, 0.5, 0.5, true));
            }

            state
        })
        .set_update(|world, state, dt, input| {
            state.total_time += dt;
            state.fps_timer += dt;
            state.frames += 1;
            if state.fps_timer >= 1.0 {
                state.fps = state.frames as f32 / state.fps_timer;
                let avg_ms = state.fps_timer / state.frames as f32 * 1000.0;
                println!("FPS: {:.1}  |  Frame: {:.2}ms", state.fps, avg_ms);
                state.frames = 0;
                state.fps_timer = 0.0;
            }

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

            let mouse_delta = input.mouse_delta();
            if input.is_mouse_button_pressed(1) {
                state.cam_yaw -= mouse_delta.0 * 0.002;
                state.cam_pitch -= mouse_delta.1 * 0.002;
                state.cam_pitch = state.cam_pitch.clamp(-1.5, 1.5);
            }

            if let Some(tr) = world.borrow_mut::<Transform>().get_mut(state.cam_id) {
                tr.rotation = pitch_yaw_quat(state.cam_pitch, state.cam_yaw);
                let forward = tr.rotation * Vec3::new(0.0, 0.0, -1.0);
                let right = forward.cross(Vec3::new(0.0, 1.0, 0.0)).normalize();
                let up = right.cross(forward).normalize();

                tr.position += right * cam_move.x + up * cam_move.y - forward * cam_move.z;
                state.cam_pos = tr.position;

                state.mouse_active = input.is_mouse_button_pressed(0);
                if state.mouse_active {
                    state.active_particles = (state.active_particles + 150).min(100_000);
                }
                state.mouse_dir = forward;

                let mut m_pos = state.cam_pos + forward * 3.0;
                m_pos.x = m_pos.x.clamp(-1.8, 1.8);
                m_pos.y = m_pos.y.clamp(0.5, 9.5);
                m_pos.z = m_pos.z.clamp(-1.8, 1.8);
                state.mouse_pos = m_pos;
            }

            if let Some(cam) = world.borrow_mut::<Camera>().get_mut(state.cam_id) {
                cam.yaw = state.cam_yaw;
                cam.pitch = state.cam_pitch;
            }

            if let Some(mut gizmos) = world.get_resource_mut::<gizmo::renderer::Gizmos>() {
                gizmos.draw_box(
                    Vec3::new(-2.0, 0.0, -2.0),
                    Vec3::new(2.0, 10.0, 2.0),
                    [0.2, 0.6, 1.0, 0.5],
                );
            }

            gizmo::systems::physics_debug_system(world);
        })
        .set_render(|world, _state, encoder, view, renderer, _light_time| {
            // 1. Fizik Gönderimi (Broadphase, Narrowphase)
            gizmo::systems::gpu_physics_submit_system(world, renderer);

            // 2. Fluid-Rigid Coupling Senkronizasyonu (Fizikten gelenleri FluidCollider'a yazar)
            gizmo::systems::gpu_fluid_coupling_system(world, renderer);

            // 3. Fluid Parametrelerini güncelle
            // (update_parameters bizim kendi FluidCollider array'imizi ezmesin diye bos liste veriyoruz)
            // Aslında gpu_fluid_coupling_system hallettiği için buradaki bos liste sorun olmayabilir!
            // Wait, update_parameters colliders listesini alır.
            // Eger bos verirsek num_colliders'i sifirlar mi?
            // "let num_colliders = (colliders.len().min(MAX_FLUID_COLLIDERS)) as u32;" -> Evet!
            // O zaman bos liste versek update_parameters içindeki write_buffer pas geçilir, AMA!
            // fluid.update_parameters içinde DynamicFluidParams struct'i num_colliders'i sifir olarak gonderir.
            // Bunu engellemek icin FluidDemo'daki update_parameters cagrisini kaldirabiliriz.
            // Veya gizmo_renderer içindeki num_colliders'ı okuyup yazarız.
            // En temizi update_parameters cagrisini modify etmek.

            gizmo::systems::default_render_pass(world, encoder, view, renderer);
        })
        .run();
}

fn pitch_yaw_quat(pitch: f32, yaw: f32) -> Quat {
    let q_yaw = Quat::from_axis_angle(Vec3::Y, yaw);
    let q_pitch = Quat::from_axis_angle(Vec3::X, pitch);
    q_yaw * q_pitch
}
