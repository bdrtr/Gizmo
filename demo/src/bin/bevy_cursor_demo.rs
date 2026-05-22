//! Port of Bevy's camera viewport_to_world raycast cursor example to Gizmo Engine.
//! This demo spawns a simple flat green plane, a camera, and a directional light,
//! and draws a white circle at the cursor's intersection with the ground.

use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};
use gizmo::core::system::{Res, ResMut, IntoSystemConfig, Phase};
use gizmo::core::query::Query;
use gizmo::core::input::Input;

fn main() {
    let mut app = gizmo::app::App::<SimpleSceneState>::new("Gizmo Engine - Bevy Cursor Demo Parity", 1280, 720);

    app = app
        .with_simple_scene(|scene, state| {
            // Expansive premium green ground plane (srgb(0.2, 0.45, 0.2))
            let mesh = AssetManager::create_plane(&scene.renderer.device, 40.0);
            let tex = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let mat = Material::new(tex).with_pbr(Vec4::new(0.07, 0.21, 0.07, 1.0), 1.0, 0.0);

            let ent = scene.world.spawn();
            scene.world.add_component(ent, Transform::new(Vec3::ZERO));
            scene.world.add_component(ent, GlobalTransform::default());
            scene.world.add_component(ent, mesh);
            scene.world.add_component(ent, mat);
            scene.world.add_component(ent, MeshRenderer::new());
            scene.world.add_bundle(ent, RigidBodyBundle::static_body().with_collider(Collider::plane(Vec3::Y, 0.0)));

            // Directional Light (Sun)
            let sun_ent = scene.world.spawn();
            let mut sun_bundle = DirectionalLightBundle::default();
            sun_bundle.rotation = Quat::from_rotation_x(-std::f32::consts::FRAC_PI_4);
            sun_bundle.intensity = 0.8;
            sun_bundle.color = Vec3::new(1.0, 1.0, 1.0);
            sun_bundle.apply(scene.world, sun_ent);

            // Camera setup looking at Vec3::ZERO from (0.0, 10.0, 15.0) to match Bevy's camera example
            scene.spawn_camera(state, Vec3::new(0.0, 10.0, 15.0), Vec3::ZERO);

            // Insert Gizmos resource with depth testing disabled to prevent flickering/z-fighting
            let mut debug_gizmos = gizmo::renderer::Gizmos::default();
            debug_gizmos.depth_test = false;
            scene.world.insert_resource(debug_gizmos);
        })
        .add_system(draw_cursor.in_phase(Phase::Update));

    app.run();
}

fn draw_cursor(
    mut gizmos: ResMut<gizmo::renderer::Gizmos>,
    win_info: Res<WindowInfo>,
    input: Res<Input>,
    q_cam: Query<(&Transform, &gizmo::renderer::components::Camera)>,
) {
    gizmos.depth_test = false; // FORCE depth testing off to guarantee lines are rendered on top
    let (mouse_x, mouse_y) = input.mouse_position();

    // Check if mouse is within window boundaries
    if mouse_x < 0.0 || mouse_y < 0.0 || mouse_x > win_info.width || mouse_y > win_info.height {
        return; // Early exit: don't draw anything when mouse is outside the window
    }

    let mut cam_data = None;
    for (_id, (trans, cam)) in q_cam.iter() {
        if cam.primary {
            cam_data = Some((trans.position, cam));
            break;
        }
    }

    let (cam_pos, cam) = match cam_data {
        Some(data) => data,
        None => return, // Early exit if no primary camera is found
    };

    let ndc_x = (mouse_x / win_info.width) * 2.0 - 1.0;
    let ndc_y = 1.0 - (mouse_y / win_info.height) * 2.0;

    let aspect = win_info.aspect_ratio();
    let proj = cam.get_projection(aspect);
    let view = cam.get_view(cam_pos);
    let view_proj_inv = (proj * view).inverse();

    let near_ndc = Vec4::new(ndc_x, ndc_y, 0.0, 1.0);
    let far_ndc = Vec4::new(ndc_x, ndc_y, 1.0, 1.0);

    let mut near_world = view_proj_inv * near_ndc;
    if near_world.w.abs() > 1e-6 {
        near_world /= near_world.w;
    }
    let mut far_world = view_proj_inv * far_ndc;
    if far_world.w.abs() > 1e-6 {
        far_world /= far_world.w;
    }

    let origin = near_world.truncate();
    let direction = (far_world.truncate() - origin).normalize();

    // Solve ray-plane intersection with ground plane (y = 0)
    let denominator = direction.y;
    if denominator.abs() <= 1e-6 {
        return; // Early exit: ray is parallel to plane
    }

    let t = -origin.y / denominator;
    if t < 0.0 {
        return; // Early exit: intersection is behind the camera
    }

    let hit_point = origin + direction * t;
    let hit_vec3 = Vec3::new(hit_point.x, hit_point.y, hit_point.z);

    // Draw a bright glowing green/teal 3D circle at the cursor hit point
    let segments = 64;
    let color = [0.0, 1.0, 0.5, 1.0]; // Bright neon green/teal
    for r_offset in [-0.015, 0.0, 0.015] {
        let radius = 0.3 + r_offset;
        for j in 0..segments {
            let a1 = j as f32 * 2.0 * std::f32::consts::PI / segments as f32;
            let a2 = (j + 1) as f32 * 2.0 * std::f32::consts::PI / segments as f32;
            let start = Vec3::new(hit_vec3.x + radius * a1.cos(), 0.1, hit_vec3.z + radius * a1.sin());
            let end = Vec3::new(hit_vec3.x + radius * a2.cos(), 0.1, hit_vec3.z + radius * a2.sin());
            gizmos.draw_line(start, end, color);
        }
    }
}
