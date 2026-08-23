//! # Uzaklığa göre ayrıntı düzeyi
//!
//! Nesne uzaklaştıkça daha kaba bir mesh'e geçmek. Yeteneğin tam hâli, her LOD kademesi için
//! başlangıç/bitiş mesafeleri ve aralarında çapraz geçiş demek (kademeler bir bantta birlikte,
//! alfa ile karışarak çizilir).
//!
//! ## Motorda LOD var, ama tek kademe ve tek eşik
//!
//! | yetenek | Gizmo |
//! |---------|-------|
//! | kademe sayısı | **bir tane**: `lod_vbufs[0]` |
//! | her kademeye ayrı mesafe | **tek kural**: `mesafe > yarıçap × 15` |
//! | çapraz geçiş (crossfade) | **yok** — anlık takas |
//! | mesafeyi sınırlayıcı kutudan (AABB) ölçme seçeneği | **yok** |
//! | nesne başına ayar | [`MeshRenderer::lod_bias`] — mesafeyi **bölen** bir çarpan |
//!
//! Kural `batching.rs`'de tek satır: `use_lod1 = dist_to_cam > world_r * 15.0`, ve `dist_to_cam`
//! önce [`effective_lod_distance`]'tan geçiyor — o da `mesafe / bias`. Yani `lod_bias = 2,0`
//! "yarı mesafede geç", `0,5` "iki katında geç" demek. 1,0 hiçbir şey yapmıyor.
//!
//! ## Asıl bulgu: motorun kendi ilkellerinin hiçbiri LOD alamıyor
//!
//! LOD üretimi `Mesh::new` içinde tek bir kapıya bağlı: **`if vertex_count > 20000`**. Altında
//! kalan hiçbir mesh LOD tamponu almıyor.
//!
//! Ve `new_indexed` — motorun bütün ilkellerinin geçtiği yol — önce **tekilleştirme** yapıyor.
//! Kodun kendi yorumu boyutu da söylüyor: *"en büyük primitif torus, 561 tekil vertex."* Yani
//! 20 000 eşiğinin yanından bile geçmiyorlar.
//!
//! Ölçüldü (2026-08-23):
//!
//! | mesh | köşe | `lod_vbufs` | LOD1 köşe |
//! |------|------|-------------|-----------|
//! | `create_sphere(1,0, 48, 72)` (ilkel, indeksli) | 3 575 | **0** | — |
//! | aynı küre, `from_vertices` (düz) | 20 736 | **1** | 10 368 |
//!
//! Aynı geometri, iki farklı kurulum yolu, taban tabana zıt sonuç. Ve ironi şurada: **daha iyi
//! olan yol (indeksli) LOD'u kaybettiren yol**, çünkü tekilleştirme köşe sayısını eşiğin altına
//! indiriyor. Yani bir oyunun LOD alması, mesh'i *verimsiz* kurmasına bağlı.
//!
//! (İlk yazımda bunun tersini varsaymıştım — "`from_vertices` meshopt'a uğramıyor, yani LOD'suz".
//! Ölçüm bunu çürüttü: `from_vertices` de `Mesh::new`'e gidiyor, ve tekilleştirmediği için eşiği
//! **aşan** taraf o oluyor.)
//!
//! ## Takas gerçekten oluyor mu — kamerayı sabit tutan deney
//!
//! Kamerayı uzaklaştırıp karşılaştırmak işe yaramıyor: küreler küçüldüğü için fark perspektiften
//! mi LOD'dan mı geldiği ayrılmıyor. Temiz deney `lod_bias`: kamera **14 m'de sabit**, yalnız
//! `bias` 2,0'den 0,5'e çevriliyor — yani etkin mesafe 7 m'den 28 m'ye, eşiğin (15 m) bir
//! yanından öbürüne geçiyor.
//!
//! | küre | ortalama fark | maks | farklı piksel |
//! |------|---------------|------|---------------|
//! | ilkel (LOD tamponu yok) | **0,000** | **0** | %0,000 |
//! | elle kurulan (LOD tamponu var) | 0,010 | **69** | %0,033 |
//!
//! Sol küre **bit bit aynı** — geçilecek bir LOD'u yok. Sağ küre değişiyor, yani takas
//! gerçekten oluyor. Ama farkın küçüklüğü de kayda değer: %50 sadeleştirme düzgün bir kürenin
//! siluetini neredeyse hiç bozmuyor, piksellerin yalnız binde üçü oynuyor.
//!
//! ## Kontroller
//!   * **Sağ-tık + fare / WASDQE** — kamera (ölçüm için dokunmayın)

use gizmo::core::query::{Query, With};
use gizmo::core::system::{IntoSystemConfig, Phase, Res, ResMut};
use gizmo::prelude::*;
use gizmo::renderer::gpu_types::Vertex;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// LOD'u ölçülen küre.
#[derive(Clone, Copy)]
struct Subject;
gizmo::core::impl_component!(Subject);

/// Ölçüm defteri.
#[derive(Default, Clone)]
struct LodReport {
    frame: u32,
    /// Motorun ilkeliyle kurulan mesh.
    primitive: String,
    /// Elle kurulan mesh.
    handmade: String,
    /// Kameranın uzaklığı ve hesaplanan eşik.
    distance: f32,
    threshold: f32,
}
gizmo::core::impl_component!(LodReport);

/// Kameranın uzaklığı — çevreden.
fn camera_distance() -> f32 {
    std::env::var("GIZMO_LOD_DIST")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10.0f32)
        .clamp(2.0, 400.0)
}

fn main() {
    let distance = camera_distance();

    App::<SimpleSceneState>::new("Gizmo Engine - Visibility Range", 1280, 720)
        .with_simple_scene(move |scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;

            // Motorun ilkeli: `new_indexed` üzerinden meshopt'a giriyor, yani LOD üretebiliyor.
            let primitive = AssetManager::create_sphere(device, 1.0, 48, 72);
            // Aynı geometri, ama `from_vertices` ile: meshopt'a hiç uğramıyor.
            let handmade = Mesh::from_vertices(
                device,
                &sphere_vertices(1.0, 48, 72),
                "visibility_range::handmade",
            );

            let report = LodReport {
                primitive: format!(
                    "lod_vbufs {} · köşe {} · lod köşeleri {:?}",
                    primitive.lod_vbufs.len(),
                    primitive.vertex_count,
                    primitive.lod_vertex_counts
                ),
                handmade: format!(
                    "lod_vbufs {} · köşe {} · lod köşeleri {:?}",
                    handmade.lod_vbufs.len(),
                    handmade.vertex_count,
                    handmade.lod_vertex_counts
                ),
                distance,
                ..Default::default()
            };
            gizmo::gizmo_log!(Info, "ilkel  : {}", report.primitive);
            gizmo::gizmo_log!(Info, "elle   : {}", report.handmade);

            for (x, mesh) in [(-1.4, &primitive), (1.4, &handmade)] {
                scene.world.spawn_bundle((
                    Transform::new(Vec3::new(x, 0.0, 0.0)),
                    GlobalTransform::default(),
                    mesh.clone(),
                    Material::new(white.clone()).with_pbr(
                        Vec4::new(0.75, 0.72, 0.68, 1.0),
                        0.45,
                        0.0,
                    ),
                    // `lod_bias` mesafeyi BÖLÜYOR: 2,0 "yarı mesafede geç", 0,5 "iki katında".
                    // Kamerayı sabit tutup yalnız LOD'u çevirmenin yolu bu.
                    MeshRenderer::new().with_lod_bias(
                        std::env::var("GIZMO_LOD_BIAS")
                            .ok()
                            .and_then(|v| v.parse().ok())
                            .unwrap_or(1.0f32),
                    ),
                    Subject,
                ));
            }

            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.5) * Quat::from_rotation_x(-0.6),
                intensity: 2.6,
                ..Default::default()
            });
            scene.world.insert_resource(report);
            scene.spawn_camera(state, Vec3::new(0.0, 0.0, distance), Vec3::ZERO);
        })
        .add_update_system(measure.in_phase(Phase::Update))
        .set_ui(|world, _state, ctx| {
            let Some(r) = world.get_resource::<LodReport>().map(|r| r.clone()) else {
                return;
            };
            gizmo::egui::Area::new("lod".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(470.0);
                        ui.heading("Ayrıntı düzeyi");
                        ui.label(format!("kamera uzaklığı: {:.1} m", r.distance));
                        ui.label(format!("eşik (yarıçap × 15): {:.1} m", r.threshold));
                        ui.label(if r.distance > r.threshold {
                            "-> LOD1 (kaba mesh) seçilmeli"
                        } else {
                            "-> LOD0 (tam mesh) seçilmeli"
                        });
                        ui.separator();
                        ui.label(format!("solda  ilkel : {}", r.primitive));
                        ui.label(format!("sağda  elle  : {}", r.handmade));
                        ui.separator();
                        ui.label("tek kademe, tek eşik, çapraz geçiş yok.");
                        ui.label("kademe sayısı ve kademe mesafeleri ayarlanamıyor,");
                        ui.label("kademeler bir bantta alfa ile karışarak geçmiyor.");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Eşiği kürenin kendi yarıçapından hesaplar ve günlüğe basar.
fn measure(
    subjects: Query<(&Transform, With<Subject>)>,
    mut report: ResMut<LodReport>,
    time: Res<Time>,
) {
    let _ = time;
    report.frame += 1;
    // Küre yarıçapı 1, ölçek 1 → dünya yarıçapı 1. Kural `yarıçap × 15`.
    let radius = subjects
        .iter()
        .next()
        .map(|(_, (t, _))| t.scale.x.max(t.scale.y).max(t.scale.z))
        .unwrap_or(1.0);
    report.threshold = radius * 15.0;

    if report.frame == 120 {
        gizmo::gizmo_log!(
            Info,
            "uzaklık {:.1} m · eşik {:.1} m · beklenen: {}",
            report.distance,
            report.threshold,
            if report.distance > report.threshold { "LOD1" } else { "LOD0" }
        );
    }
}

/// Elle kurulmuş küre — `from_vertices` yolu, yani meshopt'a uğramıyor.
fn sphere_vertices(radius: f32, stacks: u32, slices: u32) -> Vec<Vertex> {
    use std::f32::consts::{PI, TAU};
    let mut out = Vec::new();
    let at = |i: u32, j: u32| {
        let phi = i as f32 / stacks as f32 * PI;
        let theta = j as f32 / slices as f32 * TAU;
        let n = Vec3::new(phi.sin() * theta.cos(), phi.cos(), phi.sin() * theta.sin());
        Vertex {
            position: [n.x * radius, n.y * radius, n.z * radius],
            normal: [n.x, n.y, n.z],
            tex_coords: [j as f32 / slices as f32, i as f32 / stacks as f32],
            ..Default::default()
        }
    };
    for i in 0..stacks {
        for j in 0..slices {
            let (a, b, c, d) = (at(i, j), at(i, j + 1), at(i + 1, j + 1), at(i + 1, j));
            out.extend_from_slice(&[a, b, c, a, c, d]);
        }
    }
    out
}
