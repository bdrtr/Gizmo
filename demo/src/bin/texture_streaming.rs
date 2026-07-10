//! Doku Akışı (Texture Streaming) görsel doğrulama demosu.
//!
//! Sahnede uzağa doğru dizilmiş kutular BAŞTA beyaz (placeholder) — her birinin
//! `Material.texture_source`'u dolu ama dokusu henüz yüklenmemiş. Kamera bir kutuya
//! ~50 m yaklaşınca [`TextureStreamingSystem`] o kutunun dokusunu arka planda decode
//! edip GPU'ya yükler ve materyaline uygular: kutu BEYAZ → DOKULU olur ("pop-in").
//!
//! Kontroller: sağ-fare bak, W/S/A/D/E/Q uç, Shift hızlı. İleri (W) uçup kutuların
//! sırayla dokulanmasını izle. Uzaklaşıp tekrar yaklaşınca da yeniden yüklenir.
//!
//! Çalıştır: `cargo run -p demo --bin texture_streaming` (CWD = workspace kökü olmalı;
//! doku yolları `demo/assets/...` oradan çözülür).

use gizmo::math::{Vec3, Vec4};
use gizmo::physics::components::GlobalTransform;
use gizmo::physics::Transform;
use gizmo::prelude::*;
use gizmo::renderer::asset::AssetManager;
use gizmo::renderer::components::{Material, MeshRenderer};
use gizmo::simple::{SceneBuilder, SimpleAppExt, SimpleSceneState};

/// Uzağa doğru dizilecek kutular: (z uzaklığı, x kayması, doku yolu).
const CUBES: &[(f32, f32, &str)] = &[
    (-40.0, -6.0, "assets/brick.jpg"),
    (-55.0, 6.0, "demo/assets/grass.jpg"),
    (-70.0, -6.0, "demo/assets/textures/rusty_metal.jpg"),
    (-85.0, 6.0, "demo/assets/textures/tire_tread.jpg"),
    (-100.0, -6.0, "demo/assets/textures/dirt_grass.jpg"),
    (-115.0, 6.0, "demo/assets/domino_real.png"),
];

/// Statik, dokulu-ama-henüz-yüklenmemiş bir kutu: beyaz placeholder bind_group +
/// `texture_source` dolu. Fizik (rigidbody) YOK → düşmez, kamera etrafında uçar.
fn spawn_streaming_cube(scene: &mut SceneBuilder, pos: Vec3, size: f32, tex_path: &str) {
    let mesh = AssetManager::create_cube(&scene.renderer.device);
    let white = scene.asset_manager.create_white_texture(
        &scene.renderer.device,
        &scene.renderer.queue,
        &scene.renderer.scene.texture_bind_group_layout,
    );
    let mat = Material::new(white)
        .with_pbr(Vec4::new(1.0, 1.0, 1.0, 1.0), 0.8, 0.0)
        .with_texture_source(tex_path.to_string());

    let ent = scene.world.spawn();
    scene.world.add_component(
        ent,
        Transform::new(pos).with_scale(Vec3::splat(size / 2.0)),
    );
    scene.world.add_component(ent, GlobalTransform::default());
    scene.world.add_component(ent, mesh);
    scene.world.add_component(ent, mat);
    scene.world.add_component(ent, MeshRenderer::new());
}

fn main() {
    println!("Doku Akışı Demosu — W ile ileri uç; kutulara ~50m yaklaşınca");
    println!("dokuları yüklenir (BEYAZ → DOKULU). Sağ-fare: bak, Shift: hızlı.");

    gizmo::app::App::<SimpleSceneState>::new("Gizmo — Texture Streaming", 1600, 900)
        // Streaming sistemlerini (request + apply) kaydeder.
        .add_plugin(gizmo::asset_server::AssetServerPlugin)
        .with_simple_scene(|scene, state| {
            // Kamera başta TÜM kutulardan >50m uzakta (hepsi beyaz başlar).
            state.camera_pos = Vec3::new(0.0, 3.0, 10.0);
            state.camera_speed = 18.0;

            scene.spawn_ground(70.0);

            let light_ent = scene.world.spawn();
            let bundle = DirectionalLightBundle {
                rotation: Quat::from_rotation_x(-std::f32::consts::FRAC_PI_4)
                    * Quat::from_rotation_y(std::f32::consts::FRAC_PI_4),
                intensity: 2.2,
                ..Default::default()
            };
            bundle.apply(scene.world, light_ent);

            for &(z, x, tex) in CUBES {
                spawn_streaming_cube(scene, Vec3::new(x, 3.0, z), 5.0, tex);
            }

            scene.spawn_camera(state, state.camera_pos, Vec3::new(0.0, 3.0, -40.0));
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}
