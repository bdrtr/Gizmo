//! Hava Direnci (Aerodynamic Drag) görsel doğrulama demosu.
//!
//! Aynı yükseklikten (y=30) aynı anda ÜÇ top bırakılır:
//!   • KIRMIZI — hava direnci YOK (serbest düşüş, sürekli hızlanır, en hızlı iner).
//!   • YEŞİL   — orta drag (Cd 0.47, ~küre) → v_term ≈ 11.7 m/s'de sabitlenir.
//!   • MAVİ    — ağır drag (Cd 1.2, geniş alan, ~paraşüt) → v_term ≈ 3.0 m/s, yavaşça süzülür.
//!
//! Düşerken dikey olarak AYRIŞMALARI = `F = ½·ρ·Cd·A·v²` hava direncinin doğal etkisi.
//! Drag'li toplar sabit (terminal) hıza oturur; drag'siz top hızlanmaya devam eder.
//!
//! Her top TEK `RigidBodyBundle` ile kurulur: kütle, atalet (collider şeklinden OTOMATİK),
//! collider ve hava direnci hepsi tek zincirde (elle `RigidBody`/`Velocity`/`add_component`
//! dizisi yok). Kamera / fizik-adımı / render `with_simple_scene`'den gelir.
//!
//! Kontroller: sağ-fare bak, W/S/A/D/E/Q uç, Shift hızlı.
//! Çalıştır: `cargo run -p demo --bin air_drag`

use gizmo::prelude::*;
use gizmo::simple::{SceneBuilder, SimpleAppExt, SimpleSceneState};
use std::f32::consts::FRAC_PI_4;

/// y=DROP_Y'den bırakılan bir top. `drag = Some((Cd, alan))` ise fiziksel hava direnci
/// açık; `None` ise serbest düşüş. Fizik `PhysicsPlugin` tarafından sürülür.
fn spawn_ball(
    scene: &mut SceneBuilder,
    x: f32,
    radius: f32,
    color: Vec3,
    drag: Option<(f32, f32)>,
) {
    const DROP_Y: f32 = 30.0;
    let mesh = AssetManager::create_sphere(&scene.renderer.device, radius, 32, 32);
    let tex = scene.asset_manager.create_white_texture(
        &scene.renderer.device,
        &scene.renderer.queue,
        &scene.renderer.scene.texture_bind_group_layout,
    );
    let mat = Material::new(tex).with_pbr(Vec4::new(color.x, color.y, color.z, 1.0), 0.4, 0.1);

    // Küre gövde: kütle + atalet (collider şeklinden otomatik) + collider TEK bundle'da.
    // Lineer sönüm 0 → kaba proxy'yi kapat, yalnız GERÇEK v² drag görünsün (açısal sönüm
    // 0.05 varsayılanda kalır). Hava direnci varsa aynı zincire eklenir.
    let mut body = RigidBodyBundle::dynamic(2.0)
        .with_collider(Collider::sphere(radius))
        .with_damping(0.0, 0.05);
    if let Some((cd, area)) = drag {
        body = body.with_air_drag(cd, area);
    }

    scene.world.spawn_bundle((
        Transform::new(Vec3::new(x, DROP_Y, 0.0)),
        mesh,
        mat,
        MeshRenderer::new(),
        body,
    ));
}

fn main() {
    println!("Hava Direnci Demosu — 3 top aynı yükseklikten düşer:");
    println!("  KIRMIZI = drag YOK (serbest düşüş, en hızlı)");
    println!("  YEŞİL   = orta drag (terminal ~11.7 m/s)");
    println!("  MAVİ    = ağır drag / paraşüt (terminal ~3 m/s, yavaş süzülür)");
    println!("Düşerken ayrışmaları hava direncini gösterir. (Sağ-fare bak, WASD uç.)");

    App::<SimpleSceneState>::new("Gizmo — Hava Direnci", 1600, 900)
        .add_plugin(TransformPlugin)
        .add_plugin(PhysicsPlugin::default()) // fiziği sabit adımda otomatik sürer
        .with_simple_scene(|scene, state| {
            state.camera_pos = Vec3::new(24.0, 16.0, 24.0);
            state.camera_speed = 20.0;

            scene.spawn_ground(30.0);

            // Güneş — sabit yön/şiddet; bundle DOĞRUDAN spawn'lanır (elle spawn()+apply() yok).
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_x(-FRAC_PI_4) * Quat::from_rotation_y(FRAC_PI_4),
                intensity: 2.2,
                ..Default::default()
            });

            // Aynı kütle (2 kg), aynı yükseklik; yalnız drag farklı → ayrışma sırf hava direnci.
            spawn_ball(scene, -6.0, 1.0, Vec3::new(0.9, 0.2, 0.2), None); // serbest düşüş
            spawn_ball(scene, 0.0, 1.0, Vec3::new(0.2, 0.9, 0.3), Some((0.47, 0.5))); // orta
            spawn_ball(
                scene,
                6.0,
                1.0,
                Vec3::new(0.25, 0.5, 0.95),
                Some((1.2, 3.0)),
            ); // ağır/paraşüt

            scene.spawn_camera(state, state.camera_pos, Vec3::new(0.0, 12.0, 0.0));
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}
