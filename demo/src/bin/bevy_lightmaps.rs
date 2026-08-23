//! # Bevy'nin `3d/lightmaps` + `3d/mixed_lighting` örneklerinin Gizmo Engine portu
//!
//! Pişmiş aydınlatma: statik ışığın önceden hesaplanıp geometriye işlenmesi, ve gerçek zamanlı
//! ışıkla **karıştırılması**. Bevy'de bu bir `Lightmap` bileşeni + ayrı bir UV kanalı, ve
//! `mixed_lighting` örneği pişmiş ışığın üstüne gerçek zamanlı güneşi bindiriyor.
//!
//! ## Motorda karşılığı bir malzeme türü: `BakedLit`
//!
//! | Bevy | Gizmo |
//! |------|-------|
//! | `Lightmap { image, uv_rect }` bileşeni | **yok** |
//! | ayrı ışık haritası UV kanalı | **yok** — pişmiş ışık **köşe renginde** |
//! | ışık haritası dokusu | **yok** |
//! | pişmiş + gerçek zamanlı karışımı | **var** — `BakedLit` güneşin gölgesini de alıyor |
//! | pişirme aracı | **yok** — köşe renklerini siz dolduracaksınız |
//!
//! Yani "karıştırılmış aydınlatma" fikri motorda var, ama taşıyıcısı doku değil **köşe rengi**.
//! Bunun bedeli çözünürlük: bir duvarın ışık haritası dokuda piksel piksel olur, köşe renginde
//! ise mesh'in üçgen yoğunluğu kadar olur.
//!
//! ## Kazanç ölçülebilir: çizim sayısı
//!
//! `MaterialType`'ın kendi belgesi sayıyı veriyor: `BakedLit` "G-tamponunu ve nokta ışıklarını
//! atlıyor: yığın başına on bir yerine **bir** ileri çizim". Demo bunu ölçüyor — aynı sahne iki
//! kez, biri `Pbr` biri `BakedLit`.
//!
//! ## Ölçüldü
//!
//! Aynı sahne iki kez, tek değişen zeminin malzeme türü. Sahnede bir gerçek zamanlı güneş **ve**
//! mavi bir nokta ışığı var. Ölçüm sol yarı (panel sağda), ölçüldü (2026-08-23, 948×492):
//!
//! | | ortalama RGB | mavi − kırmızı |
//! |---|--------------|----------------|
//! | `Pbr` zemin | (127,8 · 127,8 · 129,1) | **+1,35** — mavimsi |
//! | `BakedLit` zemin | (108,8 · 106,7 · 104,0) | **−4,74** — sıcak |
//!
//! Renk dengesi **işaret değiştiriyor**, ve sebebi tam olarak beklenen: `Pbr` mavi nokta ışığını
//! görüyor, `BakedLit` **görmüyor** ve onun yerine köşe rengine pişmiş sıcak ışığı taşıyor.
//! Karelerin %48,98'i farklı.
//!
//! Ve zeminin parlaklık profili pişmiş ışığın gerçekten pişmiş olduğunu gösteriyor:
//!
//! | satır | `Pbr` | `BakedLit` |
//! |-------|-------|------------|
//! | 0,55 | 152,2 | 136,1 |
//! | 0,65 | 184,9 | **152,8** (tepe) |
//! | 0,75 | **192,3** (tepe) | 150,3 |
//! | 0,85 | 186,5 | 139,7 |
//! | 0,95 | 176,4 | **127,2** |
//!
//! İki eğrinin tepesi farklı yerde ve `BakedLit` daha dik düşüyor — çünkü onun düşüşü gerçek
//! zamanlı ışığın mesafe zayıflamasından değil, köşelere **yazılmış** sahte alan ışığından
//! geliyor. Bevy'de aynı bilgi bir dokuda olurdu; buradaki çözünürlük ızgaranın sıklığı kadar
//! (40×40).
//!
//! ### Ölçüm notu
//!
//! Pencere bu koşularda 948×492 açıldı, öteki demolarda 948×1028. İki koşunun boyutu birbiriyle
//! aynı olduğu için karşılaştırma geçerli, ama sabit piksel eşikleri demolar arasında
//! taşınamıyor — bölge her zaman görüntü boyutuna göre hesaplanıyor.
//!
//! ## Kontroller
//!   * `GIZMO_LIGHTMAP_MODE=pbr|baked` — malzeme türü
//!   * **Sağ-tık + fare / WASDQE** — kamera

use gizmo::prelude::*;
use gizmo::renderer::gpu_types::Vertex;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Hangi malzeme türüyle koşuyoruz.
fn baked_mode() -> bool {
    matches!(std::env::var("GIZMO_LIGHTMAP_MODE").as_deref(), Ok("baked"))
}

fn main() {
    let baked = baked_mode();
    App::<SimpleSceneState>::new("Gizmo Engine - Bevy Lightmaps", 1280, 720)
        .with_simple_scene(move |scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;

            // "Pişmiş" ışık: köşe renklerine elle yazılmış. Bevy'de bu bir doku olurdu.
            // Buradaki pişirme, sahte bir alan ışığından (tavandaki bir dikdörtgen) gelen
            // katkının köşe başına analitik hesabı.
            let baked_floor = Mesh::from_vertices(device, &baked_floor_vertices(), "lightmap::floor");

            let material_for = |mesh_is_baked: bool| {
                let base = Material::new(white.clone());
                if mesh_is_baked {
                    // `BakedLit`: köşe rengi × doku, artı güneşin gölgesi. Nokta ışıkları YOK.
                    base.with_baked_lit(Vec4::new(1.0, 1.0, 1.0, 1.0))
                } else {
                    base.with_pbr(Vec4::new(0.72, 0.70, 0.66, 1.0), 0.85, 0.0)
                }
            };

            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -0.6, 0.0)),
                GlobalTransform::default(),
                baked_floor,
                material_for(baked),
                MeshRenderer::new(),
            ));

            // Üstünde birkaç kutu: gölgeyi görünür kılıyorlar, ve `BakedLit` gölge ALDIĞI için
            // karışım burada okunuyor.
            let cube = AssetManager::create_cube(device);
            for i in 0..4 {
                scene.world.spawn_bundle((
                    Transform::new(Vec3::new((i as f32 - 1.5) * 2.0, 0.1, 0.5))
                        .with_scale(Vec3::new(0.5, 0.7, 0.5)),
                    GlobalTransform::default(),
                    cube.clone(),
                    Material::new(white.clone()).with_pbr(
                        Vec4::new(0.85, 0.45, 0.30, 1.0),
                        0.5,
                        0.0,
                    ),
                    MeshRenderer::new(),
                ));
            }

            // Gerçek zamanlı güneş — karışımın "gerçek zamanlı" yarısı.
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.4) * Quat::from_rotation_x(-0.8),
                intensity: 2.2,
                ..Default::default()
            });
            // Nokta ışığı: `Pbr` bunu görür, `BakedLit` GÖRMEZ. Farkın bir yarısı bu.
            scene.world.spawn_bundle(PointLightBundle {
                position: Vec3::new(0.0, 1.8, 2.4),
                color: Vec3::new(0.3, 0.6, 1.0),
                intensity: 8.0,
                radius: 8.0,
            });

            gizmo::gizmo_log!(
                Info,
                "zemin malzemesi: {}",
                if baked { "BakedLit" } else { "Pbr" }
            );
            let _ = white;
            scene.spawn_camera(state, Vec3::new(0.0, 3.4, 7.0), Vec3::new(0.0, 0.0, 0.0));
        })
        .set_ui(move |_world, _state, ctx| {
            gizmo::egui::Area::new("lm".into())
                .anchor(gizmo::egui::Align2::RIGHT_TOP, [-12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(420.0);
                        ui.heading("Pişmiş aydınlatma");
                        ui.label(format!(
                            "zemin: {}",
                            if baked { "BakedLit" } else { "Pbr" }
                        ));
                        ui.separator();
                        ui.label("pişmiş ışık KÖŞE RENGİNDE, dokuda değil.");
                        ui.label("Lightmap bileşeni, ayrı UV kanalı, pişirme aracı: YOK.");
                        ui.separator();
                        ui.label("BakedLit güneşin gölgesini ALIYOR (karışım),");
                        ui.label("ama nokta ışıklarını GÖRMÜYOR.");
                        ui.separator();
                        ui.label("çizim: yığın başına 11 yerine 1 (belge).");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Zemin: köşe renklerine "pişmiş" bir alan ışığı katkısı yazılmış bir ızgara.
///
/// Bevy'nin ışık haritası dokusunun yerine geçen şey bu — ve çözünürlüğün neden mesh'e bağlı
/// olduğunu da gösteriyor: katkı köşe başına, yani ızgara ne kadar sıksa o kadar yumuşak.
fn baked_floor_vertices() -> Vec<Vertex> {
    const N: usize = 40;
    const HALF: f32 = 7.0;
    // Tavandaki sahte alan ışığı — pişirmenin kaynağı.
    let panel_centre = Vec3::new(0.0, 3.0, 0.0);
    let panel_half = Vec2::new(2.2, 1.2);

    let bake = |x: f32, z: f32| -> [f32; 4] {
        // Panelin üstünde birkaç örnek al ve katkılarını topla — kaba bir alan ışığı.
        let mut sum = 0.0f32;
        const S: i32 = 3;
        for sx in -S..=S {
            for sz in -S..=S {
                let sample = panel_centre
                    + Vec3::new(
                        sx as f32 / S as f32 * panel_half.x,
                        0.0,
                        sz as f32 / S as f32 * panel_half.y,
                    );
                let to_light = sample - Vec3::new(x, 0.0, z);
                let dist = to_light.length().max(0.15);
                let cos = (to_light.y / dist).max(0.0);
                sum += cos / (dist * dist);
            }
        }
        let n = ((2 * S + 1) * (2 * S + 1)) as f32;
        let v = (sum / n * 6.0).clamp(0.0, 1.0);
        // Hafif sıcak bir renk — pişmiş ışığın kendi rengi.
        [0.10 + v * 0.95, 0.10 + v * 0.88, 0.10 + v * 0.72, 1.0]
    };

    let mut out = Vec::with_capacity(N * N * 6);
    let at = |i: usize, j: usize| -> Vertex {
        let x = -HALF + i as f32 / N as f32 * HALF * 2.0;
        let z = -HALF + j as f32 / N as f32 * HALF * 2.0;
        Vertex {
            position: [x, 0.0, z],
            normal: [0.0, 1.0, 0.0],
            tex_coords: [i as f32 / N as f32, j as f32 / N as f32],
            color: bake(x, z),
            ..Default::default()
        }
    };
    for i in 0..N {
        for j in 0..N {
            let (a, b, c, d) = (at(i, j), at(i, j + 1), at(i + 1, j + 1), at(i + 1, j));
            out.extend_from_slice(&[a, b, c, a, c, d]);
        }
    }
    out
}
