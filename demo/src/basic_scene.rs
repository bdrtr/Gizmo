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

    let mut action_map = ActionMap::new();
    action_map.bind_action("Accelerate", KeyCode::ArrowUp as u32);
    action_map.bind_action("Reverse", KeyCode::ArrowDown as u32);
    action_map.bind_action("SteerLeft", KeyCode::ArrowLeft as u32);
    action_map.bind_action("SteerRight", KeyCode::ArrowRight as u32);
    action_map.bind_action("Brake", KeyCode::Space as u32);
    action_map.bind_action("ShootNoCCD", KeyCode::KeyE as u32);
    action_map.bind_action("ShootCCD", KeyCode::KeyR as u32);
    world.insert_resource(action_map);

    world.insert_resource(gizmo::physics::components::PhysicsConfig {
        ground_y: -0.5,
    });
    world.insert_resource(gizmo::physics::JointWorld::new());
    world.insert_resource(gizmo::physics::system::PhysicsSolverState::new());

    let mut asset_manager = gizmo::renderer::asset::AssetManager::new();
    let base_tbind = asset_manager.create_white_texture(&renderer.device, &renderer.queue, &renderer.scene.texture_bind_group_layout);
    
    // ==================== ZEMİN MESHİ (Asfalt/Beton) ====================
    let floor_entity = world.spawn();
    // Şehir artık devasa boyutta olacağı için zemini de şehrin hemen alt tabanına (Y = -2.0) alıyoruz
    world.add_component(floor_entity, Transform::new(Vec3::new(0.0, -2.0, 0.0)));
    world.add_component(floor_entity, gizmo::renderer::asset::AssetManager::create_plane(&renderer.device, 20000.0));
    
    let mut floor_mat = Material::new(base_tbind.clone()).with_pbr(Vec4::new(0.1, 0.1, 0.1, 1.0), 0.95, 0.0);
    floor_mat.is_double_sided = true; // Zemin yüzeyi kameraya ters baksa bile renderlanması için
    world.add_component(floor_entity, floor_mat);
    
    world.add_component(floor_entity, gizmo::renderer::components::MeshRenderer::new());
    world.add_component(floor_entity, EntityName("Asfalt Zemin (City Ground)".to_string()));
	   
    // ==================== DIŞARIDAN YÜKLENEN HARİTA (Map GLTF) ====================
    let map_mat = Material::new(base_tbind.clone()).with_pbr(Vec4::new(0.6, 0.6, 0.6, 1.0), 0.9, 0.1);
    match asset_manager.load_gltf_scene(
        &renderer.device,
        &renderer.queue,
        &renderer.scene.texture_bind_group_layout,
        base_tbind.clone(),
        "demo/assets/city.glb",
    ) {
        Ok(asset) => {
            // Şehir modeli aslında "minyatür" olarak export edilmiş! 
            // Onu gerçek hayattaki gökdelen boyutlarına getirmek için 100 ile çarpıyoruz.
            let root_transform = Transform::new(Vec3::new(0.0, -1.0, 0.0))
                .with_scale(Vec3::new(100.0, 100.0, 100.0));

            crate::scene_setup::spawn_gltf_map(world, &asset, map_mat, root_transform);
            println!("City haritası başarıyla yüklendi ve fiziksel zemin oluşturuldu!");
        }
        Err(e) => {
            println!("HARİTA GLTF YUKLENEMEDI! HATA: {:?}", e);
        }
    }

    // ==================== ANİMASYONLU KARAKTER TESTİ (Cesium Man) ====================
    let char_mat = Material::new(base_tbind.clone()).with_pbr(Vec4::new(1.0, 1.0, 1.0, 1.0), 0.8, 0.2);
    match asset_manager.load_gltf_scene(
        &renderer.device,
        &renderer.queue,
        &renderer.scene.texture_bind_group_layout,
        base_tbind.clone(),
        "demo/assets/cesium_man.glb",
    ) {
        Ok(asset) => {
            let root_transform = Transform::new(Vec3::new(0.0, -0.5, -5.0))
                .with_scale(Vec3::new(2.5, 2.5, 2.5));

            crate::scene_setup::spawn_gltf_asset(world, &asset, renderer, char_mat, root_transform);
            println!("CesiumMan animasyonlu haritası başarıyla yüklendi!");
        }
        Err(e) => {
            println!("CESIUM MAN YUKLENEMEDI! HATA: {:?}", e);
        }
    }

    // ==================== GPU PARTICLE GÖSTERİSİ (Magma Gayzeri) ====================
    let fire_entity = world.spawn();
    world.add_component(fire_entity, Transform::new(Vec3::new(20.0, -10.0, -20.0)));
    
    let mut fire_emitter = gizmo::renderer::components::ParticleEmitter::new();
    fire_emitter.spawn_rate = 2000.0; // Saniyede 2000 partikül
    fire_emitter.initial_velocity = Vec3::new(0.0, 80.0, 0.0); // Fışkırma
    fire_emitter.velocity_randomness = 10.0;
    fire_emitter.lifespan = 4.0;
    fire_emitter.lifespan_randomness = 2.0;
    fire_emitter.size_start = 3.0; // Dev alev küreleri
    fire_emitter.size_end = 0.0;
    fire_emitter.color_start = Vec4::new(1.0, 0.2, 0.0, 1.0); // Dev ateş

    world.add_component(fire_entity, fire_emitter);
    world.add_component(fire_entity, EntityName("Magma Geyser Emitter".to_string()));

    // ==================== TAKİP KAMERASI ====================
    let camera_entity = world.spawn();
    world.add_component(camera_entity, Transform::new(Vec3::new(0.0, 5.0, -15.0)));
    world.add_component(camera_entity, Camera {
        fov: 75.0_f32.to_radians(),
        near: 0.5,
        far: 1500.0,
        yaw: std::f32::consts::FRAC_PI_2,
        pitch: -0.15,
        primary: true,
    });

    // ==================== IŞIK (GÜNEŞ) ====================
    let sun = world.spawn();
    // Daha yatay uzanan, altın saat (Golden Hour) gölgeleri için güneşi eğiyoruz
    world.add_component(sun, Transform::new(Vec3::new(0.0, 100.0, 100.0))
        .with_rotation(Quat::from_axis_angle(Vec3::new(1.0, 0.3, 0.0).normalize(), -std::f32::consts::FRAC_PI_6)));
    world.add_component(sun, gizmo::renderer::components::DirectionalLight {
        color: Vec3::new(1.0, 0.7, 0.4), // Altın / Turuncu (Sunset) güneş rengi
        intensity: 2.5,
        is_sun: true,
    });

    BasicSceneState {
        player_entity: camera_entity.id(), // Shadow focus point
        camera_entity: camera_entity.id(),
    }
}
