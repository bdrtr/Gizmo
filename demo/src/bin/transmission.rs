//! # Işığın içinden geçtiği malzemeler
//!
//! Cam: arkasındaki sahneyi gösterir, rengiyle boyar, pürüzlüyse bulanıklaştırır, kalınsa kırar. Üstündeki
//! parlama ve yansıma yerinde kalır. Soldan sağa dört bölme, arkalarında kırmızı ve mavi kutular:
//!
//! | bölme | malzeme |
//! |-------|---------|
//! | berrak | `with_transmission(1.0)` |
//! | renkli | albedosu camgöbeği: kırmızı kutu kararır, mavi geçer |
//! | buzlu | `with_pbr(.., 0.45, 0.0)`: pürüzlülük arkadakini bulanıklaştırır |
//! | kalın | `with_thickness(0.6).with_ior(1.8)`: arkadaki görüntü kayar |
//!
//! En sağda yansıma: `with_ior(2.42)`, elmas, aynı kürede varsayılan 1.5'ten parlak.
//!
//! Her kare iki geçişte çizilir: önce opak dünya, sonra onun bir kopyası mip zinciriyle alınır, sonra cam.
//! Sahnede cam yoksa ne kopya ne doku vardır (`gizmo_renderer::transmission`). Ekran uzayında çalıştığı
//! için ekranda olmayan görünmez, bir cam başka bir camı göstermez, saydam yüzeyler ve parçacıklar
//! kopyadan sonra çizildiği için camın ardından görünmez.
//!
//! ## Kontroller
//!   * **Sağ-tık + fare / WASDQE** — kamera

use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Transmission", 1280, 720)
        .with_simple_scene(move |scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;

            // Camların arkasında, sırayla kırmızı ve mavi kutular.
            for i in 0..8 {
                let x = -6.3 + i as f32 * 1.8;
                let color = if i % 2 == 0 { Vec4::new(0.9, 0.1, 0.1, 1.0) } else { Vec4::new(0.1, 0.2, 0.9, 1.0) };
                scene.world.spawn_bundle((
                    Transform::new(Vec3::new(x, -0.4, -3.0)).with_scale(Vec3::splat(0.8)),
                    GlobalTransform::default(),
                    AssetManager::create_cube(device),
                    Material::new(white.clone()).with_pbr(color, 0.6, 0.0),
                    MeshRenderer::new(),
                ));
            }

            let clear_white = Vec4::new(1.0, 1.0, 1.0, 1.0);
            let panes = [
                (-4.8f32, Material::new(white.clone()).with_pbr(clear_white, 0.02, 0.0).with_transmission(1.0)),
                (-1.6, Material::new(white.clone()).with_pbr(Vec4::new(0.1, 0.9, 0.9, 1.0), 0.02, 0.0).with_transmission(1.0)),
                (1.6, Material::new(white.clone()).with_pbr(clear_white, 0.45, 0.0).with_transmission(1.0)),
                (
                    4.8,
                    Material::new(white.clone())
                        .with_pbr(clear_white, 0.02, 0.0)
                        .with_transmission(1.0)
                        .with_thickness(0.6)
                        .with_ior(1.8),
                ),
            ];
            for (x, material) in panes {
                scene.world.spawn_bundle((
                    Transform::new(Vec3::new(x, 0.4, 0.0)).with_scale(Vec3::new(1.4, 1.6, 0.05)),
                    GlobalTransform::default(),
                    AssetManager::create_cube(device),
                    material,
                    MeshRenderer::new(),
                ));
            }

            // Aynı koyu küre, iki kırılma indisiyle: elmas daha çok yansıtır.
            for (x, ior) in [(7.6f32, 1.5f32), (10.0, 2.42)] {
                scene.world.spawn_bundle((
                    Transform::new(Vec3::new(x, 0.0, 0.0)),
                    GlobalTransform::default(),
                    AssetManager::create_sphere(device, 1.0, 32, 44),
                    Material::new(white.clone()).with_pbr(Vec4::new(0.03, 0.03, 0.03, 1.0), 0.12, 0.0).with_ior(ior),
                    MeshRenderer::new(),
                ));
            }

            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -1.2, 0.0)),
                GlobalTransform::default(),
                AssetManager::create_plane(device, 40.0),
                Material::new(white.clone()).with_pbr(Vec4::new(0.30, 0.31, 0.34, 1.0), 0.85, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.5) * Quat::from_rotation_x(-0.55),
                intensity: 3.0,
                ..Default::default()
            });
            scene.spawn_camera(state, Vec3::new(1.5, 1.6, 11.0), Vec3::new(1.5, 0.2, 0.0));
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}
