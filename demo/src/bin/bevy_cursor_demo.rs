//! Bevy'nin kamera `viewport_to_world` ışın-izleme (raycast) imleç örneğinin Gizmo Engine karşılığı.
//! Basit düz bir yeşil zemin düzlemi, bir kamera ve bir yönlü ışık spawn'lar; imlecin
//! zeminle kesişim noktasına parlak neon yeşil bir 3B çember çizer.
//!
//! Yüksek seviye `SimpleAppExt` API + Bevy-tarzı ECS sistemi (`add_system`) ile yazıldı.
//! Zemin tek-seferlik bir nesne olduğundan `spawn_bundle` ile kurulur; ışın–düzlem
//! kesişimi imleç sisteminde ELLE hesaplanır (motorun collider'ından değil — demonun
//! ASIL amacı Bevy'nin `viewport_to_world` davranışını birebir taşımaktır).

use gizmo::core::input::Input;
use gizmo::core::query::Query;
use gizmo::core::system::{IntoSystemConfig, Phase, Res, ResMut};
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

fn main() {
    let mut app = gizmo::app::App::<SimpleSceneState>::new(
        "Gizmo Engine - Bevy Cursor Demo Parity",
        1280,
        720,
    );

    app = app
        .with_simple_scene(|scene, state| {
            // Geniş premium yeşil zemin düzlemi (srgb 0.07, 0.21, 0.07). Tek-seferlik nesne
            // olduğundan Prefab DEĞİL, doğrudan spawn_bundle. GlobalTransform açıkça verilir:
            // transform-propagate onu yalnız zaten sahip olan varlıklara yazar.
            let mesh = AssetManager::create_plane(&scene.renderer.device, 40.0);
            let tex = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let mat = Material::new(tex).with_pbr(Vec4::new(0.07, 0.21, 0.07, 1.0), 1.0, 0.0);

            scene.world.spawn_bundle((
                Transform::new(Vec3::ZERO),
                GlobalTransform::default(),
                mesh,
                mat,
                MeshRenderer::new(),
                RigidBodyBundle::static_body().with_collider(Collider::plane(Vec3::Y, 0.0)),
            ));

            // Yönlü ışık (Güneş)
            let sun_ent = scene.world.spawn();
            let sun_bundle = DirectionalLightBundle {
                rotation: Quat::from_rotation_x(-std::f32::consts::FRAC_PI_4),
                intensity: 0.8,
                color: Vec3::new(1.0, 1.0, 1.0),
                ..Default::default()
            };
            sun_bundle.apply(scene.world, sun_ent);

            // Kamera: Bevy örneğiyle eşleşmek için (0, 10, 15)'ten Vec3::ZERO'ya bakar
            scene.spawn_camera(state, Vec3::new(0.0, 10.0, 15.0), Vec3::ZERO);

            // Gizmos kaynağını derinlik testi KAPALI ekle → çizgiler titremeden/z-fighting'siz üstte çizilir
            let debug_gizmos = gizmo::renderer::Gizmos {
                depth_test: false,
                ..Default::default()
            };
            scene.world.insert_resource(debug_gizmos);
        })
        .add_system(draw_cursor.in_phase(Phase::Update));

    app.run().expect("uygulama çalıştırılamadı");
}

fn draw_cursor(
    mut gizmos: ResMut<gizmo::renderer::Gizmos>,
    win_info: Res<WindowInfo>,
    input: Res<Input>,
    q_cam: Query<(&Transform, &gizmo::renderer::components::Camera)>,
) {
    gizmos.depth_test = false; // Derinlik testini KAPALI zorla: çizgiler her zaman üstte çizilsin
    let (mouse_x, mouse_y) = input.mouse_position();

    // Fare pencere sınırları içinde mi?
    if mouse_x < 0.0 || mouse_y < 0.0 || mouse_x > win_info.width || mouse_y > win_info.height {
        return; // Fare pencere dışındaysa hiçbir şey çizme
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
        None => return, // Birincil kamera yoksa çık
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

    // Işın–düzlem kesişimini zemin düzlemiyle (y = 0) çöz
    let denominator = direction.y;
    if denominator.abs() <= 1e-6 {
        return; // Işın düzleme paralel → kesişim yok
    }

    let t = -origin.y / denominator;
    if t < 0.0 {
        return; // Kesişim kameranın arkasında
    }

    let hit_point = origin + direction * t;
    let hit_vec3 = Vec3::new(hit_point.x, hit_point.y, hit_point.z);

    // İmleç kesişim noktasına parlak neon yeşil/teal 3B çember çiz
    let segments = 64;
    let color = [0.0, 1.0, 0.5, 1.0]; // parlak neon yeşil/teal
    for r_offset in [-0.015, 0.0, 0.015] {
        let radius = 0.3 + r_offset;
        for j in 0..segments {
            let a1 = j as f32 * 2.0 * std::f32::consts::PI / segments as f32;
            let a2 = (j + 1) as f32 * 2.0 * std::f32::consts::PI / segments as f32;
            let start = Vec3::new(
                hit_vec3.x + radius * a1.cos(),
                0.1,
                hit_vec3.z + radius * a1.sin(),
            );
            let end = Vec3::new(
                hit_vec3.x + radius * a2.cos(),
                0.1,
                hit_vec3.z + radius * a2.sin(),
            );
            gizmos.draw_line(start, end, color);
        }
    }
}
