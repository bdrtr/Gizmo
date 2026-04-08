use gizmo::prelude::*;
use gizmo::physics::{VehicleController, Wheel, RigidBody, Collider};

pub struct SandboxState {
    pub player_entity: u32,
    pub camera_entity: u32,
}

pub fn setup_sandbox_scene(
    world: &mut World,
    renderer: &gizmo::renderer::Renderer,
) -> SandboxState {
    // Fizik Konfigürasyonu
    world.insert_resource(gizmo::physics::components::PhysicsConfig {
        ground_y: -0.5,
    });
    world.insert_resource(gizmo::physics::JointWorld::new());
    world.insert_resource(gizmo::physics::system::PhysicsSolverState::new());

    let mut asset_manager = gizmo::renderer::asset::AssetManager::new();
    let base_tbind = asset_manager.create_white_texture(&renderer.device, &renderer.queue, &renderer.scene.texture_bind_group_layout);

    // ==================== ZEMİN ====================
    let ground = world.spawn();
    world.add_component(ground, Transform::new(Vec3::new(0.0, -0.5, 0.0)).with_scale(Vec3::new(400.0, 1.0, 400.0)));
    world.add_component(ground, RigidBody::new_static());
    world.add_component(ground, Collider::new_aabb(400.0, 0.5, 400.0));
    world.add_component(ground, gizmo::prelude::Material::new(base_tbind.clone()).with_pbr(Vec4::new(0.2, 0.3, 0.25, 1.0), 0.9, 0.1));
    world.add_component(ground, gizmo::renderer::asset::AssetManager::create_cube(&renderer.device));
    world.add_component(ground, gizmo::renderer::components::MeshRenderer::new());

    // ==================== RAMPALAR ====================
    let ramp_configs = vec![
        (Vec3::new(20.0, 1.5, 40.0), 10.0_f32.to_radians(), [0.8, 0.2, 0.2, 1.0]), // Hafif
        (Vec3::new(0.0, 3.5, 60.0), 20.0_f32.to_radians(), [0.8, 0.5, 0.2, 1.0]),  // Orta
        (Vec3::new(-20.0, 7.0, 80.0), 30.0_f32.to_radians(), [0.8, 0.8, 0.2, 1.0]), // Dik
    ];

    for (pos, angle, color) in ramp_configs {
        let ramp = world.spawn();
        let rot = Quat::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), -angle); // Yukarı bakan eğim
        world.add_component(ramp, Transform::new(pos).with_rotation(rot).with_scale(Vec3::new(10.0, 0.5, 20.0)));
        world.add_component(ramp, RigidBody::new_static());
        // AABB collider'ı döndürülemediği için yaklaşık bbox kullanacağız veya ileride OBB eklendiğinde değiştirebiliriz.
        // Şimdilik AABB büyük bir kutu olacak. Fizikte rampalar için OBB veya ConvexHull tam çözüm olur.
        world.add_component(ramp, Collider::new_aabb(10.0, 4.0, 20.0));
        world.add_component(ramp, gizmo::prelude::Material::new(base_tbind.clone()).with_pbr(Vec4::new(color[0], color[1], color[2], color[3]), 0.8, 0.3));
        world.add_component(ramp, gizmo::renderer::asset::AssetManager::create_cube(&renderer.device));
        world.add_component(ramp, gizmo::renderer::components::MeshRenderer::new());
    }

    // ==================== DOMİNO ÇARPIŞMA TEST ALANI ====================
    let start_x = 30.0;
    let start_z = 20.0;
    for i in 0..6 {
        for j in 0..4 {
            let box_ent = world.spawn();
            let p_x = start_x + (i as f32 * 2.5);
            let p_y = 1.0 + (j as f32 * 2.2);
            world.add_component(box_ent, Transform::new(Vec3::new(p_x, p_y, start_z)).with_scale(Vec3::new(1.0, 1.0, 1.0)));
            world.add_component(box_ent, RigidBody::new(10.0, 0.5, 0.5, true));
            world.add_component(box_ent, Collider::new_aabb(1.0, 1.0, 1.0));
            world.add_component(box_ent, gizmo::prelude::Material::new(base_tbind.clone()).with_pbr(Vec4::new(0.3, 0.5, 0.9, 1.0), 0.5, 0.1));
            world.add_component(box_ent, gizmo::renderer::asset::AssetManager::create_cube(&renderer.device));
            world.add_component(box_ent, gizmo::renderer::components::MeshRenderer::new());
        }
    }

    // ==================== OYUNCU (CAR GLB) ====================
    let player = world.spawn();
    world.add_component(player, Transform::new(Vec3::new(0.0, 2.0, -10.0)));
    
    let mut rb = RigidBody::new(600.0, 0.02, 0.8, true); 
    rb.calculate_box_inertia(2.0, 1.0, 4.0);
    world.add_component(player, rb);
    world.add_component(player, Collider::new_aabb(1.0, 0.5, 2.0));
    world.add_component(player, gizmo::physics::Velocity::new(Vec3::ZERO));

    // VehicleController — 4 tekerlek (RWD / 4WD Hibrit Test)
    let mut vc = VehicleController::new();
    vc.lateral_grip = 18000.0;
    vc.steering_force_mult = 15000.0;
    vc.anti_slide_force = 12000.0;
    
    // Süspansiyonları esnetelim (Rampa testleri için)
    let stiff = 25000.0;
    let damp = 3000.0;
    vc.add_wheel(Wheel::new(Vec3::new(-1.0, -0.3, 1.5), 0.8, stiff, damp, 0.4));
    vc.add_wheel(Wheel::new(Vec3::new(1.0, -0.3, 1.5), 0.8, stiff, damp, 0.4));
    // Arka tekerler motor gücü alır
    vc.add_wheel(Wheel::new(Vec3::new(-1.0, -0.3, -1.2), 0.8, stiff, damp, 0.4).with_drive());
    vc.add_wheel(Wheel::new(Vec3::new(1.0, -0.3, -1.2), 0.8, stiff, damp, 0.4).with_drive());
    world.add_component(player, vc);

    // Car GLB yükle
    if let Ok(asset) = asset_manager.load_gltf_scene(
        &renderer.device,
        &renderer.queue,
        &renderer.scene.texture_bind_group_layout,
        base_tbind.clone(),
        "demo/assets/car.glb",
    ) {
        let def_mat = gizmo::prelude::Material::new(base_tbind.clone()).with_pbr(Vec4::new(1.0, 1.0, 1.0, 1.0), 0.6, 0.1);
        crate::scene_setup::spawn_gltf_hierarchy(world, &asset.roots, Some(player.id()), def_mat);
        println!("[Sandbox] car.glb başarıyla yüklendi!");
    } else {
        println!("[Sandbox] car.glb bulunamadı! Lütfen 'demo/assets/car.glb' konumuna dosyayı koyun.");
        // Fallback küp
        world.add_component(player, gizmo::renderer::asset::AssetManager::create_cube(&renderer.device));
        world.add_component(player, gizmo::prelude::Material::new(base_tbind.clone()).with_pbr(Vec4::new(0.8, 0.2, 0.2, 1.0), 0.5, 0.1));
        world.add_component(player, gizmo::renderer::components::MeshRenderer::new());
    }

    // ==================== TAKİP KAMERASI ====================
    let camera_entity = world.spawn();
    world.add_component(camera_entity, Transform::new(Vec3::new(0.0, 5.0, -20.0)));
    world.add_component(camera_entity, Camera {
        fov: 75.0_f32.to_radians(),
        near: 0.1,
        far: 1500.0,
        yaw: 0.0,
        pitch: -0.15,
        primary: true,
    });

    // ==================== IŞIK (GÜNEŞ) ====================
    let sun = world.spawn();
    world.add_component(sun, Transform::new(Vec3::new(0.0, 100.0, 100.0))
        .with_rotation(Quat::from_axis_angle(Vec3::new(1.0, 0.5, 0.0).normalize(), -std::f32::consts::FRAC_PI_4)));
    world.add_component(sun, gizmo::renderer::components::DirectionalLight::new(Vec3::new(1.0, 0.95, 0.9), 3.0, true));

    SandboxState {
        player_entity: player.id(),
        camera_entity: camera_entity.id(),
    }
}
