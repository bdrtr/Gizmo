use crate::state::{DebugAssets, StudioState};
use gizmo::editor::EditorState;
use gizmo::math::{Quat, Vec3, Vec4};
use gizmo::physics::components::Transform;
use gizmo::prelude::*;

pub fn setup_studio_scene(world: &mut World, renderer: &gizmo::renderer::Renderer) -> StudioState {
    // --- Setup Editor Scene (Grid & Axes) ---
    let mut asset_manager = gizmo::renderer::asset::AssetManager::new();
    let white_tex = asset_manager.create_white_texture(
        &renderer.device,
        &renderer.queue,
        &renderer.scene.texture_bind_group_layout,
    );

    // Light (Güneş objesini görselleştir)
    let light = world.spawn();
    world.add_component(
        light,
        gizmo::core::component::EntityName("Directional Light".to_string()),
    );
    world.add_component(
        light,
        Transform::new(Vec3::new(3.0, 4.0, 3.0)).with_rotation(Quat::from_axis_angle(
            Vec3::new(1.0, 0.5, 0.0).normalize(),
            -std::f32::consts::FRAC_PI_4,
        )),
    );
    world.add_component(light, gizmo::physics::components::GlobalTransform::default());
    world.add_component(
        light,
        gizmo::renderer::components::DirectionalLight::new(
            Vec3::new(1.0, 0.95, 0.9),
            1.5,
            gizmo::renderer::components::LightRole::Sun,
        ),
    );
    world.add_component(
        light,
        Collider::box_collider(gizmo::math::Vec3::new(0.5, 0.5, 0.5)),
    );

    // Light Icon (Kesişen prizmalar ile yıldız/güneş imgesi)
    let icon1 = world.spawn();
    world.add_component(
        icon1,
        gizmo::core::component::EntityName("Editor Light Icon 1".to_string()),
    );
    world.add_component(
        icon1,
        Transform::new(Vec3::ZERO).with_scale(Vec3::new(0.04, 0.6, 0.04)),
    );
    world.add_component(icon1, gizmo::physics::components::GlobalTransform::default());
    world.add_component(
        icon1,
        gizmo::renderer::asset::AssetManager::create_cube(&renderer.device),
    );
    world.add_component(
        icon1,
        gizmo::prelude::Material::new(white_tex.clone()).with_unlit(Vec4::new(1.0, 0.8, 0.1, 1.0)),
    );
    world.add_component(icon1, gizmo::renderer::components::MeshRenderer::new());
    world.add_component(icon1, gizmo::core::component::Parent(light.id()));

    let icon2 = world.spawn();
    world.add_component(
        icon2,
        gizmo::core::component::EntityName("Editor Light Icon 2".to_string()),
    );
    world.add_component(
        icon2,
        Transform::new(Vec3::ZERO).with_scale(Vec3::new(0.6, 0.04, 0.04)),
    );
    world.add_component(icon2, gizmo::physics::components::GlobalTransform::default());
    world.add_component(
        icon2,
        gizmo::renderer::asset::AssetManager::create_cube(&renderer.device),
    );
    world.add_component(
        icon2,
        gizmo::prelude::Material::new(white_tex.clone()).with_unlit(Vec4::new(1.0, 0.8, 0.1, 1.0)),
    );
    world.add_component(icon2, gizmo::renderer::components::MeshRenderer::new());
    world.add_component(icon2, gizmo::core::component::Parent(light.id()));

    let icon3 = world.spawn();
    world.add_component(
        icon3,
        gizmo::core::component::EntityName("Editor Light Icon 3".to_string()),
    );
    world.add_component(
        icon3,
        Transform::new(Vec3::ZERO).with_scale(Vec3::new(0.04, 0.04, 0.6)),
    );
    world.add_component(icon3, gizmo::physics::components::GlobalTransform::default());
    world.add_component(
        icon3,
        gizmo::renderer::asset::AssetManager::create_cube(&renderer.device),
    );
    world.add_component(
        icon3,
        gizmo::prelude::Material::new(white_tex.clone()).with_unlit(Vec4::new(1.0, 0.8, 0.1, 1.0)),
    );
    world.add_component(icon3, gizmo::renderer::components::MeshRenderer::new());
    world.add_component(icon3, gizmo::core::component::Parent(light.id()));

    world.add_component(
        light,
        gizmo::core::component::Children(vec![icon1.id(), icon2.id(), icon3.id()]),
    );

    // Editor Gizmos Root Node
    let gizmo_root = world.spawn();
    world.add_component(
        gizmo_root,
        gizmo::core::component::EntityName("Editor Guidelines".to_string()),
    );
    world.add_component(gizmo_root, Transform::new(Vec3::ZERO));
    world.add_component(gizmo_root, gizmo::physics::components::GlobalTransform::default());
    let mut gizmo_children = Vec::new();

    // Procedural 3D Grid Lines and Infinite Axes
    // HDR uyumlu, hafif transparan çok şık ve ferah bir Grid materyali
    let grid_entity = world.spawn();
    world.add_component(
        grid_entity,
        gizmo::core::component::EntityName("Editor Grid".to_string()),
    );
    world.add_component(grid_entity, Transform::new(Vec3::ZERO));
    world.add_component(grid_entity, gizmo::physics::components::GlobalTransform::default());
    world.add_component(grid_entity, gizmo::core::component::Parent(gizmo_root.id()));

    let grid_mesh =
        gizmo::renderer::asset::AssetManager::create_editor_grid_mesh(&renderer.device, 500.0);

    // Grid Material
    let mut grid_mat = gizmo::prelude::Material::new(white_tex.clone());
    grid_mat.albedo = gizmo::math::Vec4::new(1.0, 1.0, 1.0, 1.0);
    grid_mat.material_type = gizmo::renderer::components::MaterialType::Grid;

    // Editörün arka plan matris nesnesi olarak mesh ata. Pickable OLMASIN (fare engellemesin).
    world.add_component(grid_entity, grid_mesh);
    world.add_component(grid_entity, grid_mat);
    world.add_component(
        grid_entity,
        gizmo::renderer::components::MeshRenderer::new(),
    );
    gizmo_children.push(grid_entity.id());

    // Merkez Eksenler (Kırmızı/Mavi Çizgiler) artık Grid Shader (grid.wgsl) tarafından
    // tam kalbinde 0 GPU sızıntısı ile çizilmektedir! Bu yüzden gereksiz Transform küplerini sildik.

    // Attach all children to root
    world.add_component(gizmo_root, gizmo::core::component::Children(gizmo_children));

    // Default Object (To have something to interact with)
    let cube1 = world.spawn();
    world.add_component(
        cube1,
        gizmo::core::component::EntityName("Default Cube".to_string()),
    );
    world.add_component(cube1, Transform::new(Vec3::new(0.0, 0.0, 0.0))); // Tam Merkez!
    world.add_component(cube1, gizmo::physics::components::GlobalTransform::default());
    world.add_component(
        cube1,
        gizmo::renderer::asset::AssetManager::create_cube(&renderer.device),
    );
    world.add_component(
        cube1,
        gizmo::prelude::Material::new(white_tex.clone()).with_pbr(
            Vec4::new(0.21, 0.21, 0.21, 1.0),
            0.5,
            0.0,
        ),
    );
    world.add_component(cube1, gizmo::renderer::components::MeshRenderer::new());
    world.add_component(
        cube1,
        Collider::box_collider(gizmo::math::Vec3::new(1.0, 1.0, 1.0)),
    ); // Visual mesh is 2x2x2 (from -1 to +1)

    // Fizik sistemleri
    world.insert_resource(gizmo::physics::world::PhysicsWorld::new());

    // Custom Skybox or proper horizon color
    world.insert_resource(asset_manager);
    world.insert_resource(gizmo::core::asset::Assets::<gizmo::renderer::components::Mesh>::default());

    // Editor Camera
    let cam = world.spawn();
    world.add_component(
        cam,
        gizmo::core::component::EntityName("Editor Camera".to_string()),
    );
    let q_yaw = Quat::from_axis_angle(Vec3::new(0.0, 1.0, 0.0), -std::f32::consts::FRAC_PI_2);
    let q_pitch = Quat::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), -0.4);
    world.add_component(
        cam,
        Transform::new(Vec3::new(0.0, 8.0, 18.0)).with_rotation(q_yaw * q_pitch),
    );
    world.add_component(cam, gizmo::physics::components::GlobalTransform::default());
    world.add_component(
        cam,
        gizmo::renderer::components::Camera::new(
            60.0_f32.to_radians(),
            0.1,
            1000.0,
            -std::f32::consts::FRAC_PI_2,
            -0.4,
            true,
        ),
    );

    // --- GIZMO HANDLES (TRANSLATE) EGUI-GIZMO İÇİN İPTAL EDİLDİ ---

    // Game Camera (Play modunda kullanılacak oyun kamerası)
    let game_cam = world.spawn();
    world.add_component(
        game_cam,
        gizmo::core::component::EntityName("Game Camera".to_string()),
    );
    // Dövüş oyunu için yan görünüm: Z ekseninde 12 birim uzakta, hafif yukarıdan bakıyor
    let game_cam_pos = Vec3::new(0.0, 3.0, 12.0);
    let look_target = Vec3::new(0.0, 1.0, 0.0);
    let game_cam_forward = (look_target - game_cam_pos).normalize();
    let game_cam_yaw = game_cam_forward.x.atan2(-game_cam_forward.z);
    let game_cam_pitch = (-game_cam_forward.y).asin();
    world.add_component(
        game_cam,
        Transform::new(game_cam_pos),
    );
    world.add_component(game_cam, gizmo::physics::components::GlobalTransform::default());
    world.add_component(
        game_cam,
        gizmo::renderer::components::Camera::new(
            50.0_f32.to_radians(), // Biraz daha dar FOV (sinematik)
            0.1,
            500.0,
            game_cam_yaw,
            game_cam_pitch,
            false, // Editör kamerası DEĞİL
        ),
    );

    let mut editor_state = EditorState::new();
    editor_state.open = true; // Always open in Studio!

world.insert_resource(editor_state);

    let debug_cube = gizmo::renderer::asset::AssetManager::create_cube(&renderer.device);
    let debug_sphere = gizmo::renderer::asset::AssetManager::create_sphere(&renderer.device, 0.5, 16, 16);
    world.insert_resource(DebugAssets {
        cube: debug_cube,
        sphere: debug_sphere,
        white_tex: white_tex.clone(),
    });
    world.insert_resource(gizmo::renderer::Gizmos::default());
    world.insert_resource(gizmo::core::FrameProfiler::new());

// Register ComponentRegistry so Editor can list components
    let mut component_registry = gizmo::core::ComponentRegistry::new();
    component_registry.register::<gizmo::physics::components::Transform>("Transform");
    component_registry.register::<gizmo::physics::Velocity>("Velocity");
    component_registry.register::<gizmo::physics::RigidBody>("RigidBody");
    component_registry.register::<gizmo::physics::Collider>("Collider");
    component_registry.register::<gizmo::renderer::components::Camera>("Camera");
    component_registry.register::<gizmo::renderer::components::PointLight>("PointLight");
    component_registry.register::<gizmo::prelude::Material>("Material");
    component_registry.register::<gizmo::scripting::Script>("Script");
    component_registry.register::<gizmo::renderer::components::ParticleEmitter>("ParticleEmitter");
    component_registry.register::<gizmo::prelude::AudioSource>("AudioSource");
    component_registry.register::<gizmo::renderer::components::Terrain>("Terrain");
    component_registry.register::<gizmo::physics::components::Hitbox>("Hitbox");
    component_registry.register::<gizmo::physics::components::Hurtbox>("Hurtbox");
    component_registry.register::<gizmo::renderer::components::BoneAttachment>("BoneAttachment");
    component_registry.register::<gizmo::physics::components::fighter::FighterController>("FighterController");
    world.insert_resource(component_registry);

    // --- SCRIPT ENGINE & ASSET WATCHER BİRLEŞİMİ ---
    if let Ok(engine) = gizmo::scripting::ScriptEngine::new() {
        tracing::info!("🚀 Gizmo Studio Script Motoru Başlatıldı.");
        // Olası scriptleri preload yapabiliriz
        world.insert_resource(engine);
    } else {
        tracing::info!("❌ HATA: Gizmo Studio Script Motoru Başlatılamadı!");
    }

    StudioState {
        current_fps: 0.0,
        actual_dt: 0.0,
        editor_camera: cam.id(),
        game_camera: game_cam.id(),
        do_raycast: false,
        physics_accumulator: 0.0,
        asset_watcher: gizmo::renderer::hot_reload::AssetWatcher::new(&["demo/assets", "scripts"]),
        gc_timer: 0.0,
        autosave_timer: 0.0,
        visible_entity_count: 0,
        draw_call_count: 0,
    }
}
