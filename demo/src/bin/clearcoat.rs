//! # Vernik katmanı
//!
//! Vernik katmanı: bir yüzeyin üstündeki ince, saydam, kendi parlaklığı olan tabaka — araba
//! boyasının, cilalı ahşabın, ıslak zeminin ortak özelliği. Altındaki malzeme mat olsa bile
//! üstündeki katman keskin bir yansıma verir.
//!
//! ## Motorda var, ve tam bağlı
//!
//! `Material::clear_coat` bir alan, ve yolun tamamı işliyor: değer `InstanceRaw`'un `pbr.w`'sine
//! paketleniyor, G-tamponunda çözülüyor, `deferred_lighting.wgsl` onu okuyup
//! `pbr_ext.wgsl`'deki `compute_clear_coat`'a veriyor.
//!
//! **Ama nasıl taşındığı dikkate değer.** G-tamponunda boş kanal kalmadığı için vernik değeri
//! **teğetin `w` kanalına** binmiş — o kanal normalde yalnız elliliği (handedness) taşıyor:
//!
//! ```text
//! packed_tangent_w = ellilik * (0,01 + 0,99 * vernik)
//! vernik           = clamp((|tangent.w| − 0,01) / 0,99, 0, 1)
//! ```
//!
//! İşaret elliliği, büyüklük verniği taşıyor. Bu yüzden vernik hiçbir zaman tam 0 olamıyor
//! (taban 0,01), çünkü 0 büyüklük elliliğin işaretini de yok ederdi. Ustaca, ama G-tamponu
//! bütçesinin ne kadar dar olduğunu da gösteriyor.
//!
//! ## Envanter: dört ayardan biri var
//!
//! | yetenek | Gizmo |
//! |---------|-------|
//! | vernik miktarı (0..1) | `Material::clear_coat` |
//! | vernik katmanının kendi pürüzü | **yok** — katmanın pürüzü sabit |
//! | vernik ve vernik pürüzü dokusu | **yok** |
//! | vernik normal dokusu (portakal kabuğu) | **yok** |
//!
//! Yani "ne kadar vernik" söylenebiliyor, "ne tür vernik" söylenemiyor.
//!
//! ## Ölçülen: vernik ikinci bir parlama lobu ekliyor
//!
//! Sahne, altı **mat** küre (pürüz 0,85) — yani kendi başlarına parlama vermeleri beklenmiyor —
//! ve soldan sağa artan vernik. Vernik çalışıyorsa parlak nokta ancak katmandan gelebilir.
//!
//! Ölçüt kürenin **en parlak pikseli**: parlama lobu küçük bir nokta olduğu için ortalamalar onu
//! göremiyor (yüzdelik ölçüt denendi, %2'lik dilim bile lobu seyreltiyordu).
//!
//! Ölçüldü (2026-08-23, kare 240, bloom kapalı):
//!
//! | vernik | en parlak piksel | gövde ortalaması |
//! |--------|------------------|------------------|
//! | 0,0 | **161** | 122,8 |
//! | 0,2 | **205** | 126,3 |
//! | 0,4 | 197 | 125,5 |
//! | 0,6 | 200 | 124,4 |
//! | 0,8 | 202 | 123,1 |
//! | 1,0 | 187 | 118,0 |
//!
//! Vernik **çalışıyor ve açık/kapalı farkı belirgin**: 0'dan 0,2'ye tepe 161 → 205, yani %27.
//! Mat bir yüzeyde (pürüz 0,85) o parlama başka hiçbir yerden gelemez.
//!
//! Ama **0,2'nin üstünde yanıt düzleşiyor**: 0,4–1,0 arası tepe 187–202 bandında salınıyor, tek
//! düze artmıyor. Yani düğme pratikte ikili gibi davranıyor — "vernik var" ile "vernik yok"
//! ayrılıyor, "ne kadar vernik" ayrılmıyor. Katmanın karakterini değiştiren düğme — vernik
//! katmanının kendi pürüzü — motorda **yok**; kalan tek sayı da bu aralıkta az iş görüyor.
//!
//! Paketlemenin bir sonucu da ölçümde görünüyor: vernik 0 iken `tangent.w = ±0,01` ve çözülmüş
//! değer tam 0 — yani "kapalı" gerçekten kapalı, taban 0,01 sızmıyor.
//!
//! **Ölçüm notu:** küreler kırmızılıklarıyla ayrıştırıldı (`R − B > 18`), ve parlama beyazladıkça
//! o maske küçülüyor (10 025 pikselden 9 092'ye). Tepe ölçümünü etkilemiyor ama gövde
//! ortalamasının hafif düşüşünün bir kısmı bundan.
//!
//! ## Kontroller
//!   * **Sağ-tık + fare / WASDQE** — kamera

use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Kürelerin vernik değerleri — soldan sağa.
const COATS: [f32; 6] = [0.0, 0.2, 0.4, 0.6, 0.8, 1.0];
/// Altlarındaki malzemenin pürüzü: bilerek mat, ki parlama ancak vernikten gelsin.
const BASE_ROUGHNESS: f32 = 0.85;
/// Küreler arasındaki aralık.
const SPACING: f32 = 2.6;

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Clearcoat", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;
            let sphere = AssetManager::create_sphere(device, 1.0, 28, 44);

            for (i, coat) in COATS.into_iter().enumerate() {
                let x = (i as f32 - 2.5) * SPACING;
                // Tek değişen şey vernik: albedo, pürüz, metaliklik ve geometri özdeş.
                let mut material = Material::new(white.clone()).with_pbr(
                    Vec4::new(0.35, 0.12, 0.12, 1.0),
                    BASE_ROUGHNESS,
                    0.0,
                );
                material.clear_coat = coat;
                scene.world.spawn_bundle((
                    Transform::new(Vec3::new(x, 0.0, 0.0)),
                    GlobalTransform::default(),
                    sphere.clone(),
                    material,
                    MeshRenderer::new(),
                ));
            }

            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -1.6, 0.0)),
                GlobalTransform::default(),
                AssetManager::create_plane(device, 30.0),
                Material::new(white).with_pbr(Vec4::new(0.09, 0.09, 0.10, 1.0), 1.0, 0.0),
                MeshRenderer::new(),
            ));

            // Tek, sert bir ışık: parlama lobu ancak böyle okunur.
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.25) * Quat::from_rotation_x(-0.45),
                intensity: 3.2,
                ..Default::default()
            });

            scene.spawn_camera(state, Vec3::new(0.0, 1.4, 16.0), Vec3::new(0.0, 0.0, 0.0));
        })
        .set_render(|world, _state, encoder, view, renderer, _light_time| {
            renderer.gpu_physics = None;
            renderer.gpu_fluid = None;
            renderer.gpu_particles = None;
            renderer.ssr = None;
            renderer.ssgi = None;
            // Bloom kapalı: ölçülen şey parlama lobu, lob + hale değil.
            renderer.bloom_intensity = 0.0;

            gizmo::systems::default_render_pass(world, encoder, view, renderer);
        })
        .set_ui(|_world, _state, ctx| {
            gizmo::egui::Area::new("coat".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(450.0);
                        ui.heading("Vernik katmanı");
                        ui.label("soldan sağa vernik: 0,0 · 0,2 · 0,4 · 0,6 · 0,8 · 1,0");
                        ui.label(format!(
                            "altındaki malzeme her kürede aynı: pürüz {BASE_ROUGHNESS:.2}, metaliklik 0"
                        ));
                        ui.separator();
                        ui.label("vernik G-tamponunda TEĞETİN w KANALINA biniyor:");
                        ui.label("  packed_tangent_w = ellilik * (0,01 + 0,99 * vernik)");
                        ui.label("  işaret elliliği, büyüklük verniği taşıyor");
                        ui.separator();
                        ui.label("YOK: vernik pürüzü ve üç doku");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}
