use gizmo::prelude::*;

pub struct BasicSceneState {
    pub player_entity: u32,
    pub camera_entity: u32,
}

pub fn setup_basic_scene(
    world: &mut World,
    renderer: &gizmo::renderer::Renderer,
) -> BasicSceneState {
    use gizmo::core::input::ActionMap;
    use gizmo::winit::keyboard::KeyCode;

    gizmo::gizmo_log!(Info, "Basic Scene kurulumu basliyor...");
    gizmo::gizmo_log!(Warning, "Bu bir Konsol (Logger) testidir.");

    let mut action_map = ActionMap::new();
    // Yön tuşları
    action_map.bind_action("Accelerate", KeyCode::ArrowUp as u32);
    action_map.bind_action("Reverse", KeyCode::ArrowDown as u32);
    action_map.bind_action("SteerLeft", KeyCode::ArrowLeft as u32);
    action_map.bind_action("SteerRight", KeyCode::ArrowRight as u32);
    // WASD tuşları
    action_map.bind_action("Accelerate", KeyCode::KeyW as u32);
    action_map.bind_action("Reverse", KeyCode::KeyS as u32);
    action_map.bind_action("SteerLeft", KeyCode::KeyA as u32);
    action_map.bind_action("SteerRight", KeyCode::KeyD as u32);

    action_map.bind_action("Brake", KeyCode::Space as u32);
    action_map.bind_action("ShootNoCCD", KeyCode::KeyE as u32);
    action_map.bind_action("Reload", KeyCode::KeyR as u32); // R Tuşu sarjörü (ammo) doldursun
    world.insert_resource(action_map);

    world.insert_resource(gizmo::physics::components::PhysicsConfig { ground_y: -0.5 });
    world.insert_resource(gizmo::physics::JointWorld::new());
    world.insert_resource(gizmo::physics::system::PhysicsSolverState::new());

    let mut asset_manager = gizmo::renderer::asset::AssetManager::new();
    let base_tbind = asset_manager.create_white_texture(
        &renderer.device,
        &renderer.queue,
        &renderer.scene.texture_bind_group_layout,
    );

    use gizmo::physics::components::{RigidBody, Velocity};
    use gizmo::physics::shape::Collider;
    use gizmo::physics::vehicle::{VehicleController, Wheel};
    use rand::Rng;

    // ==================== ZEMİN MESHİ (Asfalt/Beton) ====================
    let floor_entity = world.spawn();
    world.add_component(floor_entity, Transform::new(Vec3::new(0.0, 0.0, 0.0)));
    world.add_component(
        floor_entity,
        gizmo::renderer::asset::AssetManager::create_plane(&renderer.device, 500.0),
    );
    let mut floor_mat =
        Material::new(base_tbind.clone()).with_pbr(Vec4::new(0.1, 0.1, 0.15, 1.0), 0.9, 0.1);
    floor_mat.is_double_sided = true;
    world.add_component(floor_entity, floor_mat);
    world.add_component(
        floor_entity,
        gizmo::renderer::components::MeshRenderer::new(),
    );
    let mut ground_rb = RigidBody::new_static();
    ground_rb.friction = 1.0;
    world.add_component(floor_entity, ground_rb);
    world.add_component(floor_entity, Collider::new_aabb(250.0, 0.1, 250.0));
    world.add_component(floor_entity, EntityName("Arena Zemin".to_string()));

    // ==================== PROCEDURAL GÖKDELENLER (NEON CITY) ====================
    let mut rng = rand::rng();
    let cube_mesh = gizmo::renderer::asset::AssetManager::create_cube(&renderer.device);

    for i in 0..40 {
        let h = rng.random_range(10.0..50.0);
        let w = rng.random_range(5.0..15.0);
        let d = rng.random_range(5.0..15.0);

        let mut rx = rng.random_range(-150.0..150.0);
        let mut rz = rng.random_range(-150.0..150.0);
        // Ortayı (0,0) boş bırak ki araba spawn olabilsin
        if rx > -20.0 && rx < 20.0 {
            rx += 40.0;
        }
        if rz > -20.0 && rz < 20.0 {
            rz += 40.0;
        }

        let b_ent = world.spawn();
        world.add_component(
            b_ent,
            Transform::new(Vec3::new(rx, h / 2.0, rz)).with_scale(Vec3::new(w, h, d)),
        );
        world.add_component(b_ent, cube_mesh.clone());

        let color = if i % 3 == 0 {
            Vec4::new(0.1, 0.8, 1.0, 1.0)
        } else {
            Vec4::new(0.2, 0.2, 0.25, 1.0)
        };
        world.add_component(
            b_ent,
            Material::new(base_tbind.clone()).with_pbr(color, 0.5, 0.8),
        );
        world.add_component(b_ent, gizmo::renderer::components::MeshRenderer::new());
        world.add_component(b_ent, EntityName(format!("Bina {}", i)));

        let mut rb = RigidBody::new_static();
        rb.friction = 0.5;
        world.add_component(b_ent, rb);
        world.add_component(b_ent, Collider::new_aabb(w / 2.0, h / 2.0, d / 2.0));
    }

    // ==================== WORLD ORIGIN GIZMO (Merkez Noktası 0,0,0) ====================
    let axis_length = 50.0;
    let axis_thickness = 0.5;

    // X EKSENİ (Kırmızı)
    let xaxis = world.spawn();
    world.add_component(
        xaxis,
        Transform::new(Vec3::new(axis_length / 2.0, 0.1, 0.0)).with_scale(Vec3::new(
            axis_length,
            axis_thickness,
            axis_thickness,
        )),
    );
    world.add_component(
        xaxis,
        gizmo::renderer::asset::AssetManager::create_cube(&renderer.device),
    );
    world.add_component(
        xaxis,
        Material::new(base_tbind.clone()).with_unlit(Vec4::new(1.0, 0.0, 0.0, 1.0)),
    );
    world.add_component(xaxis, gizmo::renderer::components::MeshRenderer::new());
    world.add_component(xaxis, EntityName("X Ekseni (Kırmızı)".to_string()));

    // Y EKSENİ (Yeşil)
    let yaxis = world.spawn();
    world.add_component(
        yaxis,
        Transform::new(Vec3::new(0.0, axis_length / 2.0, 0.0)).with_scale(Vec3::new(
            axis_thickness,
            axis_length,
            axis_thickness,
        )),
    );
    world.add_component(
        yaxis,
        gizmo::renderer::asset::AssetManager::create_cube(&renderer.device),
    );
    world.add_component(
        yaxis,
        Material::new(base_tbind.clone()).with_unlit(Vec4::new(0.0, 1.0, 0.0, 1.0)),
    );
    world.add_component(yaxis, gizmo::renderer::components::MeshRenderer::new());
    world.add_component(yaxis, EntityName("Y Ekseni (Yeşil)".to_string()));

    // Z EKSENİ (Mavi)
    let zaxis = world.spawn();
    world.add_component(
        zaxis,
        Transform::new(Vec3::new(0.0, 0.1, axis_length / 2.0)).with_scale(Vec3::new(
            axis_thickness,
            axis_thickness,
            axis_length,
        )),
    );
    world.add_component(
        zaxis,
        gizmo::renderer::asset::AssetManager::create_cube(&renderer.device),
    );
    world.add_component(
        zaxis,
        Material::new(base_tbind.clone()).with_unlit(Vec4::new(0.0, 0.0, 1.0, 1.0)),
    );
    world.add_component(zaxis, gizmo::renderer::components::MeshRenderer::new());
    world.add_component(zaxis, EntityName("Z Ekseni (Mavi)".to_string()));

    // ==================== SKYBOX ====================
    let skybox = world.spawn();
    world.add_component(
        skybox,
        Transform::new(Vec3::ZERO).with_scale(Vec3::new(500.0, 500.0, 500.0)),
    );
    world.add_component(
        skybox,
        gizmo::renderer::asset::AssetManager::create_inverted_cube(&renderer.device),
    );
    world.add_component(
        skybox,
        Material::new(base_tbind.clone()).with_unlit(Vec4::new(0.05, 0.05, 0.1, 1.0)),
    );
    world.add_component(skybox, gizmo::renderer::components::MeshRenderer::new());
    world.add_component(skybox, EntityName("Gece Gökyüzü".into()));

    // ==================== TAKİP KAMERASI ====================
    let camera_entity = world.spawn();
    world.add_component(camera_entity, Transform::new(Vec3::new(0.0, 5.0, -15.0)));
    world.add_component(
        camera_entity,
        Camera {
            fov: 75.0_f32.to_radians(),
            near: 0.5,
            far: 30000.0,
            yaw: std::f32::consts::FRAC_PI_2,
            pitch: -0.15,
            primary: true,
        },
    );

    // ==================== IŞIK (GÜNEŞ) ====================
    let sun = world.spawn();
    // Daha yatay uzanan, altın saat (Golden Hour) gölgeleri için güneşi eğiyoruz
    world.add_component(
        sun,
        Transform::new(Vec3::new(0.0, 100.0, 100.0)).with_rotation(Quat::from_axis_angle(
            Vec3::new(1.0, 0.3, 0.0).normalize(),
            -std::f32::consts::FRAC_PI_6,
        )),
    );
    world.add_component(
        sun,
        gizmo::renderer::components::DirectionalLight {
            color: Vec3::new(1.0, 0.7, 0.4), // Altın / Turuncu (Sunset) güneş rengi
            intensity: 2.5,
            is_sun: true,
        },
    );

    // ==================== RADYO / BOOMBOX (DISTANCE AUDIO TEST) ====================
    let radio_entity = world.spawn();
    world.add_component(
        radio_entity,
        Transform::new(Vec3::new(10.0, 1.0, 0.0)).with_scale(Vec3::new(1.5, 1.5, 1.5)),
    ); // Havada süzülen hafifçe büyük bir obje

    world.add_component(
        radio_entity,
        gizmo::renderer::asset::AssetManager::create_cube(&renderer.device),
    );

    // Altın sarısı/fosforlu radyo materyali
    let radio_mat = gizmo::renderer::components::Material::new(base_tbind.clone()).with_pbr(
        Vec4::new(1.0, 0.8, 0.0, 1.0),
        1.0,
        0.2,
    ); // Çok parlak, metalik
    world.add_component(radio_entity, radio_mat);
    world.add_component(
        radio_entity,
        gizmo::renderer::components::MeshRenderer::new(),
    );
    world.add_component(radio_entity, EntityName("Radyo (Boombox)".to_string()));

    // AudioSource Bileşeni: max_distance ayarlı, loop aktif
    world.add_component(
        radio_entity,
        gizmo::audio::AudioSource::new("music")
            .with_loop(true)
            .with_max_distance(30.0), // 30 metreden sonra ses duyulmaz!
    );

    // ==================== PLAYER ARAÇ (GİZMO CITY DASH) ====================
    let car = world.spawn();
    world.add_component(car, EntityName("Araç (Player)".into()));
    world.add_component(
        car,
        gizmo::renderer::asset::AssetManager::create_cube(&renderer.device),
    );
    world.add_component(
        car,
        Material::new(base_tbind.clone()).with_unlit(Vec4::new(0.0, 0.5, 1.0, 1.0)),
    ); // Mavi Taksi
    world.add_component(car, gizmo::renderer::components::MeshRenderer::new());
    world.add_component(
        car,
        Transform::new(Vec3::new(0.0, 5.0, 0.0)).with_scale(Vec3::new(1.8, 0.6, 4.0)),
    );
    let mut car_rb = RigidBody::new(1400.0, 0.1, 0.5, true);
    car_rb.ccd_enabled = true;
    world.add_component(car, car_rb);
    world.add_component(car, Velocity::new(Vec3::ZERO));
    world.add_component(car, Collider::new_aabb(0.9, 0.3, 2.0));

    let rest_len = 0.5;
    let stiffness = 60000.0;
    let damping = 4500.0;
    let wheel_radius = 0.4;
    let mut vehicle = VehicleController::new();
    vehicle.add_wheel(Wheel::new(
        Vec3::new(0.95, -0.3, 1.6),
        rest_len,
        stiffness,
        damping,
        wheel_radius,
    ));
    vehicle.add_wheel(Wheel::new(
        Vec3::new(-0.95, -0.3, 1.6),
        rest_len,
        stiffness,
        damping,
        wheel_radius,
    ));
    vehicle.add_wheel(
        Wheel::new(
            Vec3::new(0.95, -0.3, -1.6),
            rest_len,
            stiffness,
            damping,
            wheel_radius,
        )
        .with_drive(),
    );
    vehicle.add_wheel(
        Wheel::new(
            Vec3::new(-0.95, -0.3, -1.6),
            rest_len,
            stiffness,
            damping,
            wheel_radius,
        )
        .with_drive(),
    );
    world.add_component(car, vehicle);
    world.add_component(car, crate::Player);

    // ==================== ALTINLAR (COINS) ====================
    let mut rng = rand::rng();
    for i in 0..15 {
        let rx = rng.random_range(-100.0..100.0);
        let rz = rng.random_range(-100.0..100.0);
        let coin = world.spawn();
        world.add_component(
            coin,
            Transform::new(Vec3::new(rx, 1.0, rz)).with_scale(Vec3::splat(1.5)),
        );
        world.add_component(
            coin,
            gizmo::renderer::asset::AssetManager::create_sphere(&renderer.device, 0.5, 16, 16),
        );
        world.add_component(
            coin,
            Material::new(base_tbind.clone()).with_unlit(Vec4::new(1.0, 0.8, 0.0, 1.0)),
        );
        world.add_component(coin, gizmo::renderer::components::MeshRenderer::new());
        world.add_component(
            coin,
            gizmo::renderer::components::PointLight::new(Vec3::new(1.0, 0.8, 0.0), 3.0),
        );
        world.add_component(coin, crate::state::Coin);
        world.add_component(coin, EntityName(format!("Golden Coin {}", i)));
    }

    BasicSceneState {
        player_entity: car.id(), // Kamera Artık Arabayı Takip Etsin!
        camera_entity: camera_entity.id(),
    }
}
