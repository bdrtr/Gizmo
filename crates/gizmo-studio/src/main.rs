use gizmo::prelude::*;
use gizmo::math::{Vec3, Quat, Vec4};
use gizmo::editor::{EditorState};
use gizmo::physics::components::Transform;

pub struct StudioState {
    pub current_fps: f32,
    pub editor_camera: u32,
    pub do_raycast: bool,
}

pub struct DebugAssets {
    pub cube: gizmo::renderer::components::Mesh,
    pub white_tex: std::sync::Arc<gizmo::wgpu::BindGroup>,
}

pub mod render_pipeline;
pub mod studio_input;
pub use studio_input::*;

fn main() {
    let mut app = App::<StudioState>::new("Gizmo Studio", 1600, 900)
        .with_icon(include_bytes!("../../../media/logo.png"));

    app = app.set_setup(|world, renderer| {
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
        world.add_component(icon1, Transform::new(Vec3::ZERO).with_scale(Vec3::new(0.04, 0.6, 0.04)));
        world.add_component(icon1, gizmo::renderer::asset::AssetManager::create_cube(&renderer.device));
        world.add_component(icon1, gizmo::prelude::Material::new(white_tex.clone()).with_unlit(Vec4::new(1.0, 0.8, 0.1, 1.0)));
        world.add_component(icon1, gizmo::renderer::components::MeshRenderer::new());
        world.add_component(icon1, gizmo::core::component::Parent(light.id()));

        let icon2 = world.spawn();
        world.add_component(icon2, Transform::new(Vec3::ZERO).with_scale(Vec3::new(0.6, 0.04, 0.04)));
        world.add_component(icon2, gizmo::renderer::asset::AssetManager::create_cube(&renderer.device));
        world.add_component(icon2, gizmo::prelude::Material::new(white_tex.clone()).with_unlit(Vec4::new(1.0, 0.8, 0.1, 1.0)));
        world.add_component(icon2, gizmo::renderer::components::MeshRenderer::new());
        world.add_component(icon2, gizmo::core::component::Parent(light.id()));
        
        let icon3 = world.spawn();
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

        // Center XYZ Axis Lines (Gizmos)
        let axis_x = world.spawn();
        world.add_component(axis_x, gizmo::core::component::EntityName("Axis X".to_string()));
        let q_x = gizmo::math::Quat::from_axis_angle(Vec3::new(0.0, 0.0, 1.0), -std::f32::consts::FRAC_PI_2);
        world.add_component(axis_x, Transform::new(Vec3::new(0.0, -10000.0, 0.0)).with_rotation(q_x));
        world.add_component(axis_x, gizmo::renderer::asset::AssetManager::create_gizmo_arrow(&renderer.device));
        world.add_component(axis_x, gizmo::prelude::Material::new(white_tex.clone()).with_unlit(Vec4::new(1.0, 0.0, 0.0, 1.0)));
        world.add_component(axis_x, gizmo::renderer::components::MeshRenderer::new());
        world.add_component(axis_x, Collider::new_aabb(0.15, 0.8, 0.15)); // Lokal uzayda Visual Arrow'a uyumlu (Y ekseni etrafında dar bar)
        world.add_component(axis_x, gizmo::core::component::Parent(gizmo_root.id()));
        gizmo_children.push(axis_x.id());

        let axis_y = world.spawn();
        world.add_component(axis_y, gizmo::core::component::EntityName("Axis Y".to_string()));
        world.add_component(axis_y, Transform::new(Vec3::new(0.0, -10000.0, 0.0)));
        world.add_component(axis_y, gizmo::renderer::asset::AssetManager::create_gizmo_arrow(&renderer.device));
        world.add_component(axis_y, gizmo::prelude::Material::new(white_tex.clone()).with_unlit(Vec4::new(0.0, 1.0, 0.0, 1.0)));
        world.add_component(axis_y, gizmo::renderer::components::MeshRenderer::new());
        world.add_component(axis_y, Collider::new_aabb(0.15, 0.8, 0.15)); // Lokal Y etrafında
        world.add_component(axis_y, gizmo::core::component::Parent(gizmo_root.id()));
        gizmo_children.push(axis_y.id());

        let axis_z = world.spawn();
        world.add_component(axis_z, gizmo::core::component::EntityName("Axis Z".to_string()));
        let q_z = gizmo::math::Quat::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), std::f32::consts::FRAC_PI_2);
        world.add_component(axis_z, Transform::new(Vec3::new(0.0, -10000.0, 0.0)).with_rotation(q_z));
        world.add_component(axis_z, gizmo::renderer::asset::AssetManager::create_gizmo_arrow(&renderer.device));
        world.add_component(axis_z, gizmo::prelude::Material::new(white_tex.clone()).with_unlit(Vec4::new(0.0, 0.0, 1.0, 1.0)));
        world.add_component(axis_z, gizmo::renderer::components::MeshRenderer::new());
        world.add_component(axis_z, Collider::new_aabb(0.15, 0.8, 0.15)); // Lokal Y etrafında
        world.add_component(axis_z, gizmo::core::component::Parent(gizmo_root.id()));
        gizmo_children.push(axis_z.id());

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

        // Editor State Initialization
        let mut editor_state = EditorState::new();
        editor_state.open = true; // Always open in Studio!
        editor_state.gizmo_x = axis_x.id();
        editor_state.gizmo_y = axis_y.id();
        editor_state.gizmo_z = axis_z.id();
        editor_state.highlight_box = highlight_box.id();
        
        world.insert_resource(editor_state);

        let debug_cube = gizmo::renderer::asset::AssetManager::create_cube(&renderer.device);
        world.insert_resource(DebugAssets { cube: debug_cube, white_tex: white_tex.clone() });

        StudioState {
            current_fps: 0.0,
            editor_camera: cam.id(),
            do_raycast: false,
        }
    });

    app = app.set_update(|world, state, dt, input| {
        state.current_fps = 1.0 / dt;
        
        let mut look_delta = None;
        if let Some(mut editor_state) = world.remove_resource::<EditorState>() {
            look_delta = editor_state.camera_look_delta;
            // Editör Scene View üzerinden gelen NDC ve raycast tetiğini okuyalım
            if let Some(ndc) = editor_state.mouse_ndc {
                let (ww, wh) = input.window_size(); 
                let aspect = if let Some(rect) = editor_state.scene_view_rect { rect.width() / rect.height() } else { ww / wh };
                
                let current_ray = studio_input::build_ray(world, state.editor_camera, ndc.x, ndc.y, aspect, 1.0);
                if let Some(ray) = current_ray {
                    let do_rc = editor_state.do_raycast;
                    if do_rc {
                        editor_state.do_raycast = false;
                        state.do_raycast = false;
                    }
                    studio_input::handle_studio_input(world, &mut editor_state, ray, state.editor_camera, do_rc);
                }
            }
            
            studio_input::sync_gizmos(world, &editor_state);

            // GIZMO DEBUG RENDERER: Spawn and Despawn logic
            // Zamanlayıcısı dolanları sil
            let mut surviving_entities = Vec::new();
            for (timer, ent) in editor_state.debug_spawned_entities.drain(..) {
                if timer - dt > 0.0 {
                    surviving_entities.push((timer - dt, ent));
                } else {
                    world.despawn_by_id(ent);
                }
            }
            editor_state.debug_spawned_entities = surviving_entities;

            // Yeni debug istekleri spawnla
            if !editor_state.debug_draw_requests.is_empty() {
                let mut pending_debug_assets = None;
                if let Some(debug_assets) = world.get_resource::<DebugAssets>() {
                    pending_debug_assets = Some((debug_assets.cube.clone(), debug_assets.white_tex.clone()));
                }
                
                if let Some((cube, white_tex)) = pending_debug_assets {
                    let reqs = std::mem::take(&mut editor_state.debug_draw_requests);
                    for (pos, rot, scale, color) in reqs {
                        let e = world.spawn();
                        world.add_component(e, Transform::new(pos).with_rotation(rot).with_scale(scale));
                        world.add_component(e, cube.clone());
                        let mut mat = gizmo::prelude::Material::new(white_tex.clone()).with_unlit(color);
                        if color.w < 0.99 {
                            mat = mat.with_transparent(true);
                        }
                        world.add_component(e, mat);
                        world.add_component(e, gizmo::renderer::components::MeshRenderer::new());
                        editor_state.debug_spawned_entities.push((2.0, e.id())); // 2 saniye kalsın
                    }
                } else {
                    editor_state.debug_draw_requests.clear();
                }
            }

            world.insert_resource(editor_state);
        }

        // Editor Camera WASD Controller
        if let (Some(mut transforms), Some(mut cameras)) = (world.borrow_mut::<Transform>(), world.borrow_mut::<gizmo::renderer::components::Camera>()) {
            if let (Some(t), Some(cam)) = (transforms.get_mut(state.editor_camera), cameras.get_mut(state.editor_camera)) {
                
                // 1. Mouse Look (Egui üzerinden gelen delta okuması)
                if let Some(delta) = look_delta {
                    let sensitivity = 0.003;
                    
                    cam.yaw += delta.x * sensitivity;
                    cam.pitch -= delta.y * sensitivity;
                    
                    // Gimbal Lock'u (tepetaklak olmayı) önle
                    let max_pitch = 89.0_f32.to_radians();
                    if cam.pitch > max_pitch { cam.pitch = max_pitch; }
                    if cam.pitch < -max_pitch { cam.pitch = -max_pitch; }
                    
                    // Transform rotasyonunu kameraya uydur (motor içi tutarlılık için)
                    let q_yaw = gizmo::math::Quat::from_axis_angle(gizmo::math::Vec3::new(0.0, 1.0, 0.0), cam.yaw);
                    let q_pitch = gizmo::math::Quat::from_axis_angle(gizmo::math::Vec3::new(1.0, 0.0, 0.0), cam.pitch);
                    t.rotation = q_yaw * q_pitch;
                }
                
                // 2. Serbest Uçuş (WASD + Q/E)
                let speed = if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::ShiftLeft as u32) { 20.0 } else { 8.0 };
                
                let forward = cam.get_front();
                let right = forward.cross(gizmo::math::Vec3::new(0.0, 1.0, 0.0)).normalize();
                let up = gizmo::math::Vec3::new(0.0, 1.0, 0.0);
                
                let mut move_dir = gizmo::math::Vec3::ZERO;
                // Kamera nereye bakıyorsa ORAYA ileri git
                if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::KeyW as u32) { move_dir += forward; }
                if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::KeyS as u32) { move_dir -= forward; }
                if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::KeyA as u32) { move_dir -= right; }
                if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::KeyD as u32) { move_dir += right; }
                // Dünyaya göre yukarı/aşağı tırmanış
                if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::KeyE as u32) { move_dir += up; }
                if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::KeyQ as u32) { move_dir -= up; }
                
                t.position += move_dir.normalize_or_zero() * (speed * dt);
            }
        }
    });

    app = app.set_ui(|world, _state, ctx| {
        // Draw the editor filling the screen
        if let Some(mut editor_state) = world.get_resource_mut::<EditorState>() {
            egui::CentralPanel::default().show(ctx, |_ui| {
                gizmo::editor::draw_editor(ctx, world, &mut editor_state);
            });
        }
    });

    app = app.set_render(|world, state, encoder, view, renderer, light_time| {
        let mut save_req = None;
        let mut load_req = None;
        let mut prefab_save_req = None;
        let mut prefab_load_req = None;
        let mut duplicate_req = None;

        if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
            save_req = ed.scene_save_request.take();
            load_req = ed.scene_load_request.take();
            prefab_save_req = ed.prefab_save_request.take();
            prefab_load_req = ed.prefab_load_request.take();
            duplicate_req = ed.duplicate_request.take();
        }

        if let Some(path) = save_req {
            gizmo::scene::SceneData::save(world, &path);
            if let Some(mut ed) = world.get_resource_mut::<EditorState>() { ed.log_info("Sahne kaydedildi."); }
        }

        if let Some(path) = load_req {
            let ents = world.alive_entities();
            for e in ents {
                if e.id() != state.editor_camera {
                    world.despawn(e);
                }
            }
            if let Some(mut asset_manager) = world.remove_resource::<gizmo::renderer::asset::AssetManager>() {
                let dummy_rgba = [255, 255, 255, 255];
                let dummy_bg = renderer.create_texture(&dummy_rgba, 1, 1);
                gizmo::scene::SceneData::load_into(
                    &path,
                    world,
                    &renderer.device,
                    &renderer.queue,
                    &renderer.scene.texture_bind_group_layout,
                    &mut asset_manager,
                    std::sync::Arc::new(dummy_bg)
                );
                world.insert_resource(asset_manager);
                if let Some(mut ed) = world.get_resource_mut::<EditorState>() { ed.log_info("Sahne yüklendi."); }
            }
        }
        
        if let Some((ent_id, path)) = prefab_save_req {
            gizmo::scene::SceneData::save_prefab(world, ent_id, &path);
            if let Some(mut ed) = world.get_resource_mut::<EditorState>() { ed.log_info("Prefab kaydedildi."); }
        }
        
        if let Some((path, parent)) = prefab_load_req {
            if let Some(mut asset_manager) = world.remove_resource::<gizmo::renderer::asset::AssetManager>() {
                let dummy_rgba = [255, 255, 255, 255];
                let dummy_bg = renderer.create_texture(&dummy_rgba, 1, 1);
                gizmo::scene::SceneData::load_prefab(
                    &path,
                    parent,
                    world,
                    &renderer.device,
                    &renderer.queue,
                    &renderer.scene.texture_bind_group_layout,
                    &mut asset_manager,
                    std::sync::Arc::new(dummy_bg)
                );
                world.insert_resource(asset_manager);
                if let Some(mut ed) = world.get_resource_mut::<EditorState>() { ed.log_info("Prefab yüklendi."); }
            }
        }
        
        if let Some(ent_id) = duplicate_req {
             let temp_path = "demo/assets/prefabs/temp_duplicate.prefab";
             gizmo::scene::SceneData::save_prefab(world, ent_id, temp_path);
             
             if let Some(mut asset_manager) = world.remove_resource::<gizmo::renderer::asset::AssetManager>() {
                let dummy_rgba = [255, 255, 255, 255];
                let dummy_bg = renderer.create_texture(&dummy_rgba, 1, 1);
                gizmo::scene::SceneData::load_prefab(
                    temp_path,
                    None,
                    world,
                    &renderer.device,
                    &renderer.queue,
                    &renderer.scene.texture_bind_group_layout,
                    &mut asset_manager,
                    std::sync::Arc::new(dummy_bg)
                );
                world.insert_resource(asset_manager);
                if let Some(mut ed) = world.get_resource_mut::<EditorState>() { ed.log_info("Obje çoğaltıldı."); }
            }
        }

        render_pipeline::execute_render_pipeline(world, state, encoder, view, renderer, light_time);
    });

    app.run();
}
