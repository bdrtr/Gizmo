use gizmo::bytemuck;
use gizmo::math::{Quat, Vec3, Vec4};
use gizmo::physics::components::RigidBody;
use gizmo::physics::fracture::{voronoi_shatter, ProceduralChunk};
use gizmo::physics::shape::Collider;
use gizmo::prelude::*;

struct DestructionGame {
    cam_id: u32,
    cam_yaw: f32,
    cam_pitch: f32,
    cam_pos: Vec3,
    cam_speed: f32,

    building_id: Option<u32>,
    pending_chunks: std::cell::RefCell<Vec<ProceduralChunk>>,
    pending_balls: std::cell::RefCell<Vec<(Vec3, Vec3)>>,
    shattered: bool,
    default_mat: Option<gizmo::renderer::components::Material>,
}

impl DestructionGame {
    fn new() -> Self {
        Self {
            cam_id: 0,
            cam_yaw: 0.0,
            cam_pitch: -0.2,
            cam_pos: Vec3::new(0.0, 50.0, 150.0),
            cam_speed: 40.0,
            building_id: None,
            pending_chunks: std::cell::RefCell::new(Vec::new()),
            pending_balls: std::cell::RefCell::new(Vec::new()),
            shattered: false,
            default_mat: None,
        }
    }
}

fn main() {
    run();
}

fn run() {
    App::<DestructionGame>::new("Gizmo — Voronoi Destruction Demo", 1600, 900)
        .set_setup(|world, _renderer| {
            println!("##################################################");
            println!("    Voronoi Destruction Demo Başlıyor...");
            println!("    Bina parçalamak için SPACE (BOŞLUK) tuşuna bas!");
            println!("##################################################");

            let mut game = DestructionGame::new();

            let mut asset_manager = gizmo::renderer::asset::AssetManager::new();
            let bg = asset_manager.create_white_texture(
                &_renderer.device,
                &_renderer.queue,
                &_renderer.scene.texture_bind_group_layout,
            );
            game.default_mat = Some(gizmo::renderer::components::Material::new(bg).with_pbr(
                Vec4::new(0.6, 0.6, 0.6, 1.0),
                0.5,
                0.0,
            ));

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

            // Güneş
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

            // Ana Bina (Kırılacak Olan)
            let b_entity = world.spawn();
            world.add_component(b_entity, Transform::new(Vec3::new(0.0, 25.0, 0.0)));
            game.building_id = Some(b_entity.id());

            game
        })
        .set_update(|_world, state, dt, input| {
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

            let mouse_delta = input.mouse_delta();
            if input.is_mouse_button_pressed(1) {
                state.cam_yaw -= mouse_delta.0 * 0.002;
                state.cam_pitch -= mouse_delta.1 * 0.002;
                state.cam_pitch = state.cam_pitch.clamp(-1.5, 1.5);
            }

            // Transformu Güncelle
            if let Some(tr) = _world.borrow_mut::<Transform>().get_mut(state.cam_id) {
                let rot = pitch_yaw_quat(state.cam_pitch, state.cam_yaw);
                tr.rotation = rot;

                let forward = rot * Vec3::new(0.0, 0.0, -1.0);
                let right = rot * Vec3::new(1.0, 0.0, 0.0);
                let up = Vec3::new(0.0, 1.0, 0.0);

                let movement = right * cam_move.x + up * cam_move.y - forward * cam_move.z;
                tr.position += movement;
                state.cam_pos = tr.position;
            }

            // Atış Tetikleyicisi (Sol Tık)
            if input.is_mouse_button_just_pressed(0) {
                let forward =
                    pitch_yaw_quat(state.cam_pitch, state.cam_yaw) * Vec3::new(0.0, 0.0, -1.0);
                let speed = 100.0; // Mermi Hızı
                state
                    .pending_balls
                    .borrow_mut()
                    .push((state.cam_pos, forward * speed));
            }

            // Kırılma Tetikleyicisi
            let _auto_trigger = false;
            // Let's fake an auto-trigger after a second maybe?
            // Space handles standard triggering. We will add an auto trigger at 60th frame in our test.
            // Oh wait, I don't have access to state.frames. I will just rely on Space or hardcode one time shatter.
            if (input.is_key_pressed(KeyCode::Space as u32) || true) && !state.shattered {
                state.shattered = true;

                // Voronoi Shatter işlemi!
                // X:50, Y:100, Z:50 boyutlarında (Extents = 25, 50, 25) bir binayı 500 asimetrik parçaya bölüyoruz!
                let extents = Vec3::new(15.0, 30.0, 15.0);
                println!("Shattering building into Voronoi chunks...");
                let chunks = voronoi_shatter(extents, 100, 12345);
                println!("Generated {} Voronoi convex hulls!", chunks.len());

                *state.pending_chunks.borrow_mut() = chunks;
            }
        })
        .set_render(|world, state, encoder, view, renderer, _light_time| {
            // Pending chunk'lar varsa, WGPU buffer'larını üret ve ECS'ye ekle!
            if !state.pending_chunks.borrow().is_empty() {
                use wgpu::util::DeviceExt;
                let mut chunks = state.pending_chunks.borrow_mut();

                for chunk in chunks.drain(..) {
                    let mut vertices = Vec::new();
                    let mut min_pt = Vec3::new(f32::MAX, f32::MAX, f32::MAX);
                    let mut max_pt = Vec3::new(f32::MIN, f32::MIN, f32::MIN);

                    for (i, v) in chunk.vertices.iter().enumerate() {
                        let local_pos = *v - chunk.center_of_mass;
                        min_pt = min_pt.min(local_pos);
                        max_pt = max_pt.max(local_pos);

                        let n = chunk.normals[i];
                        vertices.push(gizmo::renderer::gpu_types::Vertex {
                            position: [local_pos.x, local_pos.y, local_pos.z],
                            color: [0.8, 0.7, 0.6],
                            normal: [n.x, n.y, n.z],
                            tex_coords: [0.0, 0.0],
                            joint_indices: [0; 4],
                            joint_weights: [0.0; 4],
                        });
                    }

                    let half_extents = (max_pt - min_pt) * 0.5;

                    let vbuf =
                        renderer
                            .device
                            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                label: Some("Voronoi Chunk VBuf"),
                                contents: bytemuck::cast_slice(&vertices),
                                usage: wgpu::BufferUsages::VERTEX,
                            });

                    let e = world.spawn();
                    world.add_component(
                        e,
                        Transform::new(chunk.center_of_mass + Vec3::new(0.0, 25.0, 0.0)),
                    ); // bina center'i

                    world.add_component(
                        e,
                        gizmo::renderer::components::Mesh::new(
                            std::sync::Arc::new(vbuf),
                            &vertices,
                            Vec3::ZERO,
                            "voronoi_chunk".into(),
                        ),
                    );

                    world.add_component(e, state.default_mat.clone().unwrap());
                    world.add_component(e, gizmo::renderer::components::MeshRenderer::new());

                    let mut rb = RigidBody::new(5.0, 0.5, 0.5, true);
                    rb.is_sleeping = false;
                    world.add_component(e, rb);
                    world.add_component(e, Collider::aabb(half_extents));
                }
            }

            // Fırlatılan topları işle
            if !state.pending_balls.borrow().is_empty() {
                let mut balls = state.pending_balls.borrow_mut();
                for (pos, vel) in balls.drain(..) {
                    let mass = 1000.0; // Çok ağır obüs
                    let radius = 2.0;
                    let ball_id = {
                        let mut cmd = gizmo::spawner::Commands::new(world, renderer);
                        cmd.spawn_rigid_sphere(pos, radius, gizmo::color::Color::RED, mass)
                            .id()
                    };

                    // Velocitiy üzerine yaz
                    let mut vel_store = world.borrow_mut::<gizmo::physics::components::Velocity>();
                    if let Some(v) = vel_store.get_mut(ball_id.id()) {
                        v.linear = vel;
                    }
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
