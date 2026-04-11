use gizmo::prelude::*;
use gizmo::math::{Vec3, Quat, Vec4};
use gizmo::editor::EditorState;
use gizmo::physics::components::Transform;
use crate::state::{StudioState, DebugAssets};

pub fn setup_studio_scene(world: &mut World, renderer: &gizmo::renderer::Renderer) -> StudioState {
    // --- Setup Editor Scene (Grid & Axes) ---
    let mut asset_manager = gizmo::renderer::asset::AssetManager::new();
    let white_tex = asset_manager.create_white_texture(&renderer.device, &renderer.queue, &renderer.scene.texture_bind_group_layout);
    
    // Light (Güneş objesini görselleştir)
    let light = world.spawn();
    world.add_component(light, gizmo::core::component::EntityName("Directional Light".to_string()));
    world.add_component(light, Transform::new(Vec3::new(3.0, 4.0, 3.0))
        .with_rotation(Quat::from_axis_angle(Vec3::new(1.0, 0.5, 0.0).normalize(), -std::f32::consts::FRAC_PI_4)));
    world.add_component(light, gizmo::renderer::components::DirectionalLight::new(Vec3::new(1.0, 0.95, 0.9), 1.5, true));
    world.add_component(light, Collider::new_aabb(0.5, 0.5, 0.5));

    // Light Icon (Kesişen prizmalar ile yıldız/güneş imgesi)
    let icon1 = world.spawn();
    world.add_component(icon1, gizmo::core::component::EntityName("Editor Light Icon 1".to_string()));
    world.add_component(icon1, Transform::new(Vec3::ZERO).with_scale(Vec3::new(0.04, 0.6, 0.04)));
    world.add_component(icon1, gizmo::renderer::asset::AssetManager::create_cube(&renderer.device));
    world.add_component(icon1, gizmo::prelude::Material::new(white_tex.clone()).with_unlit(Vec4::new(1.0, 0.8, 0.1, 1.0)));
    world.add_component(icon1, gizmo::renderer::components::MeshRenderer::new());
    world.add_component(icon1, gizmo::core::component::Parent(light.id()));

    let icon2 = world.spawn();
    world.add_component(icon2, gizmo::core::component::EntityName("Editor Light Icon 2".to_string()));
    world.add_component(icon2, Transform::new(Vec3::ZERO).with_scale(Vec3::new(0.6, 0.04, 0.04)));
    world.add_component(icon2, gizmo::renderer::asset::AssetManager::create_cube(&renderer.device));
    world.add_component(icon2, gizmo::prelude::Material::new(white_tex.clone()).with_unlit(Vec4::new(1.0, 0.8, 0.1, 1.0)));
    world.add_component(icon2, gizmo::renderer::components::MeshRenderer::new());
    world.add_component(icon2, gizmo::core::component::Parent(light.id()));
    
    let icon3 = world.spawn();
    world.add_component(icon3, gizmo::core::component::EntityName("Editor Light Icon 3".to_string()));
    world.add_component(icon3, Transform::new(Vec3::ZERO).with_scale(Vec3::new(0.04, 0.04, 0.6)));
    world.add_component(icon3, gizmo::renderer::asset::AssetManager::create_cube(&renderer.device));
    world.add_component(icon3, gizmo::prelude::Material::new(white_tex.clone()).with_unlit(Vec4::new(1.0, 0.8, 0.1, 1.0)));
    world.add_component(icon3, gizmo::renderer::components::MeshRenderer::new());
    world.add_component(icon3, gizmo::core::component::Parent(light.id()));

    world.add_component(light, gizmo::core::component::Children(vec![icon1.id(), icon2.id(), icon3.id()]));

    // Editor Gizmos Root Node
    let gizmo_root = world.spawn();
    world.add_component(gizmo_root, gizmo::core::component::EntityName("Editor Guidelines".to_string()));
    world.add_component(gizmo_root, Transform::new(Vec3::ZERO));
    let mut gizmo_children = Vec::new();

    let axis_x_mat = gizmo::prelude::Material::new(white_tex.clone()).with_unlit(Vec4::new(0.8, 0.1, 0.1, 1.0)); // Kırmızı (X)
    let _axis_y_mat = gizmo::prelude::Material::new(white_tex.clone()).with_unlit(Vec4::new(0.1, 0.8, 0.1, 1.0)); // Yeşil (Y)
    let axis_z_mat = gizmo::prelude::Material::new(white_tex.clone()).with_unlit(Vec4::new(0.1, 0.4, 0.9, 1.0)); // Mavi (Z)

    // Procedural 3D Grid Lines and Infinite Axes
    // HDR uyumlu, hafif transparan çok şık ve ferah bir Grid materyali
    let grid_mat = gizmo::prelude::Material::new(white_tex.clone())
        .with_unlit(Vec4::new(0.2, 0.2, 0.2, 0.4))
        .with_transparent(true);

    for i in -50..=50 {
        // Her kare 10.0 metre genişliğinde çok ferah bir zemin
        let offset = i as f32 * 10.0;

        let is_center = i == 0;
        // Merkezi eksen kılavuzları sonsuza, normal ızgara çizgileri +500/-500 uzantıya gidiyor
        let len = if is_center { 10000.0 } else { 1000.0 };
        
        // Çizgiler çok ince ve zarif
        let t_width = if is_center { 0.02 } else { 0.01 };
        
        // X eksenine paralel çizgiler
        let mat_x = if is_center { axis_x_mat.clone() } else { grid_mat.clone() };
        let line_x = world.spawn();
        world.add_component(line_x, Transform::new(Vec3::new(0.0, 0.0, offset)).with_scale(Vec3::new(len, t_width, t_width)));
        world.add_component(line_x, gizmo::renderer::asset::AssetManager::create_cube(&renderer.device));
        world.add_component(line_x, mat_x);
        world.add_component(line_x, gizmo::renderer::components::MeshRenderer::new());
        world.add_component(line_x, gizmo::core::component::Parent(gizmo_root.id()));
        gizmo_children.push(line_x.id());

        // Z eksenine paralel çizgiler
        let mat_z = if is_center { axis_z_mat.clone() } else { grid_mat.clone() };
        let line_z = world.spawn();
        world.add_component(line_z, Transform::new(Vec3::new(offset, 0.0, 0.0)).with_scale(Vec3::new(t_width, t_width, len)));
        world.add_component(line_z, gizmo::renderer::asset::AssetManager::create_cube(&renderer.device));
        world.add_component(line_z, mat_z);
        world.add_component(line_z, gizmo::renderer::components::MeshRenderer::new());
        world.add_component(line_z, gizmo::core::component::Parent(gizmo_root.id()));
        gizmo_children.push(line_z.id());
    }

    // Attach all children to root
    world.add_component(gizmo_root, gizmo::core::component::Children(gizmo_children));

    // Default Object (To have something to interact with)
    let cube1 = world.spawn();
    world.add_component(cube1, gizmo::core::component::EntityName("Default Cube".to_string()));
    world.add_component(cube1, Transform::new(Vec3::new(0.0, 0.0, 0.0))); // Tam Merkez!
    world.add_component(cube1, gizmo::renderer::asset::AssetManager::create_cube(&renderer.device));
    world.add_component(cube1, gizmo::prelude::Material::new(white_tex.clone()).with_pbr(Vec4::new(0.21, 0.21, 0.21, 1.0), 0.5, 0.0));
    world.add_component(cube1, gizmo::renderer::components::MeshRenderer::new());
    world.add_component(cube1, Collider::new_aabb(1.0, 1.0, 1.0)); // Visual mesh is 2x2x2 (from -1 to +1)

    // Custom Skybox or proper horizon color
    world.insert_resource(asset_manager);

    // Editor Camera
    let cam = world.spawn();
    let q_yaw = Quat::from_axis_angle(Vec3::new(0.0, 1.0, 0.0), -std::f32::consts::FRAC_PI_2);
    let q_pitch = Quat::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), -0.4);
    world.add_component(cam, Transform::new(Vec3::new(0.0, 8.0, 18.0)).with_rotation(q_yaw * q_pitch));
    world.add_component(cam, gizmo::renderer::components::Camera::new(
        60.0_f32.to_radians(), 0.1, 1000.0, -std::f32::consts::FRAC_PI_2, -0.4, true
    ));

    // Highlight Box (Selection Outline Representation)
    let highlight_box = world.spawn();
    world.add_component(highlight_box, gizmo::core::component::EntityName("Highlight Box".to_string()));
    world.add_component(highlight_box, Transform::new(Vec3::new(0.0, -10000.0, 0.0))); // Hide initially
    world.add_component(highlight_box, gizmo::renderer::asset::AssetManager::create_cube(&renderer.device));
    world.add_component(highlight_box, gizmo::prelude::Material::new(white_tex.clone()).with_unlit(Vec4::new(0.05, 0.45, 1.0, 0.3)).with_transparent(true));
    world.add_component(highlight_box, gizmo::renderer::components::MeshRenderer::new());
    // --- GIZMO HANDLES (TRANSLATE) ---
    let mut create_handle = |name: &str, mat: gizmo::renderer::components::Material, extents: Vec3, pos_offset: Vec3| -> u32 {
        let ent = world.spawn();
        world.add_component(ent, gizmo::core::component::EntityName(name.to_string()));
        world.add_component(ent, Transform::new(pos_offset).with_scale(extents));
        world.add_component(ent, gizmo::renderer::asset::AssetManager::create_cube(&renderer.device));
        world.add_component(ent, mat);
        world.add_component(ent, gizmo::renderer::components::MeshRenderer::new());
        // Normal bounding box is 1.0 half extents, so bounding_box_half_extents equals scale
        world.add_component(ent, Collider::new_aabb(extents.x, extents.y, extents.z));
        world.add_component(ent, gizmo::core::component::IsHidden); // Hide initially
        ent.id()
    };

    let thickness = 0.08;
    let length = 1.5;
    let handle_x = create_handle("Editor Gizmo Handle X", axis_x_mat.clone(), Vec3::new(length, thickness, thickness), Vec3::new(length, 0.0, 0.0));
    let handle_y = create_handle("Editor Gizmo Handle Y", _axis_y_mat.clone(), Vec3::new(thickness, length, thickness), Vec3::new(0.0, length, 0.0));
    let handle_z = create_handle("Editor Gizmo Handle Z", axis_z_mat.clone(), Vec3::new(thickness, thickness, length), Vec3::new(0.0, 0.0, length));

    let mut editor_state = EditorState::new();
    editor_state.open = true; // Always open in Studio!
    editor_state.highlight_box = highlight_box.id();
    editor_state.gizmo_handles = [handle_x, handle_y, handle_z];
    
    world.insert_resource(editor_state);

    let debug_cube = gizmo::renderer::asset::AssetManager::create_cube(&renderer.device);
    world.insert_resource(DebugAssets { cube: debug_cube, white_tex: white_tex.clone() });

    // --- SCRIPT ENGINE & ASSET WATCHER BİRLEŞİMİ ---
    if let Ok(engine) = gizmo::scripting::ScriptEngine::new() {
        println!("🚀 Gizmo Studio Script Motoru Başlatıldı.");
        // Olası scriptleri preload yapabiliriz
        world.insert_resource(engine);
    } else {
        println!("❌ HATA: Gizmo Studio Script Motoru Başlatılamadı!");
    }

    StudioState {
        current_fps: 0.0,
        editor_camera: cam.id(),
        do_raycast: false,
        physics_accumulator: 0.0,
        asset_watcher: gizmo::renderer::hot_reload::AssetWatcher::new(&["demo/assets", "scripts"]),
    }
}
