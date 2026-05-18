use gizmo::physics::components::{Collider, RigidBody, Transform, Velocity};
use gizmo::physics::joints::Joint;
use gizmo::physics::world::PhysicsWorld;
use gizmo::prelude::*;
use gizmo::renderer::asset::AssetManager;
use gizmo::renderer::components::{Camera, Material, MeshRenderer, PointLight};
use std::f32::consts::PI;

struct DemoState {
    camera_speed: f32,
    camera_pitch: f32,
    camera_yaw: f32,
    camera_pos: Vec3,
    time: f32,
}

fn setup(world: &mut World, renderer: &Renderer) -> DemoState {
    let mut asset_manager = AssetManager::new();

    // Kamera
    let camera_ent = world.spawn();
    world.add_component(
        camera_ent,
        Transform::new(Vec3::new(0.0, 10.0, 20.0)).with_rotation(Quat::from_rotation_x(-0.2)),
    );
    world.add_component(
        camera_ent,
        Camera::new(
            std::f32::consts::FRAC_PI_3,
            0.1,
            5000.0,
            -std::f32::consts::FRAC_PI_2,
            -0.2,
            true,
        ),
    );
    world.add_component(camera_ent, gizmo::core::EntityName("Main Camera".into()));

    // Textures
    let ground_tex = asset_manager.create_checkerboard_texture(
        &renderer.device,
        &renderer.queue,
        &renderer.scene.texture_bind_group_layout,
    );

    let box_tex = asset_manager.create_checkerboard_texture(
        &renderer.device,
        &renderer.queue,
        &renderer.scene.texture_bind_group_layout,
    );

    // Meshes
    let cube_mesh = AssetManager::create_cube(&renderer.device);
    let plane_mesh = AssetManager::create_plane(&renderer.device, 100.0);

    // Işık
    let light = world.spawn();
    world.add_component(light, Transform::new(Vec3::new(0.0, 20.0, 0.0)));
    world.add_component(
        light,
        PointLight::new(Vec3::new(1.0, 1.0, 1.0), 500.0, 50.0),
    );

    // Yer Düzlemi (Static)
    let ground = world.spawn();
    world.add_component(ground, Transform::new(Vec3::ZERO));
    world.add_component(ground, plane_mesh.clone());
    world.add_component(
        ground,
        Material::new(ground_tex).with_pbr(Vec4::new(1.0, 1.0, 1.0, 1.0), 0.8, 0.1),
    );
    world.add_component(ground, MeshRenderer::new());
    world.add_component(ground, Collider::box_collider(Vec3::new(50.0, 0.1, 50.0)));
    world.add_component(ground, RigidBody::new_static());
    world.add_component(ground, Velocity::default());

    // --- CPU PHYSICS SETUP ---
    let mut phys_world = PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0));

    phys_world
        .fluid_zones
        .push(gizmo::physics::world::FluidZone {
            shape: gizmo::physics::world::ZoneShape::Box {
                min: Vec3::new(-2.0, 0.0, -2.0),
                max: Vec3::new(2.0, 2.0, 2.0),
            },
            density: 1200.0,
            viscosity: 1.0,
            linear_drag: 5.0,
            quadratic_drag: 1.0,
        });

    phys_world.enable_gpu_compute();

    // Menteşeli Kapı veya Sarkaç (Pendulum)
    let ceiling = world.spawn();
    world.add_component(
        ceiling,
        Transform::new(Vec3::new(0.0, 15.0, -10.0)).with_scale(Vec3::new(2.0, 0.5, 2.0)),
    );
    world.add_component(ceiling, cube_mesh.clone());
    world.add_component(
        ceiling,
        Material::new(box_tex.clone()).with_pbr(Vec4::splat(1.0), 0.5, 0.0),
    );
    world.add_component(ceiling, MeshRenderer::new());
    world.add_component(ceiling, Collider::box_collider(Vec3::new(2.0, 0.5, 2.0)));
    world.add_component(ceiling, RigidBody::new_static());
    world.add_component(ceiling, Velocity::default());

    let pendulum = world.spawn();
    // Tam dengede başlasın: Tavan(15.0) - TavanAltı(0.25) - SarkaçÜstü(2.5) = 12.25
    world.add_component(
        pendulum,
        Transform::new(Vec3::new(0.0, 12.25, -10.0)).with_scale(Vec3::new(1.0, 5.0, 1.0)),
    );
    world.add_component(pendulum, cube_mesh.clone());
    world.add_component(
        pendulum,
        Material::new(box_tex.clone()).with_pbr(Vec4::new(1.0, 0.0, 0.0, 1.0), 0.5, 0.5),
    );
    world.add_component(pendulum, MeshRenderer::new());
    world.add_component(pendulum, Collider::box_collider(Vec3::new(1.0, 5.0, 1.0)));
    world.add_component(pendulum, RigidBody::new(10.0, 0.2, 0.5, true));
    world.add_component(pendulum, Velocity::default());

    let mut hinge = Joint::hinge(
        ceiling,
        pendulum,
        Vec3::new(0.0, -0.25, 0.0), // ceiling local anchor (alt yüzeyi)
        Vec3::new(0.0, 2.5, 0.0),   // pendulum local anchor (üst yüzeyi)
        Vec3::new(0.0, 0.0, 1.0),   // Z ekseni etrafında dönsün
    );
    if let gizmo::physics::joints::JointData::Hinge(ref mut data) = hinge.data {
        data.use_motor = false;
        data.use_limits = true;
        data.lower_limit = -PI / 2.0;
        data.upper_limit = PI / 2.0;
    }
    phys_world.joints.push(hinge);

    // --- BALL-SOCKET ZİNCİR (CHAIN) ---
    let chain_start = Vec3::new(-5.0, 15.0, -10.0);
    let mut prev_ent = world.spawn();
    world.add_component(
        prev_ent,
        Transform::new(chain_start).with_scale(Vec3::new(1.0, 1.0, 1.0)),
    );
    world.add_component(prev_ent, cube_mesh.clone());
    world.add_component(
        prev_ent,
        Material::new(box_tex.clone()).with_pbr(Vec4::splat(1.0), 0.5, 0.0),
    );
    world.add_component(prev_ent, MeshRenderer::new());
    world.add_component(prev_ent, Collider::box_collider(Vec3::new(1.0, 1.0, 1.0)));
    world.add_component(prev_ent, RigidBody::new_static());
    world.add_component(prev_ent, Velocity::default());

    let num_links = 6;
    for i in 1..=num_links {
        // Dümdüz aşağı sırala, kopmasınlar
        let link_pos = chain_start - Vec3::new(0.0, i as f32 * 1.0, 0.0);
        let link_ent = world.spawn();
        world.add_component(
            link_ent,
            Transform::new(link_pos).with_scale(Vec3::new(1.0, 1.0, 1.0)),
        );
        world.add_component(link_ent, cube_mesh.clone());
        world.add_component(
            link_ent,
            Material::new(box_tex.clone()).with_pbr(Vec4::new(0.0, 1.0, 0.0, 1.0), 0.5, 0.5),
        );
        world.add_component(link_ent, MeshRenderer::new());
        world.add_component(link_ent, Collider::box_collider(Vec3::new(1.0, 1.0, 1.0)));
        world.add_component(link_ent, RigidBody::new(2.0, 0.1, 0.5, true));
        world.add_component(link_ent, Velocity::default());

        let local_anchor_a = if i == 1 {
            Vec3::new(0.0, -0.5, 0.0)
        } else {
            Vec3::new(0.0, -0.5, 0.0)
        };
        let local_anchor_b = Vec3::new(0.0, 0.5, 0.0);

        let mut ball_joint = Joint::ball_socket(prev_ent, link_ent, local_anchor_a, local_anchor_b);
        if let gizmo::physics::joints::JointData::BallSocket(ref mut data) = ball_joint.data {
            data.use_cone_limit = true;
            data.cone_limit_angle = PI / 4.0; // 45 derece limit
        }
        phys_world.joints.push(ball_joint);
        prev_ent = link_ent;
    }

    // --- SLIDER JOINT (KAYAN PLATFORM) ---
    let slider_base = world.spawn();
    world.add_component(
        slider_base,
        Transform::new(Vec3::new(5.0, 5.0, -10.0)).with_scale(Vec3::new(0.5, 0.5, 0.5)),
    );
    world.add_component(slider_base, cube_mesh.clone());
    world.add_component(
        slider_base,
        Material::new(box_tex.clone()).with_pbr(Vec4::splat(1.0), 0.5, 0.0),
    );
    world.add_component(slider_base, MeshRenderer::new());
    world.add_component(
        slider_base,
        Collider::box_collider(Vec3::new(0.25, 0.25, 0.25)),
    );
    world.add_component(slider_base, RigidBody::new_static());
    world.add_component(slider_base, Velocity::default());

    let slider_plat = world.spawn();
    world.add_component(
        slider_plat,
        Transform::new(Vec3::new(5.0, 5.0, -10.0)).with_scale(Vec3::new(4.0, 0.5, 2.0)),
    );
    world.add_component(slider_plat, cube_mesh.clone());
    world.add_component(
        slider_plat,
        Material::new(box_tex.clone()).with_pbr(Vec4::new(0.0, 0.0, 1.0, 1.0), 0.5, 0.5),
    );
    world.add_component(slider_plat, MeshRenderer::new());
    world.add_component(
        slider_plat,
        Collider::box_collider(Vec3::new(2.0, 0.25, 1.0)),
    );
    world.add_component(slider_plat, RigidBody::new(20.0, 0.1, 0.5, true));
    world.add_component(slider_plat, Velocity::default());

    let mut slider_joint = Joint::slider(
        slider_base,
        slider_plat,
        Vec3::ZERO,
        Vec3::ZERO,
        Vec3::new(1.0, 0.0, 0.0), // X ekseninde kayar
    );
    if let gizmo::physics::joints::JointData::Slider(ref mut data) = slider_joint.data {
        data.use_limits = true;
        data.lower_limit = -5.0;
        data.upper_limit = 5.0;
        data.use_motor = true;
        data.motor_target_velocity = 3.0; // Kendiliğinden kaysın
        data.motor_max_force = 500.0;
    }
    phys_world.joints.push(slider_joint);

    // Serbest düşen kutular (Bir kısmı suyun içine düşecek şekilde hizalı)
    for i in 0..10 {
        let box_ent = world.spawn();
        // X, Z koordinatlarını fluid havuzu (-2..2) içerisine koyalım, Y koordinatını 5.0'dan başlatalım ki ekranda hemen görünsün.
        let pos = Vec3::new(
            (i as f32 % 3.0) - 1.0,
            5.0 + (i as f32) * 1.5,
            (i as f32 % 2.0) - 0.5,
        );
        world.add_component(box_ent, Transform::new(pos));
        world.add_component(box_ent, cube_mesh.clone());
        world.add_component(
            box_ent,
            Material::new(box_tex.clone()).with_pbr(Vec4::splat(1.0), 0.5, 0.0),
        );
        world.add_component(box_ent, MeshRenderer::new());
        world.add_component(box_ent, Collider::box_collider(Vec3::new(1.0, 1.0, 1.0)));
        // Yoğunluk suya göre ayarlandı, mass = 500.0 (ahşap yoğunluğu) ile suda gerçekçi yüzecek
        world.add_component(box_ent, RigidBody::new(500.0, 0.3, 0.8, true));
        world.add_component(box_ent, Velocity::default());
    }

    // --- GPU SOFT BODY (JELLO) ---
    let jello_ent = world.spawn();
    world.add_component(jello_ent, Transform::new(Vec3::new(0.0, 15.0, 0.0)));
    world.add_component(jello_ent, gizmo::core::EntityName("Jello Cube".into()));

    // Create a 3x3x3 Soft Body Grid
    let mut soft_body = gizmo::physics::soft_body::SoftBodyMesh::new(1000.0, 0.3); // Prevent v=0.5 singularity
    soft_body.damping = 5.0; // Higher damping for stability
    let grid_size = 3;
    let spacing = 0.8;
    let offset = (grid_size as f32 - 1.0) * spacing / 2.0;

    // Add Nodes
    for x in 0..grid_size {
        for y in 0..grid_size {
            for z in 0..grid_size {
                let pos = Vec3::new(
                    x as f32 * spacing - offset,
                    y as f32 * spacing - offset + 15.0, // Start high
                    z as f32 * spacing - offset,
                );
                soft_body.add_node(pos, 2.0); // 2kg per node
            }
        }
    }

    // Function to get 1D index
    let idx = |x, y, z| -> u32 { (x * grid_size * grid_size + y * grid_size + z) as u32 };

    // Add Tetrahedrons (Elements) - Connect adjacent nodes to form voxels
    for x in 0..grid_size - 1 {
        for y in 0..grid_size - 1 {
            for z in 0..grid_size - 1 {
                let i0 = idx(x, y, z);
                let i1 = idx(x + 1, y, z);
                let i2 = idx(x, y + 1, z);
                let i3 = idx(x, y, z + 1);
                let i4 = idx(x + 1, y + 1, z);
                let i5 = idx(x + 1, y, z + 1);
                let i6 = idx(x, y + 1, z + 1);
                let i7 = idx(x + 1, y + 1, z + 1);

                // Split voxel into 5 tetrahedrons
                soft_body.add_element(i0, i1, i2, i3);
                soft_body.add_element(i1, i4, i2, i7);
                soft_body.add_element(i1, i3, i5, i7);
                soft_body.add_element(i2, i3, i6, i7);
                soft_body.add_element(i1, i2, i3, i7);
            }
        }
    }

    world.add_component(jello_ent, soft_body);

    world.insert_resource(phys_world);
    world.insert_resource(asset_manager);
    world.insert_resource(gizmo::renderer::Gizmos::default());

    DemoState {
        camera_speed: 15.0,
        camera_pitch: -0.2,
        camera_yaw: -std::f32::consts::FRAC_PI_2,
        camera_pos: Vec3::new(0.0, 10.0, 20.0),
        time: 0.0,
    }
}

fn update(world: &mut World, state: &mut DemoState, dt: f32, input: &gizmo::core::input::Input) {
    state.time += dt;

    let mut cam_forward = Vec3::new(0.0, 0.0, -1.0);
    let mut cam_pos = Vec3::ZERO;

    if let Some(mut q) = world.query::<(
        gizmo::core::query::Mut<Transform>,
        gizmo::core::query::Mut<Camera>,
    )>() {
        for (_, (mut transform, mut camera)) in q.iter_mut() {
            let sensitivity = 0.002;
            let (dx, dy) = input.mouse_delta();

            // Sadece sağ tık basılıyken kamerayı döndür
            if input.is_mouse_button_pressed(gizmo::core::input::mouse::RIGHT) {
                state.camera_yaw -= dx * sensitivity;
                state.camera_pitch -= dy * sensitivity;
                state.camera_pitch = state.camera_pitch.clamp(-1.5, 1.5);
            }

            let fx = state.camera_yaw.cos() * state.camera_pitch.cos();
            let fy = state.camera_pitch.sin();
            let fz = state.camera_yaw.sin() * state.camera_pitch.cos();
            let forward = Vec3::new(fx, fy, fz).normalize();
            let right = forward.cross(Vec3::new(0.0, 1.0, 0.0)).normalize();
            let up = Vec3::new(0.0, 1.0, 0.0);

            // Quaternion oluştur
            let yaw_rot = Quat::from_rotation_y(-state.camera_yaw + std::f32::consts::FRAC_PI_2);
            let pitch_rot = Quat::from_rotation_x(state.camera_pitch);
            transform.rotation = yaw_rot * pitch_rot;

            let speed = state.camera_speed
                * dt
                * if input.is_key_pressed(KeyCode::ShiftLeft as u32) {
                    3.0
                } else {
                    1.0
                };

            if input.is_key_pressed(KeyCode::KeyW as u32) {
                state.camera_pos += forward * speed;
            }
            if input.is_key_pressed(KeyCode::KeyS as u32) {
                state.camera_pos -= forward * speed;
            }
            if input.is_key_pressed(KeyCode::KeyA as u32) {
                state.camera_pos -= right * speed;
            }
            if input.is_key_pressed(KeyCode::KeyD as u32) {
                state.camera_pos += right * speed;
            }
            if input.is_key_pressed(KeyCode::Space as u32) {
                state.camera_pos += up * speed;
            }

            transform.position = state.camera_pos;
            transform.update_local_matrix();

            camera.yaw = state.camera_yaw;
            camera.pitch = state.camera_pitch;

            cam_forward = forward;
            cam_pos = transform.position;
        }
    }

    // Force Push (Sol tık)
    if input.is_mouse_button_pressed(gizmo::core::input::mouse::LEFT) {
        if let Some(phys) = world.get_resource::<PhysicsWorld>() {
            let ray = gizmo::physics::raycast::Ray::new(cam_pos, cam_forward);

            if let Some(hit) = phys.raycast(&ray, 50.0) {
                if let Some(q) = world.query::<(gizmo::core::query::Mut<Velocity>, &RigidBody)>() {
                    if let Some((mut vel, rb)) = q.get(hit.entity.id()) {
                        if rb.is_dynamic() {
                            vel.linear += cam_forward * 20.0; // İleri fırlat
                        }
                    }
                }
            }
        }
    }

    // CPU Physics Adımı (Gizmo ECS entegrasyonu)
    gizmo::systems::cpu_physics_step_system(world, dt);

    // Debug draw
    gizmo::systems::physics_debug_system(world);
}

fn render(
    world: &mut World,
    _state: &DemoState,
    encoder: &mut gizmo::wgpu::CommandEncoder,
    view: &gizmo::wgpu::TextureView,
    renderer: &mut Renderer,
    _light_time: f32,
) {
    // GPU Physics sistemini devreden çıkar
    renderer.gpu_physics = None;

    // CPU objelerini render et
    gizmo::systems::default_render_pass(world, encoder, view, renderer);
}

fn main() {
    App::<DemoState>::new("Gizmo Engine - CPU Physics", 1280, 720)
        .set_setup(setup)
        .set_update(update)
        .set_render(render)
        .run();
}
