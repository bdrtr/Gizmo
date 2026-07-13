use gizmo::physics::world::PhysicsWorld;
use gizmo::prelude::*;
use super::DemoState;

pub(super) fn setup(world: &mut World, renderer: &Renderer) -> DemoState {
    // --- Geliştirici Konsolu (CVar) Kayıtları ---
    if let Some(mut registry) = world.get_resource_mut::<gizmo::core::cvar::CVarRegistry>() {
        registry.register(
            "physics_gravity_y",
            "World Gravity Y-Axis",
            gizmo::core::cvar::CVarValue::Float(-9.8),
        );
        registry.register(
            "car_torque",
            "Vehicle Engine Air Torque",
            gizmo::core::cvar::CVarValue::Float(1500.0),
        );
    }

    let mut asset_manager = AssetManager::new();

    // Textures
    let ground_tex = asset_manager
        .load_material_texture(
            &renderer.device,
            &renderer.queue,
            &renderer.scene.texture_bind_group_layout,
            "tut/assets/textures/dirt_grass.jpg",
        )
        .unwrap_or_else(|_| {
            asset_manager.create_checkerboard_texture(
                &renderer.device,
                &renderer.queue,
                &renderer.scene.texture_bind_group_layout,
            )
        });

    let car_tex = asset_manager
        .load_material_texture(
            &renderer.device,
            &renderer.queue,
            &renderer.scene.texture_bind_group_layout,
            "tut/assets/textures/rusty_metal.jpg",
        )
        .unwrap_or_else(|_| {
            asset_manager.create_checkerboard_texture(
                &renderer.device,
                &renderer.queue,
                &renderer.scene.texture_bind_group_layout,
            )
        });

    let tire_tex = asset_manager
        .load_material_texture(
            &renderer.device,
            &renderer.queue,
            &renderer.scene.texture_bind_group_layout,
            "tut/assets/textures/tire_tread.jpg",
        )
        .unwrap_or_else(|_| {
            asset_manager.create_checkerboard_texture(
                &renderer.device,
                &renderer.queue,
                &renderer.scene.texture_bind_group_layout,
            )
        });

    let white_tex = asset_manager.create_white_texture(
        &renderer.device,
        &renderer.queue,
        &renderer.scene.texture_bind_group_layout,
    );

    let cube_mesh = AssetManager::create_cube(&renderer.device);
    let cylinder_mesh = AssetManager::create_sphere(&renderer.device, 1.0, 16, 16);

    // Camera
    let camera_ent = world.spawn();
    world.add_component(camera_ent, Transform::new(Vec3::new(0.0, 5.0, 25.0)));
    world.add_component(
        camera_ent,
        Camera::new(
            std::f32::consts::FRAC_PI_4,
            0.1,
            1000.0,
            -std::f32::consts::FRAC_PI_2, // Look at -Z
            0.0,
            true,
        ),
    );
    world.add_component(camera_ent, gizmo::core::EntityName("Main Camera".into()));

    // Skybox
    let skybox = world.spawn();
    let sky_mesh = AssetManager::create_sphere(&renderer.device, 500.0, 32, 32);
    world.add_component(skybox, Transform::new(Vec3::new(0.0, 0.0, 0.0)));
    world.add_component(skybox, sky_mesh);
    world.add_component(
        skybox,
        Material::new(white_tex.clone())
            .with_unlit(Vec4::new(0.6, 0.8, 1.0, 1.0))
            .with_skybox(),
    );
    world.add_component(skybox, MeshRenderer::new());

    // Light
    let light = world.spawn();
    world.add_component(light, Transform::new(Vec3::new(0.0, 50.0, 0.0)));
    // Increased intensity slightly to make the bloom pop more
    world.add_component(
        light,
        PointLight::new(Vec3::new(1.0, 0.9, 0.8), 5000.0, 150.0),
    );

    let phys_world = PhysicsWorld::new().with_gravity(Vec3::new(0.0, -20.0, 0.0));

    // --- HILL TERRAIN ---
    let mut x_pos = -20.0;
    let y_pos = 0.0;

    // Starting platform - Make it a long straight track for building speed!
    let ground = world.spawn();
    world.add_component(
        ground,
        Transform::new(Vec3::new(60.0, y_pos - 2.0, 0.0)).with_scale(Vec3::new(80.0, 2.0, 5.0)),
    );
    world.add_component(ground, cube_mesh.clone());
    world.add_component(
        ground,
        Material::new(ground_tex.clone()).with_pbr(Vec4::new(1.0, 1.0, 1.0, 1.0), 0.8, 0.1),
    );
    world.add_component(ground, MeshRenderer::new());
    world.add_component(ground, Collider::box_collider(Vec3::new(80.0, 2.0, 5.0)));
    world.add_component(ground, RigidBody::new_static());
    world.add_component(ground, Velocity::default());

    x_pos += 140.0;

    // --- DESTRUCTIBLE BOX PYRAMID ---
    let box_mat =
        Material::new(white_tex.clone()).with_pbr(Vec4::new(0.9, 0.4, 0.1, 1.0), 0.7, 0.1); // Orange/Red boxes

    let box_size = 2.0;
    let gap = 0.05;
    let start_x = 50.0;
    let start_y = 1.0 + gap; // Top of the ground is 0.0, half extent is 1.0

    // Just spawn 3 boxes stacked on top of each other
    for i in 0..3 {
        let bx = world.spawn();
        let x = start_x;
        let y = start_y + (i as f32 * (box_size + gap));

        world.add_component(
            bx,
            Transform::new(Vec3::new(x, y, 0.0)).with_scale(Vec3::splat(box_size * 0.5)),
        );
        world.add_component(bx, cube_mesh.clone());
        world.add_component(bx, box_mat.clone());
        world.add_component(bx, MeshRenderer::new());

        let col = Collider::box_collider(Vec3::splat(box_size * 0.5));
        let mut rb = RigidBody::new(30.0, true); // 30kg, 0 bounce, high friction
        rb.linear_damping = 2.0; // High air resistance so they don't fly to infinity
        rb.angular_damping = 2.0;
        rb.update_inertia_from_collider(&col);
        rb.ccd_enabled = false;

        // 2.5D Constraints: Z ekseninde hareketi kısıtla
        rb.lock_translation_z = true;
        rb.lock_rotation_x = true;
        rb.lock_rotation_y = true;

        world.add_component(bx, col);
        world.add_component(bx, rb);
        world.add_component(bx, Velocity::default());
    }

    let num_segments = 250;
    let mut prev_pos = Vec3::new(x_pos, y_pos, 0.0);

    for i in 1..=num_segments {
        let t = i as f32;
        let x = x_pos + t * 4.0;

        // Procedural hills: mix of sine waves
        let local_x = t * 4.0;
        let y = (local_x * 0.05).sin() * 5.0
            + (local_x * 0.1).sin() * 2.0
            + (local_x * 0.02).sin() * 15.0;

        let next_pos = Vec3::new(x, y, 0.0);
        let diff = next_pos - prev_pos;
        let length = diff.length();
        let angle = diff.y.atan2(diff.x);
        let center = prev_pos + diff * 0.5;

        let hill = world.spawn();

        // Make sure it looks seamless by slightly overlapping length
        let width = length * 0.5 + 0.1;
        let rotation = Quat::from_rotation_z(angle);
        let local_down = rotation.mul_vec3(Vec3::new(0.0, -2.0, 0.0));

        let transform = Transform::new(center + local_down)
            .with_rotation(rotation)
            .with_scale(Vec3::new(width, 2.0, 5.0));

        world.add_component(hill, transform);
        world.add_component(hill, cube_mesh.clone());
        world.add_component(
            hill,
            Material::new(ground_tex.clone()).with_pbr(Vec4::new(1.0, 1.0, 1.0, 1.0), 0.9, 0.8),
        );
        world.add_component(hill, MeshRenderer::new());
        world.add_component(hill, Collider::box_collider(Vec3::new(width, 2.0, 5.0)));
        world.add_component(hill, RigidBody::new_static());
        world.add_component(hill, Velocity::default());

        prev_pos = next_pos;
    }

    // --- CAR (Using Raycast Vehicle Controller for Ultimate Stability) ---
    let car_start_pos = Vec3::new(-10.0, 3.0, 0.0);

    // --- PROCEDURAL CONVEX HULL ROCKS ---
    use rand::{Rng, SeedableRng};
    let mut rng = rand::rngs::StdRng::seed_from_u64(42);
    for i in 1..=3 {
        // Generate random points for the rock
        let mut points = Vec::new();
        let num_points = rng.gen_range(8..20);
        let rock_radius = rng.gen_range(0.5..1.5);
        for _ in 0..num_points {
            points.push(Vec3::new(
                rng.gen_range(-rock_radius..rock_radius),
                rng.gen_range(-rock_radius..rock_radius),
                rng.gen_range(-rock_radius..rock_radius),
            ));
        }

        // Generate position slightly in front of the car
        let x = car_start_pos.x + 20.0;
        let local_x = x * 0.25;
        let mut y = (local_x * 0.05).sin() * 5.0
            + (local_x * 0.1).sin() * 2.0
            + (local_x * 0.02).sin() * 15.0;

        y += rock_radius * 1.5 + (i as f32) * (rock_radius * 2.2); // Stack them

        let rock = world.spawn();
        world.add_component(rock, Transform::new(Vec3::new(x, y, 0.0)));
        // Not adding mesh because gizmo_physics debug lines will draw the convex hull!
        // We will just let physics debug rendering draw it in pink!
        world.add_component(rock, Collider::convex_hull(&points));
        world.add_component(rock, RigidBody::new(100.0, true));
        world.add_component(rock, Velocity::default());
    }

    let car_w = 1.8;
    let car_h = 1.2;
    let car_l = 4.0;

    // Chassis
    let chassis = world.spawn();
    // Rotate -90 degrees around Y so the car faces +X (Right)
    let start_rot = Quat::from_axis_angle(Vec3::Y, -std::f32::consts::FRAC_PI_2);
    let car_half_extents = Vec3::new(car_w * 0.5, car_h * 0.5, car_l * 0.5);
    world.add_component(
        chassis,
        Transform::new(car_start_pos)
            .with_rotation(start_rot)
            .with_scale(car_half_extents),
    );
    world.add_component(chassis, cube_mesh.clone());
    world.add_component(
        chassis,
        Material::new(car_tex.clone()).with_pbr(Vec4::new(1.0, 1.0, 1.0, 1.0), 0.3, 0.1),
    );
    world.add_component(chassis, MeshRenderer::new());

    let car_col = Collider::box_collider(car_half_extents);
    let mut chassis_rb = RigidBody::new(1500.0, true);
    chassis_rb.update_inertia_from_collider(&car_col);
    chassis_rb.center_of_mass = Vec3::new(0.0, -0.4, 0.0); // Ağırlık merkezini yere yaklaştır (ters takla atmasın)
    chassis_rb.ccd_enabled = false;

    // 2.5D CONSTRAINTS
    // Fizik motorundaki kilitler (locks) DÜNYA (WORLD) eksenlerine uygulanır!
    // World X = Devrilme (Roll) -> KİLİTLE
    // World Y = Sağa/Sola dönme (Yaw) -> KİLİTLE
    // World Z = Burnu kaldırıp/indirme (Pitch) -> İZİN VER! (False)
    chassis_rb.lock_translation_z = true;
    chassis_rb.lock_rotation_x = true;
    chassis_rb.lock_rotation_y = true;
    chassis_rb.lock_rotation_z = false;

    world.add_component(chassis, car_col);
    world.add_component(chassis, chassis_rb);
    world.add_component(chassis, Velocity::default());

    // Visual Wheels
    let wheel_radius = 0.4;
    let wheel_mat =
        Material::new(tire_tex.clone()).with_pbr(Vec4::new(1.0, 1.0, 1.0, 1.0), 0.8, 0.0);
    let mut wheel_entities = [chassis; 4];
    let wheel_local_pos = [
        Vec3::new(-1.0, -0.6, -1.5), // Rear Left
        Vec3::new(1.0, -0.6, -1.5),  // Rear Right
        Vec3::new(-1.0, -0.6, 1.5),  // Front Left
        Vec3::new(1.0, -0.6, 1.5),   // Front Right
    ];

    for i in 0..4 {
        let w = world.spawn();
        world.add_component(
            w,
            Transform::new(car_start_pos + wheel_local_pos[i])
                .with_scale(Vec3::splat(wheel_radius)),
        );
        world.add_component(w, cylinder_mesh.clone());
        world.add_component(w, wheel_mat.clone());
        world.add_component(w, MeshRenderer::new());
        // NO COLLIDERS or RIGIDBODIES FOR WHEELS! The VehicleController handles it perfectly.
        wheel_entities[i] = w;
    }

    let mut suspension_entities = [chassis; 4];
    let strut_mat =
        Material::new(tire_tex.clone()).with_pbr(Vec4::new(0.9, 0.2, 0.2, 1.0), 0.5, 0.8);
    for slot in suspension_entities.iter_mut() {
        let s = world.spawn();
        world.add_component(
            s,
            Transform::new(car_start_pos).with_scale(Vec3::new(0.1, 0.5, 0.1)),
        );
        world.add_component(s, cylinder_mesh.clone());
        world.add_component(s, strut_mat.clone());
        world.add_component(s, MeshRenderer::new());
        *slot = s;
    }

    // Vehicle Controller Setup
    let mut vehicle = gizmo::physics::vehicle::VehicleController::new();

    // Canavar kamyon torku (ters takla atmaması için biraz dengelendi)
    vehicle.tuning.max_rpm = 6000.0;
    vehicle.tuning.max_engine_torque = 45000.0;
    vehicle.tuning.gear_ratios = vec![-4.0, 0.0, 4.0, 2.5, 1.8]; // Low gear ratios for climbing

    for (i, &local_pos) in wheel_local_pos.iter().enumerate() {
        let axle_type = if i < 2 {
            gizmo::physics::vehicle::Axle::Rear
        } else {
            gizmo::physics::vehicle::Axle::Front
        };
        let is_left = i % 2 == 0;

        // Custom pacejka for arcade climbing
        let pacejka = gizmo::physics::vehicle::PacejkaParams {
            d: 4.0, // Devasa sürtünme (tutunma) gücü
            ..Default::default()
        };

        vehicle.add_wheel(gizmo::physics::vehicle::Wheel {
            attachment_local_pos: local_pos,
            radius: wheel_radius,
            axle_type,
            is_left,
            suspension_rest_length: 0.8, // Aracın yerden yüksekliği arttı
            suspension_max_travel: 0.7,  // Amortisör hareket mesafesi (esneme) arttı
            suspension_stiffness: 35000.0, // Daha yumuşak yaylar (1500kg aracı yumuşak taşır)
            suspension_damping: 4500.0,  // Daha az sönümleme, arazide sekmeye izin verir
            pacejka_long: pacejka.clone(),
            pacejka_lat: pacejka.clone(),
            ..Default::default()
        });
    }
    world.add_component(chassis, vehicle);

    world.insert_resource(phys_world);
    let tire_track_bg = tire_tex.clone();

    let mut engine_audio_id = None;
    let mut am = gizmo::prelude::AudioManager::new().ok();
    if let Some(audio_manager) = &mut am {
        let _ = audio_manager.load_sound("engine", "tut/assets/audio/engine.wav");
        let _ = audio_manager.load_sound("crash", "tut/assets/audio/crash.wav");
        engine_audio_id = audio_manager.play_looped("engine").ok();
        if let Some(id) = engine_audio_id {
            audio_manager.set_volume(id, 0.2); // Idle volume
        }
    }

    world.insert_resource(asset_manager);
    world.insert_resource(gizmo::renderer::Gizmos::default());

    DemoState {
        car_entity: chassis,
        phys_accum: 0.0,
        wheel_entities,
        suspension_entities,
        camera_offset: Vec3::new(0.0, 5.0, 30.0),
        post_process: gizmo::renderer::gpu_types::PostProcessUniforms {
            bloom_intensity: 1.5,
            bloom_threshold: 0.85,
            exposure: 1.2,
            chromatic_aberration: 0.005,
            vignette_intensity: 0.4,
            film_grain_intensity: 0.02,
            dof_focus_dist: 30.0,
            dof_focus_range: 20.0,
            dof_blur_size: 1.0,
            cam_near: 0.1,
            cam_far: 2000.0,
            underwater: 0.0,
            fog_r: 0.0,
            fog_g: 0.0,
            fog_b: 0.0,
            fog_density: 0.0,
        },
        pending_particles: std::cell::RefCell::new(Vec::new()),
        show_car: true,
        show_physics_debug: true,
        update_wheel_radius: None,
        tire_track_bg,
        decals: Vec::new(),
        decal_index: 0,
        pending_decals: std::cell::RefCell::new(Vec::new()),
        engine_audio_id,
        audio_manager: am,
    }
}
