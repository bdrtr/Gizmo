use gizmo::prelude::*;
use crate::state::*;

pub fn spawn_gltf_hierarchy(
    world: &mut World,
    nodes: &[gizmo::renderer::GltfNodeData],
    parent_id: Option<u32>,
    default_material: Material,
) -> Vec<gizmo::core::Entity> {
    let mut spawned_entities = Vec::new();

    for node in nodes {
        let entity = world.spawn();
        let id = entity.id();
        spawned_entities.push(entity);

        let entity_name = node.name.clone().unwrap_or_else(|| "GLTF_Node".to_string());
        world.add_component(entity, EntityName(entity_name.clone()));

        // Transform hesapla
        let t = Transform::new(Vec3::new(node.translation[0], node.translation[1], node.translation[2]))
            .with_rotation(Quat::from_xyzw(node.rotation[0], node.rotation[1], node.rotation[2], node.rotation[3]))
            .with_scale(Vec3::new(node.scale[0], node.scale[1], node.scale[2]));
        world.add_component(entity, t);

        if let Some(pid) = parent_id {
            world.add_component(entity, Parent(pid));
        }

        let mut immediate_children = Vec::new();

        for (prim_i, (mesh, mat_opt)) in node.primitives.iter().enumerate() {
            let prim_entity = world.spawn();
            world.add_component(prim_entity, Parent(id));
            world.add_component(prim_entity, Transform::new(Vec3::ZERO));
            world.add_component(prim_entity, EntityName(format!("{}_Primitive_{}", entity_name, prim_i)));
            world.add_component(prim_entity, mesh.clone());
            
            if let Some(mat) = mat_opt {
                world.add_component(prim_entity, mat.clone());
            } else {
                world.add_component(prim_entity, default_material.clone()); // Eğer GLTF material okumadıysa default
            }
            world.add_component(prim_entity, gizmo::renderer::components::MeshRenderer::new());
            immediate_children.push(prim_entity.id());
        }

        // Recursive olarak çocukları in
        if !node.children.is_empty() {
            let child_entities = spawn_gltf_hierarchy(world, &node.children, Some(id), default_material.clone());
            immediate_children.extend(child_entities.iter().map(|e| e.id()));
        }

        if !immediate_children.is_empty() {
            world.add_component(entity, Children(immediate_children));
        }
    }

    spawned_entities
}

pub fn setup_default_scene(world: &mut World, renderer: &gizmo::renderer::renderer::Renderer) -> GameState {
    println!("Gizmo Engine: Sahne başlatılıyor...");

    let mut audio = gizmo::audio::AudioManager::new();
    if let Some(ref mut a) = audio {
        a.load_sound("bounce", "demo/assets/bounce.wav");
    }

    let mut asset_manager = gizmo::renderer::asset::AssetManager::new();

    // Varsayılan Kaplama
    let tbind = asset_manager.load_material_texture(
         &renderer.device,
         &renderer.queue,
         &renderer.texture_bind_group_layout,
         "demo/assets/stone_tiles.jpg"
    ).expect("Varsayilan texture bulunamadi!");

    let dummy_mat = world.spawn();
    world.add_component(dummy_mat, Material::new(tbind.clone()));
    let bouncing_box_id = dummy_mat.id();

    // Sphere Mesh'ini bir kez oluşturup paylaşalım (Instancing için şart!)
    let _sphere_mesh = gizmo::renderer::asset::AssetManager::create_sphere(&renderer.device, 1.0, 16, 16);

    // --- FİZİKLİ ZEMİN OLUŞTURMA ---
    let ground_mesh = gizmo::renderer::asset::AssetManager::create_plane(&renderer.device, 50.0);
    let ground_entity = world.spawn();
    world.add_component(ground_entity, ground_mesh);
    world.add_component(ground_entity, Transform::new(Vec3::new(0.0, -1.0, 0.0)).with_scale(Vec3::new(50.0, 1.0, 50.0)));
    world.add_component(ground_entity, Material::new(tbind.clone()).with_pbr(Vec4::new(0.3, 0.3, 0.3, 1.0), 0.9, 0.1));
    world.add_component(ground_entity, gizmo::renderer::components::MeshRenderer::new());
    let mut ground_rb = gizmo::physics::components::RigidBody::new_static();
    ground_rb.restitution = 1.0; 
    world.add_component(ground_entity, ground_rb);
    world.add_component(ground_entity, gizmo::physics::shape::Collider::new_aabb(25.0, 0.05, 25.0));
    
    // --- GÜNEŞ (Directional Light / Gerçek Zamanlı Ana Gölgelendirici) ---
    let sun = world.spawn();
    let sun_transform = Transform::new(Vec3::new(0.0, 50.0, 50.0))
        .with_rotation(Quat::from_axis_angle(Vec3::new(1.0, 0.5, 0.0).normalize(), -std::f32::consts::FRAC_PI_4));
    world.add_component(sun, sun_transform);
    world.add_component(sun, gizmo::renderer::components::DirectionalLight::new(Vec3::new(1.0, 0.98, 0.9), 1.5, true));
    world.add_component(sun, EntityName("Güneş (Directional)".into()));

    let joint_world = gizmo::physics::JointWorld::new(); // Bağlantı yok ama ECS'ye verilecek
    world.insert_resource(joint_world);
    world.insert_resource(gizmo::physics::system::PhysicsSolverState::new());

    // --- Player (Kamera) ---
    let player = world.spawn();
    world.add_component(player, Transform::new(Vec3::new(0.0, 5.0, 15.0)));
    world.add_component(player, Camera::new(
        std::f32::consts::FRAC_PI_4, 0.1, 2000.0,
        -std::f32::consts::FRAC_PI_2, -0.3, true,
    ));
    world.add_component(player, EntityName("Kamera (Göz)".into()));

    // --- Skybox (Sonsuz Gökyüzü) ---
    let skybox = world.spawn();
    let mut sky_transform = Transform::new(Vec3::ZERO);
    sky_transform.scale = Vec3::new(500.0, 500.0, 500.0); 
    world.add_component(skybox, sky_transform);
    world.add_component(skybox, gizmo::renderer::asset::AssetManager::create_inverted_cube(&renderer.device));
    world.add_component(skybox, Material::new(tbind.clone()).with_skybox());
    world.add_component(skybox, gizmo::renderer::components::MeshRenderer::new());
    world.add_component(skybox, EntityName("Skybox (Gök Kubbe)".into()));

    // --- GIZMO EKSENLERI (X, Y, Z) ---
    let x_gizmo = world.spawn();
    world.add_component(x_gizmo, Transform::new(Vec3::new(0.0, -1000.0, 0.0)).with_scale(Vec3::new(1.5, 0.08, 0.08)));
    world.add_component(x_gizmo, gizmo::renderer::asset::AssetManager::create_sphere(&renderer.device, 1.0, 6, 6));
    world.add_component(x_gizmo, Material::new(tbind.clone()).with_unlit(Vec4::new(1.0, 0.0, 0.0, 1.0)));
    world.add_component(x_gizmo, Collider::new_aabb(1.5, 0.3, 0.3));
    world.add_component(x_gizmo, gizmo::renderer::components::MeshRenderer::new());
    world.add_component(x_gizmo, EntityName("Gizmo_X".into()));

    let y_gizmo = world.spawn();
    world.add_component(y_gizmo, Transform::new(Vec3::new(0.0, -1000.0, 0.0)).with_scale(Vec3::new(0.08, 1.5, 0.08)));
    world.add_component(y_gizmo, gizmo::renderer::asset::AssetManager::create_sphere(&renderer.device, 1.0, 6, 6));
    world.add_component(y_gizmo, Material::new(tbind.clone()).with_unlit(Vec4::new(0.0, 1.0, 0.0, 1.0)));
    world.add_component(y_gizmo, Collider::new_aabb(0.3, 1.5, 0.3));
    world.add_component(y_gizmo, gizmo::renderer::components::MeshRenderer::new());
    world.add_component(y_gizmo, EntityName("Gizmo_Y".into()));

    let z_gizmo = world.spawn();
    world.add_component(z_gizmo, Transform::new(Vec3::new(0.0, -1000.0, 0.0)).with_scale(Vec3::new(0.08, 0.08, 1.5)));
    world.add_component(z_gizmo, gizmo::renderer::asset::AssetManager::create_sphere(&renderer.device, 1.0, 6, 6));
    world.add_component(z_gizmo, Material::new(tbind.clone()).with_unlit(Vec4::new(0.0, 0.0, 1.0, 1.0)));
    world.add_component(z_gizmo, Collider::new_aabb(0.3, 0.3, 1.5));
    world.add_component(z_gizmo, gizmo::renderer::components::MeshRenderer::new());
    world.add_component(z_gizmo, EntityName("Gizmo_Z".into()));

    let player_id = player.id();
    let skybox_id = skybox.id();
    let gizmo_x = x_gizmo.id();
    let gizmo_y = y_gizmo.id();
    let gizmo_z = z_gizmo.id();

    // Yüksek kaliteli top (Sphere) Mesh'i yarat ve sakla
    let sphere_prefab = world.spawn();
    world.add_component(sphere_prefab, gizmo::renderer::asset::AssetManager::create_sphere(&renderer.device, 1.0, 8, 8)); // 8x8
    let cube_prefab = world.spawn();
    world.add_component(cube_prefab, gizmo::renderer::asset::AssetManager::create_cube(&renderer.device));

    // --- YAĞMUR PARTİKÜLLERİ ---
    println!("Gizmo Engine: Yağmur partikülleri oluşturuluyor...");
    let rain_mat = world.spawn();
    world.add_component(rain_mat, Material::new(tbind.clone()).with_pbr(Vec4::new(0.6, 0.7, 0.9, 0.6), 0.1, 0.0).with_unlit(Vec4::new(0.6, 0.7, 0.9, 0.6)));
    let rain_mesh = gizmo::renderer::asset::AssetManager::create_cube(&renderer.device);

    // assuming rand is available, or use a simple LCG
    let mut rng_seed: u32 = 12345;
    let mut rand_f32 = || -> f32 {
        rng_seed = rng_seed.wrapping_mul(1664525).wrapping_add(1013904223);
        (rng_seed as f32 / std::u32::MAX as f32)
    };

    for i in 0..500 {
        let drop = world.spawn();
        let rx = (rand_f32() - 0.5) * 60.0;
        let rz = (rand_f32() - 0.5) * 60.0;
        let ry = 10.0 + (rand_f32() * 30.0);
        
        // Yağmur damlası şekli (ince uzun küp)
        world.add_component(drop, Transform::new(Vec3::new(rx, ry, rz))
            .with_scale(Vec3::new(0.04, 0.8, 0.04)));
        
        world.add_component(drop, EntityName(format!("Raindrop_{}", i)));
        world.add_component(drop, gizmo::physics::components::Velocity::new(Vec3::new(0.0, -15.0 - (rand_f32() * 10.0), 0.0)));
        world.add_component(drop, rain_mesh.clone());
        world.add_component(drop, Material::new(tbind.clone()).with_unlit(Vec4::new(0.4, 0.6, 0.9, 0.8))); // Biraz mavi şeffafımsı görünüm
        world.add_component(drop, gizmo::renderer::components::MeshRenderer::new());
        // LUA SCRIPT EKLENTİSİ
        world.add_component(drop, gizmo::scripting::Script {
            file_path: "demo/assets/scripts/rain.lua".to_string(),
            initialized: false,
        });
        // RigidBody ve Collider EKLENMİYOR! Böylece sadece script hareket ettirir.
    }

    GameState {
        bouncing_box_id,
        player_id,
        skybox_id,
        inspector_selected_entity: None,
        audio,
        do_raycast: false,
        gizmo_x,
        gizmo_y,
        gizmo_z,
        dragging_axis: None,
        drag_start_t: 0.0,
        drag_original_pos: Vec3::ZERO,
        drag_original_scale: Vec3::ONE,
        drag_original_rot: Quat::IDENTITY,
        current_fps: 60.0,
        new_selection_request: std::cell::Cell::new(None),
        spawn_domino_requests: std::cell::Cell::new(1),
        release_domino_requests: std::cell::Cell::new(0),
        domino_ball_id: std::cell::Cell::new(None),
        texture_load_requests: std::cell::RefCell::new(Vec::new()),
        asset_spawn_requests: std::cell::RefCell::new(Vec::new()),
        asset_manager: std::cell::RefCell::new(asset_manager),
        gizmo_mode: GizmoMode::Translate,
        egui_wants_pointer: false,
        asset_watcher: gizmo::renderer::hot_reload::AssetWatcher::new(&["demo/assets"]),
        script_engine: std::cell::RefCell::new(gizmo::scripting::ScriptEngine::new().ok()),
        physics_accumulator: 0.0,
        target_physics_fps: 240.0, // Sub-stepping: saniyede 240 simülasyon adımı (60 FPS'te kare başı 4 adım)
        sphere_prefab_id: sphere_prefab.id(),
        cube_prefab_id: cube_prefab.id(),
        post_process_settings: std::cell::RefCell::new(gizmo::renderer::renderer::PostProcessUniforms {
            bloom_intensity: 0.3,
            bloom_threshold: 1.0,
            exposure: 1.0,
            chromatic_aberration: 0.0,
            vignette_intensity: 0.0,
            _padding: [0.0; 3],
        }),
        shader_reload_request: std::cell::Cell::new(false),
        editor_state: std::cell::RefCell::new(gizmo::editor::EditorState::new()),
        free_cam: true,
    }
}
