//! Zıplama (Restitution / Bounce) görsel doğrulama demosu.
//!
//! Aynı yükseklikten (y=12) DÖRT top bırakılır; hepsi aynı ama `restitution`'ları farklı:
//!   • KIRMIZI  e=0.0  — hiç zıplamaz (tamamen inelastik, "güm" durur).
//!   • TURUNCU  e=0.5  — orta zıplar.
//!   • YEŞİL    e=0.8  — yüksek zıplar.
//!   • MAVİ     e=0.95 — neredeyse elastik, defalarca yükseğe zıplar.
//!
//! Zıplama yükseklikleri ≈ e²·(düşüş yüksekliği) → belirgin farklı. `restitution_velocity_
//! threshold` (1 m/s) altında zıplama durur (jitter önlemi), böylece toplar sonunda oturur.
//!
//! Her top TEK `RigidBodyBundle` ile kurulur: kütle + atalet collider şeklinden OTOMATİK
//! türetilir (elle `calculate_sphere_inertia` yok), zıplaklık collider'ın `with_restitution`'ında.
//! Kamera / fizik-adımı / render `with_simple_scene`'den gelir (serbest-uçuş kamera + varsayılan pass).
//!
//! Kontroller: sağ-fare bak, W/S/A/D/E/Q uç, Shift hızlı.
//! Çalıştır: `cargo run -p demo --bin bounce`

use gizmo::prelude::*;
use gizmo::simple::{SceneBuilder, SimpleAppExt, SimpleSceneState};
use std::f32::consts::FRAC_PI_4;

/// y=12'den bırakılan, `e` restitution'lı bir top.
fn spawn_ball(scene: &mut SceneBuilder, x: f32, color: Vec3, e: f32) {
    const DROP_Y: f32 = 12.0;
    const R: f32 = 1.0;
    let mesh = AssetManager::create_sphere(&scene.renderer.device, R, 32, 32);
    let tex = scene.asset_manager.create_white_texture(
        &scene.renderer.device,
        &scene.renderer.queue,
        &scene.renderer.scene.texture_bind_group_layout,
    );
    let mat = Material::new(tex).with_pbr(Vec4::new(color.x, color.y, color.z, 1.0), 0.4, 0.1);

    // Küre gövde: kütle + atalet (collider şeklinden otomatik) + collider TEK bundle'da.
    // Zıplaklık collider'da (e); lineer sönüm 0 → zıplama enerjisi YALNIZ restitution'dan
    // gelsin (açısal sönüm 0.05 varsayılanda kalır).
    scene.world.spawn_bundle((
        Transform::new(Vec3::new(x, DROP_Y, 0.0)),
        mesh,
        mat,
        MeshRenderer::new(),
        RigidBodyBundle::dynamic(1.0)
            .with_collider(Collider::sphere(R).with_restitution(e))
            .with_damping(0.0, 0.05),
    ));
}

/// Geniş, DÜZ, STATİK zemin (toplar üstünde zıplasın).
fn spawn_ground(scene: &mut SceneBuilder) {
    let mesh = AssetManager::create_plane(&scene.renderer.device, 60.0);
    let tex = scene.asset_manager.create_white_texture(
        &scene.renderer.device,
        &scene.renderer.queue,
        &scene.renderer.scene.texture_bind_group_layout,
    );
    let mat = Material::new(tex).with_pbr(Vec4::new(0.18, 0.18, 0.20, 1.0), 0.9, 0.0);

    // Zemin mesh'i düz plane; collider'ı ise üst yüzeyi y=0'da duran ince statik OFFSET kutu.
    scene.world.spawn_bundle((
        Transform::new(Vec3::ZERO),
        mesh,
        mat,
        MeshRenderer::new(),
        RigidBodyBundle::static_body().with_collider(
            Collider::offset_box(Vec3::new(0.0, -0.5, 0.0), Vec3::new(30.0, 0.5, 30.0))
                .with_restitution(0.5),
        ),
    ));
}

fn main() {
    println!("Zıplama Demosu — 4 top aynı yükseklikten düşer, restitution'ları farklı:");
    println!("  KIRMIZI e=0.0 (zıplamaz)  TURUNCU e=0.5  YEŞİL e=0.8  MAVİ e=0.95 (en yüksek)");
    println!("Zıplama yükseklikleri ≈ e²·h → belirgin farklı. (Sağ-fare bak, WASD uç.)");

    App::<SimpleSceneState>::new("Gizmo — Zıplama (Restitution)", 1600, 900)
        .add_plugin(TransformPlugin)
        .add_plugin(PhysicsPlugin::default())
        .with_simple_scene(|scene, state| {
            state.camera_pos = Vec3::new(0.0, 8.0, 30.0);
            state.camera_speed = 20.0;

            spawn_ground(scene);

            // Güneş — sabit yön/şiddet; bundle DOĞRUDAN spawn'lanır (elle spawn()+apply() yok).
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_x(-FRAC_PI_4) * Quat::from_rotation_y(FRAC_PI_4),
                intensity: 2.2,
                ..Default::default()
            });

            spawn_ball(scene, -9.0, Vec3::new(0.9, 0.2, 0.2), 0.0);
            spawn_ball(scene, -3.0, Vec3::new(0.95, 0.55, 0.15), 0.5);
            spawn_ball(scene, 3.0, Vec3::new(0.25, 0.85, 0.3), 0.8);
            spawn_ball(scene, 9.0, Vec3::new(0.25, 0.5, 0.95), 0.95);

            scene.spawn_camera(state, state.camera_pos, Vec3::new(0.0, 4.0, 0.0));
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}
