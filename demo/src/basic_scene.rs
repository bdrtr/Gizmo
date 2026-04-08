use gizmo::prelude::*;
use gizmo::physics::{VehicleController, Wheel, RigidBody, Collider};

pub struct BasicSceneState {
    pub player_entity: u32,
    pub camera_entity: u32,
}

pub fn setup_basic_scene(
    world: &mut World,
    renderer: &gizmo::renderer::Renderer,
) -> BasicSceneState {
    world.insert_resource(gizmo::physics::components::PhysicsConfig {
        ground_y: -0.5,
    });
    world.insert_resource(gizmo::physics::JointWorld::new());
    world.insert_resource(gizmo::physics::system::PhysicsSolverState::new());

    let mut asset_manager = gizmo::renderer::asset::AssetManager::new();
    let base_tbind = asset_manager.create_white_texture(&renderer.device, &renderer.queue, &renderer.scene.texture_bind_group_layout);
	   
    // ==================== DÜMDÜZ ZEMİN ====================
    // Duvarda Texture dümdüz dursun diye UV Repeat ile yaratılan create_plane kullanıyoruz.
    // scale edilirse texture aşırı sünüp tek renk gibi görünür.
    let ground_mesh = gizmo::renderer::asset::AssetManager::create_plane(&renderer.device, 400.0);
    
    // YUKARI BAKAN YÜZ
    let ground_top = world.spawn();
    world.add_component(ground_top, Transform::new(Vec3::new(0.0, -0.5, 0.0)));
    world.add_component(ground_top, RigidBody::new_static());
    world.add_component(ground_top, Collider::new_aabb(400.0, 0.5, 400.0));
    
    // AŞAĞI BAKAN YÜZ (Culling hatasına karşı)
    let ground_bottom = world.spawn();
    // 180 derece (PI) döndürüyoruz
    world.add_component(ground_bottom, Transform::new(Vec3::new(0.0, -0.51, 0.0))
        .with_rotation(Quat::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), std::f32::consts::PI)));
    
    let mut ground_mat = Material::new(base_tbind.clone()).with_pbr(Vec4::new(0.4, 0.4, 0.4, 1.0), 0.9, 0.1);
    // Kaplama ekleyelim ki zemin algılansın
    match asset_manager.load_material_texture(&renderer.device, &renderer.queue, &renderer.scene.texture_bind_group_layout, "demo/assets/stone_tiles.jpg") {
        Ok(tex) => {
            ground_mat = Material::new(tex).with_pbr(Vec4::new(0.6, 0.6, 0.6, 1.0), 0.9, 0.1);
            ground_mat.texture_source = Some("demo/assets/stone_tiles.jpg".to_string());
        }
        Err(e) => println!("Zemin kaplamasi yuklenemedi: {:?}", e),
    }
    world.add_component(ground_top, ground_mat.clone());
    world.add_component(ground_top, ground_mesh.clone());
    world.add_component(ground_top, gizmo::renderer::components::MeshRenderer::new());

    world.add_component(ground_bottom, ground_mat);
    world.add_component(ground_bottom, ground_mesh);
    world.add_component(ground_bottom, gizmo::renderer::components::MeshRenderer::new());

    // Gökyüzü bembeyaz olduğu için (White Texture kullandık) göz alıyor. 
    // Skybox'ı siliyoruz, motorun varsayılan karanlık arka plan rengi devreye girecek.

    // ==================== OYUNCU (CAR GLB) ====================
    let player = world.spawn();
    world.add_component(player, Transform::new(Vec3::new(0.0, 2.0, 0.0)));
    
    let mut rb = RigidBody::new(600.0, 0.02, 0.8, true); 
    rb.calculate_box_inertia(2.0, 1.0, 4.0);
    world.add_component(player, rb);
    world.add_component(player, Collider::new_aabb(1.0, 0.5, 2.0));
    world.add_component(player, gizmo::physics::Velocity::new(Vec3::ZERO));

    let mut vc = VehicleController::new();
    vc.lateral_grip = 18000.0;
    vc.steering_force_mult = 15000.0;
    vc.anti_slide_force = 12000.0;
    
    let stiff = 25000.0;
    let damp = 3000.0;
    vc.add_wheel(Wheel::new(Vec3::new(-1.0, -0.3, 1.5), 0.8, stiff, damp, 0.4));
    vc.add_wheel(Wheel::new(Vec3::new(1.0, -0.3, 1.5), 0.8, stiff, damp, 0.4));
    vc.add_wheel(Wheel::new(Vec3::new(-1.0, -0.3, -1.2), 0.8, stiff, damp, 0.4).with_drive());
    vc.add_wheel(Wheel::new(Vec3::new(1.0, -0.3, -1.2), 0.8, stiff, damp, 0.4).with_drive());
    world.add_component(player, vc);

    match asset_manager.load_gltf_scene(
        &renderer.device,
        &renderer.queue,
        &renderer.scene.texture_bind_group_layout,
        base_tbind.clone(),
        "demo/assets/models/ToyCar.glb",
    ) {
        Ok(asset) => {
            let def_mat = gizmo::prelude::Material::new(base_tbind.clone()).with_pbr(Vec4::new(1.0, 1.0, 1.0, 1.0), 0.6, 0.1);
            
            let car_root = world.spawn();
            // Araba çok küçük (ToyCar modeli santimetre bazlı), bu yüzden 100 kat büyütüyoruz
            world.add_component(car_root, Transform::new(Vec3::ZERO).with_scale(Vec3::new(100.0, 100.0, 100.0)));
            world.add_component(car_root, Parent(player.id()));
            
            let children = crate::scene_setup::spawn_gltf_hierarchy(world, &asset.roots, Some(car_root.id()), def_mat);
            world.add_component(car_root, Children(children.iter().map(|e| e.id()).collect()));
            world.add_component(player, Children(vec![car_root.id()]));
        }
        Err(e) => {
            println!("ARABA GLFT YUKLENEMEDI! HATA: {:?}", e);
            world.add_component(player, gizmo::renderer::asset::AssetManager::create_cube(&renderer.device));
            world.add_component(player, gizmo::prelude::Material::new(base_tbind.clone()).with_pbr(Vec4::new(0.8, 0.2, 0.2, 1.0), 0.5, 0.1));
            world.add_component(player, gizmo::renderer::components::MeshRenderer::new());
        }
    }

    // ==================== TAKİP KAMERASI ====================
    let camera_entity = world.spawn();
    world.add_component(camera_entity, Transform::new(Vec3::new(0.0, 5.0, -15.0)));
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
    // is_sun'ı true yapıp gölgeleri geri açıyoruz (Artık shader düzeldi)
    world.add_component(sun, gizmo::renderer::components::DirectionalLight::new(Vec3::new(1.0, 0.95, 0.9), 1.0, true));

    BasicSceneState {
        player_entity: player.id(),
        camera_entity: camera_entity.id(),
    }
}
