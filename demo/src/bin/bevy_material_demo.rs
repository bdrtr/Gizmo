//! # Bevy "material animation" örneğinin Gizmo portu — temiz sürüm
//!
//! 3×3'lük bir küp ızgarası spawn'lar; her küpün rengini (HSL tonu) zamanla akıcı biçimde
//! döndürür. Amaç Bevy'nin material animation örneğiyle görsel PARİTE.
//!
//! Bu sürüm motorun yüksek-seviye idiomlarını kullanır ama NEYİN uygun olduğu konusunda
//! dürüst kalır:
//!   * **`spawn_bundle`** — her küp TEK tuple-blueprint ile kurulur (eski `spawn()` +
//!     art arda 6 `add_component` anti-pattern'i gitti). Her küp KENDİ `Material`'ına sahip
//!     (animasyon sistemi her küpü ayrı mutasyona uğratır) + bir `CubeHue` işareti taşır.
//!   * **`DirectionalLightBundle`** — güneş de doğrudan `spawn_bundle` ile (elle `spawn()`+`apply`
//!     yok); bundle Transform+GlobalTransform+DirectionalLight'ı tek seferde ekler.
//!   * **Materyal animasyonu = idiomatik ECS sistemi** — `Query` + `Res<Time>` + `Mut`;
//!     `Phase::Update`'e kaydedilir. Elle döngü/state senkronu yok.
//!   * **Kamera = `scene.spawn_camera` yardımcısı**; kamera girişi/render'ı basit-sahne API'si yönetir.
//!
//! `Prefab`/`auto_box_collider`, `DespawnAfter`/`DespawnBelowY` ve `despawn_all_with` BİLEREK
//! KULLANILMADI: küplerin gövdesi/collider'ı YOK (saf görsel varlıklar — basit-sahne CPU-fizik
//! adımını çalıştırsa da gövdesiz entity'ler entegre edilmez, dolayısıyla yerlerinde dururlar).
//! O idiomlar fizik-gövdesi / geçici-uçan-nesne / sahne-reset ister; bu vitrinde bunların hiçbiri yok.

use gizmo::core::query::{Mut, Query};
use gizmo::core::system::{IntoSystemConfig, Phase, Res};
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Her küpün o anki tonunu (derece, 0.0..360.0) izleyen bileşen.
#[derive(Clone, Copy)]
struct CubeHue(f32);
gizmo::core::impl_component!(CubeHue);

// Bevy örneğiyle parite için altın açı adımı (ardışık küpler arası ton farkı).
const GOLDEN_ANGLE: f32 = 137.507_77;

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Bevy Material Animation Parity", 1280, 720)
        .with_simple_scene(|scene, state| {
            // Küplere bakan kamera (basit-sahne yardımcısı yaw/pitch'i look-at'tan türetir).
            scene.spawn_camera(state, Vec3::new(3.0, 2.0, 4.0), Vec3::new(0.0, -0.5, 0.0));

            // Yönlü güneş ışığı — bundle tek seferde Transform+GlobalTransform+DirectionalLight ekler.
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_x(-std::f32::consts::FRAC_PI_4),
                intensity: 0.8,
                color: Vec3::new(1.0, 1.0, 1.0),
                ..Default::default()
            });

            // Küplerin ortak mesh'i + beyaz doku (materyal albedo'yu taşır, animasyon onu değiştirir).
            let cube_mesh = AssetManager::create_cube(&scene.renderer.device);
            let white_texture = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );

            // Gizmo küpü varsayılan olarak 2.0 boyutunda; 0.25 ölçek → 0.5 boyut
            // (Bevy'nin Cuboid::new(0.5, 0.5, 0.5)'iyle eşleşir).
            let mut current_hue = 0.0;
            for x in -1..2 {
                for z in -1..2 {
                    let rgb = hsl_to_rgb(current_hue, 1.0, 0.5);
                    // Her küpe kendi materyali (animasyon sistemi her birini ayrı mutasyona uğratır).
                    let mat = Material::new(white_texture.clone()).with_pbr(
                        Vec4::new(rgb[0], rgb[1], rgb[2], 1.0),
                        0.1,
                        0.1,
                    );

                    let cube = scene.world.spawn_bundle((
                        Transform::new(Vec3::new(x as f32, 0.0, z as f32))
                            .with_scale(Vec3::splat(0.25)),
                        GlobalTransform::default(),
                        cube_mesh.clone(),
                        mat,
                        MeshRenderer::new(),
                    ));
                    // İzleme işareti bundle'dan ayrı (yikim'deki marker eklerken izlenen yol).
                    scene.world.add_component(cube, CubeHue(current_hue));

                    current_hue += GOLDEN_ANGLE;
                }
            }
        })
        .add_system(animate_materials.in_phase(Phase::Update))
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Dinamik HSL → RGB dönüşümü.
fn hsl_to_rgb(h: f32, s: f32, l: f32) -> [f32; 4] {
    let h = h.rem_euclid(360.0);
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
    let m = l - c / 2.0;

    let (r, g, b) = if h < 60.0 {
        (c, x, 0.0)
    } else if h < 120.0 {
        (x, c, 0.0)
    } else if h < 180.0 {
        (0.0, c, x)
    } else if h < 240.0 {
        (0.0, x, c)
    } else if h < 300.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };

    [r + m, g + m, b + m, 1.0]
}

/// Küp renklerini zamanla akıcı biçimde döndüren ECS sistemi (Bevy örneğindeki gibi 100°/sn).
fn animate_materials(mut q_materials: Query<(Mut<Material>, Mut<CubeHue>)>, time: Res<Time>) {
    let dt = time.dt();
    for (_entity, (mut mat, mut hue)) in q_materials.iter_mut() {
        hue.0 = (hue.0 + dt * 100.0).rem_euclid(360.0);
        let rgb = hsl_to_rgb(hue.0, 1.0, 0.5);
        mat.albedo = Vec4::new(rgb[0], rgb[1], rgb[2], 1.0);
    }
}
