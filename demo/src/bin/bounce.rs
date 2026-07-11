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
//! Kontroller: sağ-fare bak, W/S/A/D/E/Q uç, Shift hızlı.
//! Çalıştır: `cargo run -p demo --bin bounce`

use gizmo::math::{Vec3, Vec4};
use gizmo::physics::components::{Collider, GlobalTransform, RigidBody, Velocity};
use gizmo::physics::Transform;
use gizmo::plugins::{PhysicsPlugin, TransformPlugin};
use gizmo::prelude::*;
use gizmo::renderer::asset::AssetManager;
use gizmo::renderer::components::{Material, MeshRenderer};
use gizmo::simple::{SceneBuilder, SimpleAppExt, SimpleSceneState};

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

    let ent = scene.world.spawn();
    scene
        .world
        .add_component(ent, Transform::new(Vec3::new(x, DROP_Y, 0.0)));
    scene.world.add_component(ent, GlobalTransform::default());
    scene.world.add_component(ent, mesh);
    scene.world.add_component(ent, mat);
    scene.world.add_component(ent, MeshRenderer::new());

    let mut rb = RigidBody::new(1.0, true);
    rb.linear_damping = 0.0; // zıplama sönümü yalnız restitution'dan gelsin
    rb.calculate_sphere_inertia(R);
    scene.world.add_component(ent, rb);
    scene.world.add_component(ent, Velocity::default());
    // Temiz söz dizimi: elle PhysicsMaterial kurmak yerine tek satır zıplaklık.
    scene
        .world
        .add_component(ent, Collider::sphere(R).with_restitution(e));
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

    let ent = scene.world.spawn();
    scene.world.add_component(ent, Transform::new(Vec3::ZERO));
    scene.world.add_component(ent, GlobalTransform::default());
    scene.world.add_component(ent, mesh);
    scene.world.add_component(ent, mat);
    scene.world.add_component(ent, MeshRenderer::new());
    scene.world.add_component(ent, RigidBody::new_static());
    scene.world.add_component(ent, Velocity::default());
    // Üst yüzeyi y=0'da olan ince statik kutu.
    scene.world.add_component(
        ent,
        Collider::offset_box(Vec3::new(0.0, -0.5, 0.0), Vec3::new(30.0, 0.5, 30.0))
            .with_restitution(0.5),
    );
}

fn main() {
    println!("Zıplama Demosu — 4 top aynı yükseklikten düşer, restitution'ları farklı:");
    println!("  KIRMIZI e=0.0 (zıplamaz)  TURUNCU e=0.5  YEŞİL e=0.8  MAVİ e=0.95 (en yüksek)");
    println!("Zıplama yükseklikleri ≈ e²·h → belirgin farklı. (Sağ-fare bak, WASD uç.)");

    gizmo::app::App::<SimpleSceneState>::new("Gizmo — Zıplama (Restitution)", 1600, 900)
        .add_plugin(TransformPlugin)
        .add_plugin(PhysicsPlugin::default())
        .with_simple_scene(|scene, state| {
            state.camera_pos = Vec3::new(0.0, 8.0, 30.0);
            state.camera_speed = 20.0;

            spawn_ground(scene);

            let light_ent = scene.world.spawn();
            let bundle = DirectionalLightBundle {
                rotation: Quat::from_rotation_x(-std::f32::consts::FRAC_PI_4)
                    * Quat::from_rotation_y(std::f32::consts::FRAC_PI_4),
                intensity: 2.2,
                ..Default::default()
            };
            bundle.apply(scene.world, light_ent);

            spawn_ball(scene, -9.0, Vec3::new(0.9, 0.2, 0.2), 0.0);
            spawn_ball(scene, -3.0, Vec3::new(0.95, 0.55, 0.15), 0.5);
            spawn_ball(scene, 3.0, Vec3::new(0.25, 0.85, 0.3), 0.8);
            spawn_ball(scene, 9.0, Vec3::new(0.25, 0.5, 0.95), 0.95);

            scene.spawn_camera(state, state.camera_pos, Vec3::new(0.0, 4.0, 0.0));
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}
