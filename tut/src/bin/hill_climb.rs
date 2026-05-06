use gizmo::prelude::*;
use gizmo::physics::components::{Collider, RigidBody, Transform, Velocity};
use gizmo::physics::world::PhysicsWorld;
use gizmo::renderer::asset::AssetManager;
use gizmo::renderer::components::{Camera, Material, MeshRenderer, PointLight};

struct PendingDecal {
    position: Vec3,
    rotation: Quat,
}

struct DemoState {
    car_entity: gizmo::core::Entity,
    wheel_entities: [gizmo::core::Entity; 4],
    suspension_entities: [gizmo::core::Entity; 4],
    camera_offset: Vec3,
    post_process: gizmo::renderer::gpu_types::PostProcessUniforms,
    pending_particles: std::cell::RefCell<Vec<gizmo::renderer::gpu_particles::GpuParticle>>,
    show_car: bool,
    show_physics_debug: bool,
    update_wheel_radius: Option<f32>,
    tire_track_bg: std::sync::Arc<wgpu::BindGroup>,
    decals: Vec<gizmo::core::Entity>,
    decal_index: usize,
    pending_decals: std::cell::RefCell<Vec<PendingDecal>>,
    engine_audio_id: Option<u64>,
    audio_manager: Option<gizmo::prelude::AudioManager>,
}

fn setup(world: &mut World, renderer: &Renderer) -> DemoState {
    // --- Geliştirici Konsolu (CVar) Kayıtları ---
    if let Some(mut registry) = world.get_resource_mut::<gizmo::core::cvar::CVarRegistry>() {
        registry.register("physics_gravity_y", "World Gravity Y-Axis", gizmo::core::cvar::CVarValue::Float(-9.8));
        registry.register("car_torque", "Vehicle Engine Air Torque", gizmo::core::cvar::CVarValue::Float(1500.0));
    }

    let mut asset_manager = AssetManager::new();

    // Textures
    let ground_tex = asset_manager.load_material_texture(
        &renderer.device,
        &renderer.queue,
        &renderer.scene.texture_bind_group_layout,
        "tut/assets/textures/dirt_grass.jpg"
    ).unwrap_or_else(|_| asset_manager.create_checkerboard_texture(&renderer.device, &renderer.queue, &renderer.scene.texture_bind_group_layout));

    let car_tex = asset_manager.load_material_texture(
        &renderer.device,
        &renderer.queue,
        &renderer.scene.texture_bind_group_layout,
        "tut/assets/textures/rusty_metal.jpg"
    ).unwrap_or_else(|_| asset_manager.create_checkerboard_texture(&renderer.device, &renderer.queue, &renderer.scene.texture_bind_group_layout));
    
    let tire_tex = asset_manager.load_material_texture(
        &renderer.device,
        &renderer.queue,
        &renderer.scene.texture_bind_group_layout,
        "tut/assets/textures/tire_tread.jpg"
    ).unwrap_or_else(|_| asset_manager.create_checkerboard_texture(&renderer.device, &renderer.queue, &renderer.scene.texture_bind_group_layout));
    
    let white_tex = asset_manager.create_white_texture(
        &renderer.device,
        &renderer.queue,
        &renderer.scene.texture_bind_group_layout,
    );

    let cube_mesh = AssetManager::create_cube(&renderer.device);
    let cylinder_mesh = AssetManager::create_sphere(&renderer.device, 1.0, 16, 16); 

    // Camera
    let camera_ent = world.spawn();
    world.add_component(
        camera_ent,
        Transform::new(Vec3::new(0.0, 5.0, 25.0)),
    );
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
    world.add_component(light, PointLight::new(Vec3::new(1.0, 0.9, 0.8), 5000.0, 150.0));

    let phys_world = PhysicsWorld::new().with_gravity(Vec3::new(0.0, -20.0, 0.0));

    // --- HILL TERRAIN ---
    let mut x_pos = -20.0;
    let y_pos = 0.0;
    
    // Starting platform - Make it a long straight track for building speed!
    let ground = world.spawn();
    world.add_component(ground, Transform::new(Vec3::new(60.0, y_pos - 2.0, 0.0)).with_scale(Vec3::new(80.0, 2.0, 5.0)));
    world.add_component(ground, cube_mesh.clone());
    world.add_component(ground, Material::new(ground_tex.clone()).with_pbr(Vec4::new(1.0, 1.0, 1.0, 1.0), 0.8, 0.1));
    world.add_component(ground, MeshRenderer::new());
    world.add_component(ground, Collider::box_collider(Vec3::new(80.0, 2.0, 5.0)));
    world.add_component(ground, RigidBody::new_static());
    world.add_component(ground, Velocity::default());
    
    x_pos += 140.0;

    // --- DESTRUCTIBLE BOX PYRAMID ---
    let box_mat = Material::new(white_tex.clone())
        .with_pbr(Vec4::new(0.9, 0.4, 0.1, 1.0), 0.7, 0.1); // Orange/Red boxes

    let box_size = 2.0; 
    let gap = 0.05; 
    let start_x = 50.0; 
    let start_y = 1.0 + gap; // Top of the ground is 0.0, half extent is 1.0

    // Just spawn 3 boxes stacked on top of each other
    for i in 0..3 {
        let bx = world.spawn();
        let x = start_x;
        let y = start_y + (i as f32 * (box_size + gap));
        
        world.add_component(bx, Transform::new(Vec3::new(x, y, 0.0)).with_scale(Vec3::splat(box_size * 0.5)));
        world.add_component(bx, cube_mesh.clone());
        world.add_component(bx, box_mat.clone());
        world.add_component(bx, MeshRenderer::new());
        
        let col = Collider::box_collider(Vec3::splat(box_size * 0.5));
        let mut rb = RigidBody::new(30.0, 0.0, 0.8, true); // 30kg, 0 bounce, high friction
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
        world.add_component(hill, Material::new(ground_tex.clone()).with_pbr(Vec4::new(1.0, 1.0, 1.0, 1.0), 0.9, 0.8));
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
        world.add_component(rock, RigidBody::new(100.0, 0.3, 0.8, true));
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
    world.add_component(chassis, Transform::new(car_start_pos).with_rotation(start_rot).with_scale(car_half_extents));
    world.add_component(chassis, cube_mesh.clone());
    world.add_component(chassis, Material::new(car_tex.clone()).with_pbr(Vec4::new(1.0, 1.0, 1.0, 1.0), 0.3, 0.1));
    world.add_component(chassis, MeshRenderer::new());
    
    let car_col = Collider::box_collider(car_half_extents);
    let mut chassis_rb = RigidBody::new(1500.0, 0.1, 0.5, true);
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
    let wheel_mat = Material::new(tire_tex.clone()).with_pbr(Vec4::new(1.0, 1.0, 1.0, 1.0), 0.8, 0.0);
    let mut wheel_entities = [chassis; 4];
    let wheel_local_pos = [
        Vec3::new(-1.0, -0.6, -1.5),  // Rear Left
        Vec3::new(1.0, -0.6, -1.5),   // Rear Right
        Vec3::new(-1.0, -0.6, 1.5),   // Front Left
        Vec3::new(1.0, -0.6, 1.5),    // Front Right
    ];

    for i in 0..4 {
        let w = world.spawn();
        world.add_component(w, Transform::new(car_start_pos + wheel_local_pos[i]).with_scale(Vec3::splat(wheel_radius)));
        world.add_component(w, cylinder_mesh.clone());
        world.add_component(w, wheel_mat.clone());
        world.add_component(w, MeshRenderer::new());
        // NO COLLIDERS or RIGIDBODIES FOR WHEELS! The VehicleController handles it perfectly.
        wheel_entities[i] = w;
    }

    let mut suspension_entities = [chassis; 4];
    let strut_mat = Material::new(tire_tex.clone()).with_pbr(Vec4::new(0.9, 0.2, 0.2, 1.0), 0.5, 0.8);
    for i in 0..4 {
        let s = world.spawn();
        world.add_component(s, Transform::new(car_start_pos).with_scale(Vec3::new(0.1, 0.5, 0.1)));
        world.add_component(s, cylinder_mesh.clone());
        world.add_component(s, strut_mat.clone());
        world.add_component(s, MeshRenderer::new());
        suspension_entities[i] = s;
    }

    // Vehicle Controller Setup
    let mut vehicle = gizmo::physics::vehicle::VehicleController::new();
    
    // Canavar kamyon torku (ters takla atmaması için biraz dengelendi)
    vehicle.tuning.max_rpm = 6000.0;
    vehicle.tuning.max_engine_torque = 45000.0; 
    vehicle.tuning.gear_ratios = vec![-4.0, 0.0, 4.0, 2.5, 1.8]; // Low gear ratios for climbing

    for i in 0..4 {
        let axle_type = if i < 2 { gizmo::physics::vehicle::Axle::Rear } else { gizmo::physics::vehicle::Axle::Front };
        let is_left = i % 2 == 0;
        
        // Custom pacejka for arcade climbing
        let mut pacejka = gizmo::physics::vehicle::PacejkaParams::default();
        pacejka.d = 4.0; // Devasa sürtünme (tutunma) gücü

        vehicle.add_wheel(gizmo::physics::vehicle::Wheel {
            attachment_local_pos: wheel_local_pos[i],
            radius: wheel_radius,
            axle_type,
            is_left,
            suspension_rest_length: 0.8,   // Aracın yerden yüksekliği arttı
            suspension_max_travel: 0.7,    // Amortisör hareket mesafesi (esneme) arttı
            suspension_stiffness: 35000.0, // Daha yumuşak yaylar (1500kg aracı yumuşak taşır)
            suspension_damping: 4500.0,    // Daha az sönümleme, arazide sekmeye izin verir
            pacejka_long: pacejka.clone(),
            pacejka_lat: pacejka.clone(),
            ..Default::default()
        });
    }
    world.add_component(chassis, vehicle);

    world.insert_resource(phys_world);
    let tire_track_bg = tire_tex.clone();

    let mut engine_audio_id = None;
    let mut am = gizmo::prelude::AudioManager::new();
    if let Some(audio_manager) = &mut am {
        let _ = audio_manager.load_sound("engine", "tut/assets/audio/engine.wav");
        let _ = audio_manager.load_sound("crash", "tut/assets/audio/crash.wav");
        engine_audio_id = audio_manager.play_looped("engine");
        if let Some(id) = engine_audio_id {
            audio_manager.set_volume(id, 0.2); // Idle volume
        }
    }

    world.insert_resource(asset_manager);
    world.insert_resource(gizmo::renderer::Gizmos::default());

    DemoState {
        car_entity: chassis,
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
            _padding: [0.0; 3],
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

fn update(world: &mut World, state: &mut DemoState, dt: f32, input: &gizmo::core::input::Input) {
    let mut is_console_open = false;
    if let Some(console_state) = world.get_resource::<gizmo::core::cvar::DevConsoleState>() {
        is_console_open = console_state.is_open;
    }

    let mut throttle = 0.0;
    let mut brake = 0.0;
    let mut air_torque = 0.0;

    if !is_console_open {
        if input.is_key_pressed(KeyCode::KeyW as u32) || input.is_key_pressed(KeyCode::KeyD as u32) || input.is_key_pressed(KeyCode::ArrowUp as u32) || input.is_key_pressed(KeyCode::ArrowRight as u32) {
            throttle = 1.0;
            air_torque = 1500.0; // Lean back (+Z rotation is nose up when facing right)
        } else if input.is_key_pressed(KeyCode::KeyS as u32) || input.is_key_pressed(KeyCode::KeyA as u32) || input.is_key_pressed(KeyCode::ArrowDown as u32) || input.is_key_pressed(KeyCode::ArrowLeft as u32) {
            brake = 1.0;
            air_torque = -1500.0; // Lean forward (-Z rotation is nose down when facing right)
        }
    }

    // --- CVAR (Developer Console) UYGULAMASI ---
    if let Some(registry) = world.get_resource::<gizmo::core::cvar::CVarRegistry>() {
        // Yerçekimini canlı güncelle
        if let Some(gizmo::core::cvar::CVarValue::Float(g)) = registry.get("physics_gravity_y") {
            if let Ok(mut phys) = world.try_get_resource_mut::<gizmo::physics::world::PhysicsWorld>() {
                phys.integrator.gravity = Vec3::new(0.0, *g, 0.0);
            }
        }
        
        // Tork gücünü canlı güncelle
        if let Some(gizmo::core::cvar::CVarValue::Float(t)) = registry.get("car_torque") {
            if throttle > 0.0 { air_torque = *t; }
            else if brake > 0.0 { air_torque = -*t; }
        }
    }

    // --- PHYSICS DEBUG CONTROLS ---
    if !is_console_open {
        if let Ok(mut phys_world) = world.try_get_resource_mut::<gizmo::physics::world::PhysicsWorld>() {
            // P: Pause toggle
            if input.is_key_just_pressed(KeyCode::KeyP as u32) {
                phys_world.is_paused = !phys_world.is_paused;
                tracing::warn!("Physics Paused: {}", phys_world.is_paused);
            }
            
            // O: Step once (when paused)
            if input.is_key_pressed(KeyCode::KeyO as u32) {
                phys_world.step_once = true;
                tracing::warn!("Physics Step Triggered");
            }
            
            // R: Rewind time! (Hold to continuously rewind)
            if input.is_key_pressed(KeyCode::KeyR as u32) {
                phys_world.rewind_requested = true;
            }
        }
    }

    // Air Control and Inputs
    if let Some(q_v) = world.query::<gizmo::core::query::Mut<gizmo::physics::vehicle::VehicleController>>() {
        if let Some(mut vehicle) = q_v.get(state.car_entity.id()) {
            let is_reverse = brake > 0.0 && vehicle.current_speed_kmh < 1.0;
            vehicle.set_reverse(is_reverse);

            if vehicle.reverse_input {
                // In reverse, the S key (brake) acts as the gas pedal
                vehicle.throttle_input = brake;
                vehicle.brake_input = throttle; // W key becomes the brake when reversing
            } else {
                vehicle.throttle_input = throttle;
                vehicle.brake_input = brake;
            }
            
            if throttle > 0.0 || brake > 0.0 {
                if let Some(q_rb) = world.query::<gizmo::core::query::Mut<RigidBody>>() {
                    if let Some(mut rb) = q_rb.get(state.car_entity.id()) {
                        rb.wake_up();
                    }
                }
            }
            
            // Check if grounded to apply air control
            let is_grounded = vehicle.wheels.iter().any(|w| w.is_grounded);
            if !is_grounded && air_torque != 0.0 {
                if let Some(q_rb) = world.query::<gizmo::core::query::Mut<Velocity>>() {
                    if let Some(mut vel) = q_rb.get(state.car_entity.id()) {
                        // Directly applying torque scaled by dt
                        vel.angular.z += air_torque * dt * 0.005;
                    }
                }
            }
        }
    }

    // Step physics
    gizmo::systems::cpu_physics_step_system(world, dt);

    if state.show_physics_debug {
        gizmo::systems::physics::physics_debug_system(world);
    }

    // Sync Visual Wheels and Spawns Particles
    let mut car_pos = Vec3::ZERO;
    let mut car_rot = Quat::IDENTITY;
    if let Some(q) = world.query::<&Transform>() {
        if let Some(t) = q.get(state.car_entity.id()) {
            car_pos = t.position;
            car_rot = t.rotation;
        }
    }

    if let Some(q_v) = world.query::<&gizmo::physics::vehicle::VehicleController>() {
        if let Some(vehicle) = q_v.get(state.car_entity.id()) {
            let speed = vehicle.current_speed_kmh;
            let mut pending = state.pending_particles.borrow_mut();

            if let Some(q_t) = world.query::<gizmo::core::query::Mut<Transform>>() {
                for i in 0..4 {
                    let wheel = &vehicle.wheels[i];
                    let anchor_world = car_pos + car_rot.mul_vec3(wheel.attachment_local_pos);
                    let up = car_rot.mul_vec3(Vec3::new(0.0, 1.0, 0.0));
                    let wheel_world_pos = anchor_world - up * wheel.suspension_length;

                    if let Some(mut wt) = q_t.get(state.wheel_entities[i].id()) {
                        wt.set_position(wheel_world_pos);
                        
                        let align_rot = Quat::from_rotation_z(-std::f32::consts::FRAC_PI_2);
                        let spin_rot = Quat::from_rotation_x(wheel.rotation_angle);
                        wt.set_rotation(car_rot * spin_rot * align_rot);

                        if let Some(new_radius) = state.update_wheel_radius {
                            wt.set_scale(Vec3::splat(new_radius));
                        }
                    }

                    if let Some(mut st) = q_t.get(state.suspension_entities[i].id()) {
                        st.set_position((anchor_world + wheel_world_pos) * 0.5);
                        let mut scale = st.scale;
                        scale.y = wheel.suspension_length * 0.5;
                        st.set_scale(scale);
                        st.set_rotation(car_rot); // Keep strut upright
                    }

                    // Dust particles ONLY for grounded wheels
                    if wheel.is_grounded && (speed > 5.0 || (speed < 5.0 && (throttle > 0.0 || brake > 0.0))) {
                        let pos_bottom = wheel_world_pos - Vec3::new(0.0, wheel.radius * 0.9, 0.0);
                            for _ in 0..2 {
                                let vx = (rand::random::<f32>() - 0.5) * 1.5; // less horizontal spread
                                let vy = rand::random::<f32>() * 1.5 + 0.5; // gentle lift
                                let vz = (rand::random::<f32>() - 0.5) * 1.5;
                                
                                pending.push(gizmo::renderer::gpu_particles::GpuParticle {
                                    position: [pos_bottom.x, pos_bottom.y, pos_bottom.z],
                                    life: 0.4 + rand::random::<f32>() * 0.3, // Shorter life
                                    velocity: [vx, vy, vz],
                                    max_life: 0.8,
                                    color: [0.55, 0.45, 0.35, 0.5], // Brownish dust
                                    size_start: 0.2 + rand::random::<f32>() * 0.2, // Very small
                                    size_end: 0.8 + rand::random::<f32>() * 0.4, // Small dissipation
                                    _padding: [0.0; 2],
                                });
                            }
                            
                            // If slipping heavily or braking hard, spawn a tire track
                            if brake > 0.0 || (throttle > 0.0 && speed < 10.0) {
                                state.pending_decals.borrow_mut().push(PendingDecal {
                                    position: pos_bottom,
                                    rotation: Quat::from_rotation_y(car_rot.to_euler(EulerRot::YXZ).0),
                                });
                            }
                    }
                }
            }
        }
    }

    let mut car_vel = Vec3::ZERO;
    if let Some(q) = world.query::<&Velocity>() {
        if let Some(v) = q.get(state.car_entity.id()) {
            car_vel = v.linear;
        }
    }

    // Camera follow
    if let Some(mut q) = world.query::<(gizmo::core::query::Mut<Transform>, &Camera)>() {
        for (_, (mut cam_trans, _)) in q.iter_mut() {
            let speed = car_vel.length();
            let look_ahead = if speed > 1.0 { (car_vel / speed) * (speed * 0.3).min(15.0) } else { Vec3::ZERO };
            let zoom_out = (speed * 0.2).min(15.0);
            
            let mut dynamic_offset = state.camera_offset;
            dynamic_offset.z += zoom_out;
            dynamic_offset.y += zoom_out * 0.2;
            
            let desired_pos = car_pos + look_ahead + dynamic_offset;
            cam_trans.position = cam_trans.position.lerp(desired_pos, dt * 3.0);
            cam_trans.update_local_matrix();
        }
    }

    let pending_decals = state.pending_decals.replace(Vec::new());
    if !pending_decals.is_empty() {
        for pd in pending_decals {
            if state.decals.len() < 400 {
                let decal = world.spawn();
                world.add_component(decal, Transform::new(pd.position)
                    .with_scale(Vec3::new(1.0, 2.0, 1.0)) // Height must be enough to hit the ground
                    .with_rotation(pd.rotation)
                );
                world.add_component(decal, gizmo::renderer::components::Decal::new(
                    state.tire_track_bg.clone(),
                    Vec4::new(0.05, 0.05, 0.05, 0.9),
                ));
                state.decals.push(decal);
            } else {
                let decal = state.decals[state.decal_index];
                if let Some(q_t) = world.query::<gizmo::core::query::Mut<Transform>>() {
                    if let Some(mut dt) = q_t.get(decal.id()) {
                        dt.position = pd.position;
                        dt.rotation = pd.rotation;
                    }
                }
                state.decal_index = (state.decal_index + 1) % 400;
            }
        }
    }

    // Audio Update
    if let Some(audio_manager) = &mut state.audio_manager {
        audio_manager.update();

        // Engine Pitch
        if let Some(id) = state.engine_audio_id {
            let speed = car_vel.length();
            let base_pitch = 0.5; // Idle
            let target_pitch = base_pitch + (speed * 0.05) + (throttle * 0.4).max(brake * 0.4);
            audio_manager.set_pitch(id, target_pitch);
            
            let volume = 0.1 + (throttle * 0.3).max(brake * 0.3) + (speed * 0.01).min(0.2);
            audio_manager.set_volume(id, volume);
        }

        // Crash sounds
        if let Ok(phys_world) = world.try_get_resource::<PhysicsWorld>() {
            for event in phys_world.collision_events() {
                // If collision is strong enough
                let max_impulse = event.contact_points.iter().map(|c| c.normal_impulse).fold(0.0_f32, f32::max);
                if max_impulse > 1000.0 { // arbitrary threshold
                    if let Some(id) = audio_manager.play("crash") {
                        let vol = (max_impulse / 10000.0).clamp(0.1, 1.0);
                        audio_manager.set_volume(id, vol);
                    }
                }
            }
        }
    }

    state.update_wheel_radius = None;
}

fn render(
    world: &mut World,
    state: &DemoState,
    encoder: &mut gizmo::wgpu::CommandEncoder,
    view: &gizmo::wgpu::TextureView,
    renderer: &mut Renderer,
    _light_time: f32,
) {
    renderer.gpu_physics = None;
    renderer.update_post_process(&renderer.queue, state.post_process);
    
    let mut pending = state.pending_particles.borrow_mut();
    if !pending.is_empty() {
        if let Some(gpu_particles) = &renderer.gpu_particles {
            gpu_particles.spawn_particles(&renderer.queue, &pending);
        }
        pending.clear();
    }
    
    gizmo::systems::default_render_pass(world, encoder, view, renderer);
}

fn ui_debug_panel(world: &mut World, state: &mut DemoState, ctx: &gizmo::egui::Context) {
    gizmo::egui::Window::new("🛠 Gizmo Debugger")
        .default_pos([10.0, 10.0])
        .show(ctx, |ui| {
            // --- METRICS ---
            ui.heading("Performance");
            if let Ok(time) = world.try_get_resource::<gizmo::core::time::Time>() {
                ui.label(format!("FPS: {:.0}", time.fps()));
                ui.label(format!("Frame Time: {:.2} ms", time.raw_dt() * 1000.0));
            }
            ui.separator();
            
            // --- PHYSICS ---
            if let Ok(mut phys) = world.try_get_resource_mut::<gizmo::physics::world::PhysicsWorld>() {
                ui.heading("Physics Engine");
                ui.checkbox(&mut state.show_physics_debug, "Gizmo Debug Draw (Görsel Çarpışma)");
                ui.horizontal(|ui| {
                    ui.checkbox(&mut phys.is_paused, "Pause (P)");
                    if ui.button("Step (O)").clicked() {
                        phys.step_once = true;
                    }
                    if ui.button("Rewind (R)").clicked() {
                        phys.rewind_requested = true;
                    }
                });
                ui.label(format!("Active Rigidbodies: {}", phys.rigid_bodies.len()));
                ui.label(format!("History Buffer: {} / {}", phys.history.len(), phys.max_history_frames));
            }
            
            ui.separator();
            ui.label("Use W/A/S/D to drive");
            
            ui.separator();
            ui.heading("Araba Görünürlüğü");
            let mut show_car = state.show_car;
            if ui.checkbox(&mut show_car, "Arabayı Göster").changed() {
                state.show_car = show_car;
                if show_car {
                    world.add_component(state.car_entity, MeshRenderer::new());
                    for w in &state.wheel_entities {
                        world.add_component(*w, MeshRenderer::new());
                    }
                } else {
                    world.remove_component::<MeshRenderer>(state.car_entity);
                    for w in &state.wheel_entities {
                        world.remove_component::<MeshRenderer>(*w);
                    }
                }
            }
            
            ui.separator();
            ui.heading("Görsel Kalite / Post-Process");
            ui.add(gizmo::egui::Slider::new(&mut state.post_process.bloom_intensity, 0.0..=5.0).text("Bloom Yoğunluğu"));
            ui.add(gizmo::egui::Slider::new(&mut state.post_process.bloom_threshold, 0.0..=2.0).text("Bloom Eşiği"));
            ui.add(gizmo::egui::Slider::new(&mut state.post_process.exposure, 0.1..=5.0).text("Exposure (Pozlama)"));
            ui.add(gizmo::egui::Slider::new(&mut state.post_process.chromatic_aberration, 0.0..=0.05).text("Kromatik Sapma"));
            ui.add(gizmo::egui::Slider::new(&mut state.post_process.vignette_intensity, 0.0..=1.0).text("Vignette"));
            ui.add(gizmo::egui::Slider::new(&mut state.post_process.film_grain_intensity, 0.0..=0.5).text("Film Greni"));
            
            ui.label("Depth of Field (Alan Derinliği)");
            ui.add(gizmo::egui::Slider::new(&mut state.post_process.dof_focus_dist, 0.0..=100.0).text("Odak Uzaklığı"));
            ui.add(gizmo::egui::Slider::new(&mut state.post_process.dof_focus_range, 0.0..=50.0).text("Odak Derinliği"));
            ui.add(gizmo::egui::Slider::new(&mut state.post_process.dof_blur_size, 0.0..=5.0).text("Bulanıklık Miktarı"));
        });

    gizmo::egui::Window::new("Araç Dinamikleri (Vehicle Tuning)")
        .default_pos([10.0, 450.0])
        .show(ctx, |ui| {
            let mut vehicles = world.borrow_mut::<gizmo::physics::vehicle::VehicleController>();
            if let Some(vehicle) = vehicles.get_mut(state.car_entity.id()) {
                    ui.heading("Telemetri (Canlı Veri)");
                    ui.label(format!("Hız: {:.1} km/h", vehicle.current_speed_kmh.abs()));
                    ui.label(format!("Motor Devri: {:.0} RPM", vehicle.engine_rpm));
                    let gear_str = if vehicle.reverse_input { "R".to_string() } else if vehicle.current_gear <= 1 { "N".to_string() } else { format!("{}", vehicle.current_gear - 1) };
                    ui.label(format!("Vites: {}", gear_str));
                    
                    ui.separator();
                    ui.heading("Motor & Şanzıman");
                    ui.add(gizmo::egui::Slider::new(&mut vehicle.tuning.max_engine_torque, 1000.0..=100000.0).text("Max Motor Torku"));
                    ui.add(gizmo::egui::Slider::new(&mut vehicle.tuning.max_rpm, 1000.0..=12000.0).text("Max RPM"));
                    ui.checkbox(&mut vehicle.auto_shift, "Otomatik Vites");
                    ui.add(gizmo::egui::Slider::new(&mut vehicle.tuning.upshift_rpm, 3000.0..=12000.0).text("Vites Yükseltme (RPM)"));
                    ui.add(gizmo::egui::Slider::new(&mut vehicle.tuning.downshift_rpm, 1000.0..=8000.0).text("Vites Düşürme (RPM)"));
                    
                    ui.heading("Süspansiyon & Tekerlekler");
                    // Update all wheels
                    let mut stiffness = vehicle.wheels[0].suspension_stiffness;
                    let mut damping = vehicle.wheels[0].suspension_damping;
                    let mut rest_length = vehicle.wheels[0].suspension_rest_length;
                    let current_radius = vehicle.wheels[0].radius;
                    let mut current_diameter_inches = current_radius * 2.0 / 0.0254;
                    
                    if ui.add(gizmo::egui::Slider::new(&mut current_diameter_inches, 14.0..=40.0).text("Tekerlek Çapı (İnç)")).changed() {
                        let new_radius = current_diameter_inches * 0.0254 / 2.0;
                        for w in vehicle.wheels.iter_mut() { w.radius = new_radius; }
                        state.update_wheel_radius = Some(new_radius);
                    }
                    if ui.add(gizmo::egui::Slider::new(&mut stiffness, 10000.0..=100000.0).text("Yay Sertliği (Stiffness)")).changed() {
                        for w in vehicle.wheels.iter_mut() { w.suspension_stiffness = stiffness; }
                    }
                    if ui.add(gizmo::egui::Slider::new(&mut damping, 1000.0..=20000.0).text("Amortisör (Damping)")).changed() {
                        for w in vehicle.wheels.iter_mut() { w.suspension_damping = damping; }
                    }
                    if ui.add(gizmo::egui::Slider::new(&mut rest_length, 0.2..=2.0).text("Yerden Yükseklik (Rest Len)")).changed() {
                        for w in vehicle.wheels.iter_mut() { w.suspension_rest_length = rest_length; }
                    }
                }

            ui.separator();
            ui.heading("Şasi & Ağırlık");
            let mut bodies = world.borrow_mut::<gizmo::physics::RigidBody>();
            if let Some(rb) = bodies.get_mut(state.car_entity.id()) {
                let mut com_y = rb.center_of_mass.y;
                    let mut mass = rb.mass;
                    
                    if ui.add(gizmo::egui::Slider::new(&mut com_y, -2.0..=1.0).text("Ağırlık Merkezi (Y)")).changed() {
                        rb.center_of_mass.y = com_y;
                    }
                    if ui.add(gizmo::egui::Slider::new(&mut mass, 500.0..=5000.0).text("Araç Kütlesi (KG)")).changed() {
                        rb.mass = mass;
                    }
                }
        });
}

fn main() {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::Layer;
    tracing::subscriber::set_global_default(
        tracing_subscriber::registry()
            .with(tracing_subscriber::fmt::layer().with_filter(tracing_subscriber::filter::LevelFilter::INFO))
            .with(tracing_tracy::TracyLayer::default())
    ).expect("Set global default subscriber failed");
    let mut app = App::<DemoState>::new("Gizmo Engine - Hill Climb Racing 2D", 1280, 720)
        .set_setup(setup)
        .set_update(update)
        .set_render(render)
        .set_ui(ui_debug_panel);

    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|arg| arg == "--record") {
        println!("== OYUN KAYDI BASLADI ==");
        app = app.start_recording();
    } else if let Some(idx) = args.iter().position(|arg| arg == "--playback") {
        if idx + 1 < args.len() {
            println!("== OYUN KAYDI OYNATILIYOR: {} ==", args[idx + 1]);
            app = app.start_playback(&args[idx + 1]);
        }
    }

    app.run();
}
