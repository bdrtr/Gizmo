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

pub fn spawn_gltf_map_hierarchy(
    world: &mut World,
    nodes: &[gizmo::renderer::GltfNodeData],
    parent_pos: Vec3,
    parent_rot: Quat,
    parent_scale: Vec3,
    default_material: Material,
) -> Vec<gizmo::core::Entity> {
    let mut spawned_entities = Vec::new();

    for node in nodes {
        let local_pos = Vec3::new(node.translation[0], node.translation[1], node.translation[2]);
        let local_rot = Quat::from_xyzw(node.rotation[0], node.rotation[1], node.rotation[2], node.rotation[3]);
        let local_scale = Vec3::new(node.scale[0], node.scale[1], node.scale[2]);

        let global_scale = parent_scale * local_scale;
        let global_rot = parent_rot * local_rot;
        let scaled_pos = Vec3::new(local_pos.x * parent_scale.x, local_pos.y * parent_scale.y, local_pos.z * parent_scale.z);
        let global_pos = parent_pos + parent_rot * scaled_pos;

        let entity_name = node.name.clone().unwrap_or_else(|| "GLTF_MapNode".to_string());

        for (prim_i, (mesh, mat_opt)) in node.primitives.iter().enumerate() {
            let prim_entity = world.spawn();
            spawned_entities.push(prim_entity);
            
            world.add_component(prim_entity, Transform::new(global_pos).with_rotation(global_rot).with_scale(global_scale));
            world.add_component(prim_entity, EntityName(format!("{}_Primitive_{}", entity_name, prim_i)));
            world.add_component(prim_entity, mesh.clone());
            
            if let Some(mat) = mat_opt {
                world.add_component(prim_entity, mat.clone());
            } else {
                world.add_component(prim_entity, default_material.clone()); 
            }
            world.add_component(prim_entity, gizmo::renderer::components::MeshRenderer::new());
            
            world.add_component(prim_entity, RigidBody::new_static());
            let bounds_extents = (mesh.bounds.max - mesh.bounds.min) * 0.5;
            let safe_hx = (bounds_extents.x * global_scale.x).max(0.05);
            let safe_hy = (bounds_extents.y * global_scale.y).max(0.05);
            let safe_hz = (bounds_extents.z * global_scale.z).max(0.05);
            world.add_component(prim_entity, Collider::new_aabb(safe_hx, safe_hy, safe_hz));
        }

        if !node.children.is_empty() {
            let child_entities = spawn_gltf_map_hierarchy(world, &node.children, global_pos, global_rot, global_scale, default_material.clone());
            spawned_entities.extend(child_entities);
        }
    }

    spawned_entities
}

pub fn spawn_gltf_map(
    world: &mut World,
    asset: &gizmo::renderer::asset::GltfSceneAsset,
    default_material: Material,
    root_transform: Transform,
) -> Vec<gizmo::core::Entity> {
    spawn_gltf_map_hierarchy(
        world, 
        &asset.roots, 
        root_transform.position, 
        root_transform.rotation, 
        root_transform.scale, 
        default_material
    )
}


pub fn setup_headless_scene(world: &mut World) -> crate::state::GameState {
    world.insert_resource(gizmo::core::input::ActionMap::new());
    world.insert_resource(gizmo::physics::components::PhysicsConfig {
        ground_y: -0.5,
    });
    world.insert_resource(gizmo::physics::JointWorld::new());
    world.insert_resource(gizmo::physics::system::PhysicsSolverState::new());
    
    // Physics and default entities setup without rendering components
    let player = world.spawn();
    world.add_component(player, Transform::new(Vec3::new(0.0, 5.0, 0.0)));
    world.add_component(player, gizmo::physics::character::CharacterController::new(0.4, 0.8));
    world.add_component(player, crate::Player);

    crate::state::GameState {
        player_id: player.id(),
        bouncing_box_id: 0,
        skybox_id: 0,
        inspector_selected_entity: None,
        audio: None,
        do_raycast: false,
        gizmo_x: 0,
        gizmo_y: 0,
        gizmo_z: 0,
        dragging_axis: None,
        drag_start_t: 0.0,
        drag_original_pos: Vec3::ZERO,
        drag_original_scale: Vec3::ONE,
        drag_original_rot: Quat::IDENTITY,
        current_fps: 60.0,
        gizmo_mode: crate::state::GizmoMode::Translate,
        egui_wants_pointer: false,
        asset_watcher: gizmo::renderer::hot_reload::AssetWatcher::new(&["demo/assets"]),
        physics_accumulator: 0.0,
        target_physics_fps: 240.0, 
        sphere_prefab_id: 0,
        cube_prefab_id: 0,
        free_cam: false,
        active_dialogue: None,
        active_cutscene: None,
        checkpoints: Vec::new(),
        race_status: crate::state::RaceStatus::Idle,
        race_timer: 0.0,
        camera_follow_target: None,
        total_elapsed: 0.0,
        ps1_race: None,
        basic_scene: None,
        show_devtools: false,
    }
}

pub fn spawn_gltf_asset(
    world: &mut World,
    asset: &gizmo::renderer::asset::GltfSceneAsset,
    renderer: &gizmo::renderer::renderer::Renderer,
    default_material: Material,
    root_transform: Transform,
) -> gizmo::core::Entity {
    // Tüm sahneyi sarmalayan Ana (Root) Entity
    let root_ent = world.spawn();
    world.add_component(root_ent, root_transform);
    world.add_component(root_ent, EntityName("GLTF_Asset_Root".into()));

    // Hiyerarşiyi Root altında oluştur
    let children = spawn_gltf_hierarchy(world, &asset.roots, Some(root_ent.id()), default_material);
    let child_ids: Vec<u32> = children.iter().map(|e| e.id()).collect();
    if !child_ids.is_empty() {
        world.add_component(root_ent, Children(child_ids));
    }

    // İskelet Animasyon (Skeletal Animation) Bileşeni
    if !asset.skeletons.is_empty() {
        // İlk SkeletonHierarchy'i alıyoruz (genelde tek kök iskelet vardır)
        let hierarchy = std::sync::Arc::new(asset.skeletons[0].clone());
        
        let buf = renderer.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Skeletal Animation Buffer"),
            size: 64 * 64, // 64 mat4x4 (64 * 16 * 4 bayt = 4096)
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        
        let bg = renderer.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Skeletal Animation BindGroup"),
            layout: &renderer.scene.skeleton_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buf.as_entire_binding(),
            }],
        });
        
        world.add_component(root_ent, gizmo::renderer::components::Skeleton {
            bind_group: std::sync::Arc::new(bg),
            buffer: std::sync::Arc::new(buf),
            hierarchy,
            local_poses: Vec::new(),
        });
    }

    // Animasyon (Keyframe) Bileşeni
    if !asset.animations.is_empty() {
        world.add_component(root_ent, gizmo::renderer::components::AnimationPlayer {
            current_time: 0.0,
            active_animation: 0, // İlk animasyonu varsayılan başlat
            loop_anim: true,
            animations: std::sync::Arc::new(asset.animations.clone()),
        });
    }

    root_ent
}

pub fn setup_empty_scene(_world: &mut World, _renderer: &gizmo::renderer::renderer::Renderer) -> GameState {
    let audio = gizmo::audio::AudioManager::new();
    let asset_watcher = gizmo::renderer::hot_reload::AssetWatcher::new(&["demo/assets"]);

    GameState {
        bouncing_box_id: 0,
        player_id: 0,
        skybox_id: 0,
        inspector_selected_entity: None,
        audio,
        do_raycast: false,
        gizmo_x: 0,
        gizmo_y: 0,
        gizmo_z: 0,
        dragging_axis: None,
        drag_start_t: 0.0,
        drag_original_pos: Vec3::ZERO,
        drag_original_scale: Vec3::ONE,
        drag_original_rot: Quat::IDENTITY,
        current_fps: 60.0,
        gizmo_mode: GizmoMode::Translate,
        egui_wants_pointer: false,
        asset_watcher,
        physics_accumulator: 0.0,
        target_physics_fps: 240.0,
        sphere_prefab_id: 0,
        cube_prefab_id: 0,
        free_cam: true,
        active_dialogue: None,
        active_cutscene: None,
        checkpoints: Vec::new(),
        race_status: crate::state::RaceStatus::Idle,
        race_timer: 0.0,
        camera_follow_target: None,
        total_elapsed: 0.0,
        ps1_race: None,
        basic_scene: None,
        show_devtools: false,
    }
}

pub fn setup_default_scene(world: &mut World, renderer: &gizmo::renderer::renderer::Renderer) -> GameState {
    println!("Gizmo Engine: Sahne başlatılıyor...");
    world.insert_resource(gizmo::core::event::Events::<gizmo::physics::CollisionEvent>::new());

    let mut audio = gizmo::audio::AudioManager::new();
    if let Some(ref mut a) = audio {
        a.load_sound("bounce", "demo/assets/bounce.wav");
    }

    let mut asset_manager = gizmo::renderer::asset::AssetManager::new();

    // Varsayılan Kaplama
    let tbind = asset_manager.load_material_texture(
         &renderer.device,
         &renderer.queue,
         &renderer.scene.texture_bind_group_layout,
         "demo/assets/stone_tiles.jpg"
    ).expect("Varsayilan texture bulunamadi!");

    let dummy_mat = world.spawn();
    world.add_component(dummy_mat, Material::new(tbind.clone()));
    let bouncing_box_id = dummy_mat.id();

    // Sphere Mesh'ini bir kez oluşturup paylaşalım (Instancing için şart!)
    let _sphere_mesh = gizmo::renderer::asset::AssetManager::create_sphere(&renderer.device, 1.0, 16, 16);

    // --- FİZİKLİ ZEMİN OLUŞTURMA ---
    let ground_mesh = gizmo::renderer::asset::AssetManager::create_plane(&renderer.device, 200.0);
    let ground_entity = world.spawn();
    world.add_component(ground_entity, ground_mesh);
    world.add_component(ground_entity, Transform::new(Vec3::new(0.0, -1.0, 0.0)).with_scale(Vec3::new(200.0, 1.0, 200.0)));
    
    let mut ground_mat = Material::new(tbind.clone()).with_pbr(Vec4::new(0.8, 0.7, 0.4, 1.0), 0.9, 0.1);
    ground_mat.texture_source = Some("demo/assets/stone_tiles.jpg".to_string());
    world.add_component(ground_entity, ground_mat);
    world.add_component(ground_entity, gizmo::renderer::components::MeshRenderer::new());
    let mut ground_rb = gizmo::physics::components::RigidBody::new_static();
    ground_rb.restitution = 1.0; 
    world.add_component(ground_entity, ground_rb);
    world.add_component(ground_entity, gizmo::physics::shape::Collider::new_aabb(100.0, 0.05, 100.0));
    
    // --- GÜNEŞ (Karanlık Ambiyans İçin Işık Şiddeti Çok Düşüldü) ---
    let sun = world.spawn();
    let sun_transform = Transform::new(Vec3::new(0.0, 50.0, 50.0))
        .with_rotation(Quat::from_axis_angle(Vec3::new(1.0, 0.5, 0.0).normalize(), -std::f32::consts::FRAC_PI_4));
    world.add_component(sun, sun_transform);
    world.add_component(sun, gizmo::renderer::components::DirectionalLight::new(Vec3::new(1.0, 0.95, 0.85), 2.0, true)); // Güçlü Güneş Işığı
    world.add_component(sun, EntityName("Ay (Directional)".into()));

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

    // --- Skybox (Gece Gökyüzü) ---
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
    let base_sphere_mesh = gizmo::renderer::asset::AssetManager::create_sphere(&renderer.device, 1.0, 8, 8); // 8x8
    world.add_component(sphere_prefab, base_sphere_mesh.clone()); 
    world.add_component(sphere_prefab, Material::new(tbind.clone()));
    let cube_prefab = world.spawn();
    world.add_component(cube_prefab, gizmo::renderer::asset::AssetManager::create_cube(&renderer.device));
    world.add_component(cube_prefab, Material::new(tbind.clone()));

    // --- SKELETAL ANIMATION TEST (CesiumMan) ---
    println!("Gizmo Engine: CesiumMan.glb test karakteri yükleniyor...");
    if let Ok(cesium_man_asset) = asset_manager.load_gltf_scene(
        &renderer.device,
        &renderer.queue,
        &renderer.scene.texture_bind_group_layout,
        tbind.clone(),
        "demo/assets/cesium_man.glb",
    ) {
        let man_root_transform = Transform::new(Vec3::new(0.0, 1.0, -10.0))
            .with_rotation(Quat::from_axis_angle(Vec3::new(0.0, 1.0, 0.0), std::f32::consts::PI));
        
        let mut def_mat = Material::new(tbind.clone());
        def_mat.albedo = Vec4::new(1.0, 1.0, 1.0, 1.0);

        let entity = spawn_gltf_asset(world, &cesium_man_asset, renderer, def_mat, man_root_transform);
        if let Some(mut ent_name) = world.borrow_mut::<EntityName>() {
            if let Some(n) = ent_name.get_mut(entity.id()) {
                n.0 = "CesiumMan (Animated)".to_string();
            }
        }
    } else {
        println!("Gizmo Engine: demo/assets/cesium_man.glb bulunamadı! İskelet animasyon testi atlanıyor.");
    }

    // --- PACHINKO / GALTON KUTUSU (GÖRSEL ŞOV) - TEST İÇİN DEVRE DIŞI BIRAKILDI ---
    /*
    println!("Gizmo Engine: Galton Kutusu (Pachinko) inşaa ediliyor...");
    ...
    */

    // --- NAVGRID & AI TESTİ ---
    let mut nav_grid = gizmo_ai::NavGrid::new(1.0); // Hücre boyutunu 1.0 yaptık

    println!("Gizmo Engine: AI Engelleri ve NavGrid yükleniyor...");
    for _ in 0..20 {
        let x = (rand::random::<f32>() - 0.5) * 40.0;
        let z = (rand::random::<f32>() - 0.5) * 40.0;
        if x.abs() < 5.0 && z.abs() < 5.0 { continue; } // Player başlangıcından uzağa koy
        
        let pos = Vec3::new(x, 1.0, z);
        let wall = world.spawn();
        
        world.add_component(wall, Transform::new(pos).with_scale(Vec3::new(1.0, 2.0, 1.0)));
        world.add_component(wall, gizmo::renderer::asset::AssetManager::create_cube(&renderer.device));
        world.add_component(wall, Material::new(tbind.clone()));
        world.add_component(wall, Collider::new_aabb(1.0, 2.0, 1.0));
        world.add_component(wall, gizmo::physics::components::RigidBody::new_static());
        world.add_component(wall, gizmo::renderer::components::MeshRenderer::new());
        world.add_component(wall, EntityName("Duvar Engel".to_string()));

        // NavGrid engel padding: collider boyutuna göre hesapla (half_extent / cell_size + 1)
        let cell_size = 1.0_f32; // NavGrid cell_size ile eşleşmeli
        let pad_x = (1.0_f32 / cell_size).ceil() as i32; // half_extent.x = 1.0
        let pad_z = (1.0_f32 / cell_size).ceil() as i32; // half_extent.z = 1.0
        for ix in -pad_x..=pad_x {
            for iz in -pad_z..=pad_z {
                nav_grid.add_obstacle_world(pos + Vec3::new(ix as f32 * cell_size, 0.0, iz as f32 * cell_size));
            }
        }
    }
    
    // NPCs
    println!("Gizmo Engine: Kırmızı AI NPC test için oluşturuluyor!");
    let npc = world.spawn();
    world.add_component(npc, Transform::new(Vec3::new(20.0, 1.5, 20.0)).with_scale(Vec3::new(1.0, 2.0, 1.0)));
    world.add_component(npc, gizmo::renderer::asset::AssetManager::create_cube(&renderer.device));
    world.add_component(npc, Material::new(tbind.clone()).with_pbr(Vec4::new(1.0, 0.0, 0.0, 1.0), 0.5, 0.5)); // Kırmızı
    world.add_component(npc, gizmo::renderer::components::MeshRenderer::new());
    world.add_component(npc, Collider::new_aabb(1.0, 2.0, 1.0)); // Küp visual ile eşleşiyor
    let mut rb = gizmo::physics::components::RigidBody::new(1.0, 0.0, 0.0, false);
    rb.ccd_enabled = false; // NPC Yavaş olduğu için CCD ye gerek yok
    rb.calculate_box_inertia(2.0, 4.0, 2.0); // Collider boyutları: 2*half_extents
    world.add_component(npc, rb);
    world.add_component(npc, gizmo::physics::components::Velocity::new(Vec3::ZERO));
    world.add_component(npc, gizmo_ai::NavAgent::new(5.0, 50.0, 2.0)); // Hız 5.0 (solver uyumlu), Güç 50.0
    world.add_component(npc, EntityName("NPC Takipçi".to_string()));

    world.insert_resource(nav_grid);


    world.insert_resource(gizmo::core::event::Events::<crate::state::SpawnDominoEvent>::new());
    world.insert_resource(gizmo::core::event::Events::<crate::state::ReleaseDominoEvent>::new());
    world.insert_resource(gizmo::core::event::Events::<crate::state::TextureLoadEvent>::new());
    world.insert_resource(gizmo::core::event::Events::<crate::state::AssetSpawnEvent>::new());
    world.insert_resource(gizmo::core::event::Events::<crate::state::ShaderReloadEvent>::new());
    world.insert_resource(gizmo::core::event::Events::<crate::state::SelectionEvent>::new());
    world.insert_resource(crate::state::DominoAppState { active_ball_id: None });
    world.insert_resource(crate::state::PachinkoSpawnerState { timer: 0.0, count: 0 });
    world.insert_resource(asset_manager);
    
    if let Ok(engine) = gizmo::scripting::ScriptEngine::new() {
        world.insert_resource(engine);
    }
    
    world.insert_resource(gizmo::renderer::renderer::PostProcessUniforms {
        bloom_intensity: 0.3,
        bloom_threshold: 1.0,
        exposure: 1.0,
        chromatic_aberration: 0.0,
        vignette_intensity: 0.0,
        _padding: [0.0; 3],
    });
    
    world.insert_resource(gizmo::editor::EditorState::new());
    
    // UI Durumları
    world.insert_resource(crate::state::AppMode::InGame);
    world.insert_resource(crate::state::PlayerStats {
        health: 100.0,
        max_health: 100.0,
        ammo: 30,
        max_ammo: 120,
    });

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
        gizmo_mode: GizmoMode::Translate,
        egui_wants_pointer: false,
        asset_watcher: gizmo::renderer::hot_reload::AssetWatcher::new(&["demo/assets"]),
        physics_accumulator: 0.0,
        target_physics_fps: 240.0, // Sub-stepping: saniyede 240 simülasyon adımı (60 FPS'te kare başı 4 adım)
        sphere_prefab_id: sphere_prefab.id(),
        cube_prefab_id: cube_prefab.id(),
        free_cam: false,

        // Oyun sistemi
        active_dialogue: None,
        active_cutscene: None,
        checkpoints: Vec::new(),
        race_status: crate::state::RaceStatus::Idle,
        race_timer: 0.0,
        camera_follow_target: None,
        total_elapsed: 0.0,
        ps1_race: None,
        basic_scene: None,
        show_devtools: false,
    }
}
