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
    world.add_component(
        light,
        gizmo::renderer::components::DirectionalLight::new(Vec3::new(1.0, 0.95, 0.9), 1.5, true),
    );
    world.add_component(light, Collider::new_aabb(0.5, 0.5, 0.5));

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
    let mut gizmo_children = Vec::new();

    let axis_x_mat =
        gizmo::prelude::Material::new(white_tex.clone()).with_unlit(Vec4::new(0.8, 0.1, 0.1, 1.0)); // Kırmızı (X)
    let _axis_y_mat =
        gizmo::prelude::Material::new(white_tex.clone()).with_unlit(Vec4::new(0.1, 0.8, 0.1, 1.0)); // Yeşil (Y)
    let axis_z_mat =
        gizmo::prelude::Material::new(white_tex.clone()).with_unlit(Vec4::new(0.1, 0.4, 0.9, 1.0)); // Mavi (Z)

    // Procedural 3D Grid Lines and Infinite Axes
    // HDR uyumlu, hafif transparan çok şık ve ferah bir Grid materyali
    let grid_mat = gizmo::prelude::Material::new(white_tex.clone())
        // Shader'da 'if (alpha < 0.5) discard;' kontrolü olduğu için alfa daima > 0.5 olmalı.
        // Silik görünüm için RGB'yi kısıp alfayı 0.55'te tutuyoruz.
        .with_unlit(Vec4::new(0.15, 0.15, 0.15, 0.55))
        .with_transparent(true);

    // Tekil Procedural Grid (1 Entity, 1 Draw Call - Zero Entity Overhead)
    let grid_mesh = gizmo::renderer::asset::AssetManager::create_editor_grid_mesh(&renderer.device, 100, 1.0);
    let grid_entity = world.spawn();
    world.add_component(grid_entity, Transform::new(Vec3::ZERO));
    world.add_component(grid_entity, grid_mesh);
    world.add_component(grid_entity, grid_mat);
    world.add_component(grid_entity, gizmo::renderer::components::MeshRenderer::new());
    world.add_component(grid_entity, gizmo::core::component::Parent(gizmo_root.id()));
    gizmo_children.push(grid_entity.id());

    // Merkez X Ekseni (Kırmızı Çizgi)
    let len = 200.0;
    let center_width = 0.035;
    
    let center_x = world.spawn();
    world.add_component(center_x, Transform::new(Vec3::ZERO).with_scale(Vec3::new(len, center_width, center_width)));
    world.add_component(center_x, gizmo::renderer::asset::AssetManager::create_cube(&renderer.device));
    world.add_component(center_x, axis_x_mat.clone());
    world.add_component(center_x, gizmo::renderer::components::MeshRenderer::new());
    world.add_component(center_x, gizmo::core::component::Parent(gizmo_root.id()));
    gizmo_children.push(center_x.id());

    // Merkez Z Ekseni (Mavi Çizgi)
    let center_z = world.spawn();
    world.add_component(center_z, Transform::new(Vec3::ZERO).with_scale(Vec3::new(center_width, center_width, len)));
    world.add_component(center_z, gizmo::renderer::asset::AssetManager::create_cube(&renderer.device));
    world.add_component(center_z, axis_z_mat.clone());
    world.add_component(center_z, gizmo::renderer::components::MeshRenderer::new());
    world.add_component(center_z, gizmo::core::component::Parent(gizmo_root.id()));
    gizmo_children.push(center_z.id());

    // Attach all children to root
    world.add_component(gizmo_root, gizmo::core::component::Children(gizmo_children));

    // Default Object (To have something to interact with)
    let cube1 = world.spawn();
    world.add_component(
        cube1,
        gizmo::core::component::EntityName("Default Cube".to_string()),
    );
    world.add_component(cube1, Transform::new(Vec3::new(0.0, 0.0, 0.0))); // Tam Merkez!
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
    world.add_component(cube1, Collider::new_aabb(1.0, 1.0, 1.0)); // Visual mesh is 2x2x2 (from -1 to +1)

    // Custom Skybox or proper horizon color
    world.insert_resource(asset_manager);

    // Editor Camera
    let cam = world.spawn();
    let q_yaw = Quat::from_axis_angle(Vec3::new(0.0, 1.0, 0.0), -std::f32::consts::FRAC_PI_2);
    let q_pitch = Quat::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), -0.4);
    world.add_component(
        cam,
        Transform::new(Vec3::new(0.0, 8.0, 18.0)).with_rotation(q_yaw * q_pitch),
    );
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

    // Highlight Box (Selection Outline Representation)
    let highlight_box = world.spawn();
    world.add_component(
        highlight_box,
        gizmo::core::component::EntityName("Highlight Box".to_string()),
    );
    world.add_component(highlight_box, Transform::new(Vec3::new(0.0, -10000.0, 0.0))); // Hide initially
    world.add_component(
        highlight_box,
        gizmo::renderer::asset::AssetManager::create_cube(&renderer.device),
    );
    world.add_component(
        highlight_box,
        gizmo::prelude::Material::new(white_tex.clone())
            .with_unlit(Vec4::new(0.05, 0.45, 1.0, 0.3))
            .with_transparent(true),
    );
    world.add_component(
        highlight_box,
        gizmo::renderer::components::MeshRenderer::new(),
    );
    // --- GIZMO HANDLES (TRANSLATE) EGUI-GIZMO İÇİN İPTAL EDİLDİ ---
    
    let mut editor_state = EditorState::new();
    editor_state.open = true; // Always open in Studio!
    editor_state.highlight_box = highlight_box.id();

    world.insert_resource(editor_state);

    let debug_cube = gizmo::renderer::asset::AssetManager::create_cube(&renderer.device);
    world.insert_resource(DebugAssets {
        cube: debug_cube,
        white_tex: white_tex.clone(),
    });

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
        actual_dt: 0.0,
        editor_camera: cam.id(),
        do_raycast: false,
        physics_accumulator: 0.0,
        asset_watcher: gizmo::renderer::hot_reload::AssetWatcher::new(&["demo/assets", "scripts"]),
    }
}
