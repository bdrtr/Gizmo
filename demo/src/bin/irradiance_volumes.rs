//! # Işınım hacimleri
//!
//! Işınım hacimleri: sahneye bir **prob ızgarası** serilir, her probda çevreden gelen ışık
//! küresel harmoniklerle saklanır, ve nesneler aralarındaki değerden **karıştırarak** okur.
//! Statik dolaylı aydınlatmanın klasik yolu.
//!
//! ## Motorda matematiğin tamamı vardı — ve **2026-08-24'te render'a bağlandı**
//!
//! Bu demonun asıl bulgusu buydu. `gizmo_renderer::gi` içinde tam bir altyapı duruyordu:
//! [`SHCoeffs`] (L0+L1, `add_directional_light` / `evaluate` / `lerp` / `to_gpu_data`),
//! [`LightProbe`], ve trilineer örneklemeli [`ProbeGrid`] — üstelik yedi testi de var.
//!
//! Ama render yolunda **hiçbir yerde kullanılmıyor**. Ölçüldü (2026-08-23):
//!
//! | arama | sonuç |
//! |-------|-------|
//! | `ProbeGrid` geçen dosyalar (gi.rs dışında) | yalnız `lib.rs`'teki `pub use` |
//! | `to_gpu_data` çağrıları | **yalnız kendi testi** |
//! | SH katsayısı okuyan shader | **yok** |
//! | `ProbeGrid` kullanan demo | (bu demodan önce) **yok** |
//!
//! Yani özellik yazılmış, test edilmiş, dışa açılmış — ve **fişi takılmamıştı**. Bir oyun bunu
//! kullanmak isterse örneklemeyi kendi yapıp sonucu kendi uygulamak zorundaydı.
//!
//! **Fiş takıldı.** `IrradianceState` ızgarayı GPU'ya yüklüyor, `irradiance.wgsl`
//! `ProbeGrid::sample` + `SHCoeffs::evaluate`'in birebir portu, ve ertelenmiş aydınlatma geçişi
//! grup 3'ten okuyup dolaylı ışığı albedoya çarpıyor. Demo iki yolu da koşturuyor:
//! `GIZMO_IRR_GPU=1` ile GPU, onsuz eski CPU yolu.
//!
//! ## Ölçülen 0: iki yol aynı karışım eğrisini veriyor
//!
//! Aynı ızgara, aynı sahne, tek değişen uygulama yolu. Yedi kürenin merkezinden okunan
//! **kırmızı − mavi** farkı (2026-08-24, kare 200):
//!
//! | küre | CPU (K−M) | GPU (K−M) |
//! |------|-----------|-----------|
//! | −3 | +17,6 | +46,9 |
//! | −2 | +13,5 | +38,5 |
//! | −1 | +4,0 | +20,9 |
//! | +0 | −9,0 | −8,1 |
//! | +1 | −25,4 | −41,3 |
//! | +2 | −43,6 | −70,6 |
//! | +3 | −48,9 | −79,7 |
//!
//! | | |
//! |---|---|
//! | ikisi de tekdüze düşüyor | **evet** |
//! | korelasyon | **0,99887** |
//! | ölçek oranı (GPU/CPU) | 1,920 |
//!
//! Aynı eğri, farklı ölçek. Ölçek farkı bir hata değil, iki yolun **nereye** uyguladığından:
//! CPU yolu ışınımı albedoya işliyor (0,18 taban + ışınım, sonra doğrudan ışıkla çarpılıyor), GPU
//! yolu küreyi beyaz bırakıp dolaylı terimi ayrıca ekliyor. Karışımın *şekli* — probların
//! aralarındaki geçiş — ikisinde de aynı, ve ölçülen şey oydu.
//!
//! ## Yetenek dökümü
//!
//! | yetenek | Gizmo |
//! |---------|-------|
//! | render'a bağlı ışınım hacmi | **var** (2026-08-24) — `DeferredState::irradiance` |
//! | GPU'da SH örneklemesi | **var** — `irradiance.wgsl`, grup 3 |
//! | ışınım hacmi **bileşeni** (varlığa takılan) | **yok** — ızgara elle yükleniyor |
//! | voxel dokusundan örnekleme | **yok** — storage buffer, doku değil |
//! | ışık probu bileşeni + otomatik seçim | **yok** |
//! | proplar arası karışım | **var** — `ProbeGrid::sample` trilineer |
//! | katsayı karışımı | **var** — `SHCoeffs::lerp` |
//! | gerçek ışın izlemeli pişirme | **yok** — analitik izdüşüm ("simple baking") |
//!
//! ## Ölçüldü
//!
//! 4×3×4 = **48** prob, iki renkli nokta ışığı (solda kırmızı, sağda mavi) ve zayıf bir güneşten
//! pişirilmiş. Yedi küre kendi konumundaki ışınımı okuyor. Ölçüldü (2026-08-23):
//!
//! | küre | okunan ışınım (K, Y, M) |
//! |------|-------------------------|
//! | x = −4,2 | **(0,935 · 0,462 · 0,333)** — kırmızı baskın |
//! | x = −2,8 | (0,702 · 0,377 · 0,279) |
//! | x = −1,4 | (0,359 · 0,253 · 0,198) |
//! | x = 0,0 | (0,273 · 0,245 · 0,239) — neredeyse nötr |
//! | x = +1,4 | (0,232 · 0,268 · 0,326) |
//! | x = +2,8 | (0,312 · 0,426 · 0,669) |
//! | x = +4,2 | **(0,366 · 0,533 · 0,902)** — mavi baskın |
//!
//! Yani trilineer örnekleme çalışıyor: iki ışığın etkisi ortada düzgünce karışıyor.
//! `ProbeGrid::sample` + `SHCoeffs::evaluate` prob karışımının işini yapıyor — **CPU'da**.
//!
//! ### Ölçüm notu: ilk ölçüt yanlıştı, karışım değil
//!
//! Karışımı sınamak için önce "kırmızı kanal soldan sağa düşsün, mavi yükselsin" dedim ve sonuç
//! `false` çıktı. Karışım bozuk değildi, **ölçüt** yanlıştı: mavi ışığın rengi (0,20 · 0,45 ·
//! 1,00) kırmızı da içeriyor, o yüzden mavi proba yaklaşınca kırmızı kanal haklı olarak yeniden
//! yükseliyor (x = +0,88'de 0,238 dip, x = +3,50'de 0,352'ye çıkıyor). İki ışığın **farkı**
//! (kırmızı − mavi) ise baştan sona tekdüze düşüyor — ve karışımı ölçen büyüklük o.
//!
//! Bir de yükseklik: ilk örnekleme ışıklarla tam aynı hizadaydı (y = 1,6) ve orada
//! `evaluate(Vec3::Y)` fiziksel olarak anlamsız (ışık yandan geliyor, yukarı bakan normale düşen
//! L1 terimi ters işaretli). Örnekleme kürelerin hizasına (y = 0,6) çekildi.
//!
//! ## Kontroller
//!   * `GIZMO_IRR_GPU=1` — ışınımı GPU'da uygula (kapalıyken CPU yolu)
//!   * **Sağ-tık + fare / WASDQE** — kamera

use gizmo::prelude::*;
use gizmo::renderer::{ProbeGrid, SHCoeffs};
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Işınımı örneklenmiş nesne.
#[derive(Clone, Copy)]
struct Probed;
gizmo::core::impl_component!(Probed);

/// Ölçüm defteri.
#[derive(Default, Clone)]
struct GiReport {
    probe_count: usize,
    resolution: [u32; 3],
    /// Beş örnekleme noktası: konum ve okunan ışınım.
    samples: Vec<(Vec3, Vec3)>,
    /// İki prob arasında yürürken okunan değerin gerçekten karıştığının kanıtı.
    blend_check: Vec<(f32, Vec3)>,
    /// Karışım gerçekten tekdüze mi ilerliyor.
    monotonic: bool,
}
gizmo::core::impl_component!(GiReport);

const RES: [u32; 3] = [4, 3, 4];

/// Işınımı GPU'da mı uygulayalım. `GIZMO_IRR_GPU=1`.
fn gpu_path() -> bool {
    std::env::var("GIZMO_IRR_GPU").is_ok()
}

fn main() {
    let gpu_path = gpu_path();
    App::<SimpleSceneState>::new("Gizmo Engine - Irradiance Volumes", 1280, 720)
        .with_simple_scene(move |scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;
            let sphere = AssetManager::create_sphere(device, 1.0, 20, 28);

            // Sahnenin ışıkları — hem gerçek ışık olarak hem de pişirmenin girdisi olarak.
            let sun_dir = Vec3::new(0.3, -1.0, 0.2).normalize();
            let sun_colour = Vec3::new(1.0, 0.95, 0.85);
            let point_lights: [(Vec3, Vec3, f32, f32); 2] = [
                (Vec3::new(-3.5, 1.6, 0.0), Vec3::new(1.0, 0.35, 0.20), 6.0, 7.0),
                (Vec3::new(3.5, 1.6, 0.0), Vec3::new(0.20, 0.45, 1.0), 6.0, 7.0),
            ];

            // Izgarayı kur ve pişir. Bu satırlar motorun kendi API'si — ama render bunları
            // hiç görmüyor.
            let mut grid = ProbeGrid::new(
                Vec3::new(-5.0, -0.5, -5.0),
                Vec3::new(5.0, 3.5, 5.0),
                RES,
            );
            grid.bake_from_lights(
                &[(sun_dir, sun_colour, 0.35)],
                &point_lights,
                Vec3::splat(0.03),
            );

            // 2026-08-24: ızgara artık GPU'ya yüklenebiliyor. Ertelenmiş aydınlatma geçişi
            // grup 3'ten okuyup dolaylı ışığı albedoya çarpıyor — yani bu demonun CPU'da yaptığı
            // işin aynısını, boru hattının içinde.
            if gpu_path {
                let (device, queue) = (&scene.renderer.device, &scene.renderer.queue);
                if let Some(def) = scene.renderer.deferred.as_mut() {
                    def.irradiance.upload(device, queue, &grid);
                    gizmo::gizmo_log!(
                        Info,
                        "prob ızgarası GPU'ya yüklendi: {} prob",
                        def.irradiance.probe_count
                    );
                }
            }

            // Sahne: bir sıra küre. Her birinin rengi kendi konumundaki ışınımdan geliyor.
            let mut samples = Vec::new();
            for i in 0..7 {
                let position = Vec3::new((i as f32 - 3.0) * 1.4, 0.6, 0.0);
                // SH'yi yukarı bakan normal için değerlendir — "bu noktada tepeden ne kadar
                // ışık geliyor" sorusunun cevabı.
                let irradiance = grid.sample(position).evaluate(Vec3::Y).max(Vec3::ZERO);
                samples.push((position, irradiance));

                scene.world.spawn_bundle((
                    Transform::new(position).with_scale(Vec3::splat(0.55)),
                    GlobalTransform::default(),
                    sphere.clone(),
                    // `GIZMO_IRR_GPU=1` ile uygulama GPU'da: küre beyaz kalıyor ve ışınımı
                    // ertelenmiş aydınlatma geçişi prob ızgarasından okuyup ekliyor. Kapalıyken
                    // eski yol — örneklenen ışınım doğrudan albedoya işleniyor, çünkü boru hattı
                    // SH bilmiyordu.
                    Material::new(white.clone()).with_pbr(
                        if gpu_path {
                            Vec4::new(1.0, 1.0, 1.0, 1.0)
                        } else {
                            Vec4::new(
                                (0.18 + irradiance.x).min(1.0),
                                (0.18 + irradiance.y).min(1.0),
                                (0.18 + irradiance.z).min(1.0),
                                1.0,
                            )
                        },
                        0.55,
                        0.0,
                    ),
                    MeshRenderer::new(),
                    Probed,
                ));
            }

            // Karışım sınaması: iki prob arasında adım adım yürü, okunan değer düzgün mü.
            //
            // Yükseklik kürelerinkiyle AYNI (y = 0,6), ışıklarınkiyle değil. İlk denemede
            // y = 1,6'daydı — ışıklarla tam aynı hizada — ve orada `evaluate(Vec3::Y)` fiziksel
            // olarak anlamsız: ışık yandan geliyor, yukarı bakan normale düşen L1 terimi ters
            // işaretli çıkıyor ve okuma tersine dönüyordu.
            let mut blend_check = Vec::new();
            for step in 0..=8 {
                let t = step as f32 / 8.0;
                let x = -3.5 + t * 7.0; // kırmızı proba yakından mavi proba yakına
                blend_check.push((
                    x,
                    grid.sample(Vec3::new(x, 0.6, 0.0)).evaluate(Vec3::Y).max(Vec3::ZERO),
                ));
            }
            // Ölçüt **kırmızı − mavi**, tek tek kanallar değil.
            //
            // İlk denemede ölçüt "kırmızı düşsün, mavi yükselsin"di ve `false` verdi. Sebep
            // karışımın bozuk olması değil, ölçütün yanlış olmasıydı: mavi ışığın rengi
            // (0,20 0,45 1,00) kırmızı da içeriyor, yani mavi proba yaklaşınca kırmızı kanal
            // haklı olarak yeniden yükseliyor (x=+0,88'de 0,238 dip, x=+3,50'de 0,352). İki
            // ışığın FARKI ise baştan sona tekdüze düşüyor, ve karışımı ölçen büyüklük o.
            let monotonic = blend_check
                .windows(2)
                .all(|w| (w[1].1.x - w[1].1.z) <= (w[0].1.x - w[0].1.z) + 1e-4);

            // Gerçek ışıklar da sahnede dursun ki CPU örneklemesiyle karşılaştırılabilsin.
            for (position, colour, intensity, radius) in point_lights {
                scene.world.spawn_bundle(PointLightBundle {
                    position,
                    color: colour,
                    intensity,
                    radius,
                });
            }
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.3) * Quat::from_rotation_x(-0.9),
                intensity: 1.2,
                ..Default::default()
            });
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -0.5, 0.0)),
                GlobalTransform::default(),
                AssetManager::create_plane(device, 24.0),
                Material::new(white).with_pbr(Vec4::new(0.16, 0.16, 0.18, 1.0), 0.9, 0.0),
                MeshRenderer::new(),
            ));

            gizmo::gizmo_log!(
                Info,
                "prob {} ({}x{}x{}) · karışım tekdüze: {} · uçlar {:?} -> {:?}",
                grid.probe_count(),
                RES[0],
                RES[1],
                RES[2],
                monotonic,
                blend_check.first().map(|(_, v)| *v),
                blend_check.last().map(|(_, v)| *v)
            );
            for (x, v) in &blend_check {
                gizmo::gizmo_log!(Info, "  karışım x={:+.2} -> ({:.4}, {:.4}, {:.4})", x, v.x, v.y, v.z);
            }
            for (position, irradiance) in &samples {
                gizmo::gizmo_log!(
                    Info,
                    "  x={:+.1} -> ışınım ({:.3}, {:.3}, {:.3})",
                    position.x,
                    irradiance.x,
                    irradiance.y,
                    irradiance.z
                );
            }

            scene.world.insert_resource(GiReport {
                probe_count: grid.probe_count(),
                resolution: RES,
                samples,
                blend_check,
                monotonic,
            });
            scene.spawn_camera(state, Vec3::new(0.0, 3.0, 8.5), Vec3::new(0.0, 0.5, 0.0));
        })
        .set_ui(|world, _state, ctx| {
            let Some(r) = world.get_resource::<GiReport>().map(|r| r.clone()) else {
                return;
            };
            gizmo::egui::Area::new("gi".into())
                .anchor(gizmo::egui::Align2::RIGHT_TOP, [-12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(430.0);
                        ui.heading("Işınım hacimleri");
                        ui.label(format!(
                            "{} prob ({}x{}x{})",
                            r.probe_count, r.resolution[0], r.resolution[1], r.resolution[2]
                        ));
                        ui.separator();
                        ui.label("örneklenen ışınım (yukarı normal):");
                        for (position, irradiance) in &r.samples {
                            ui.monospace(format!(
                                "  x={:+.1}  ({:.3} {:.3} {:.3})",
                                position.x, irradiance.x, irradiance.y, irradiance.z
                            ));
                        }
                        ui.separator();
                        ui.label(format!(
                            "proplar arası karışım tekdüze: {}",
                            if r.monotonic { "EVET" } else { "hayır" }
                        ));
                        for (x, v) in r.blend_check.iter().step_by(2) {
                            ui.monospace(format!(
                                "  x={:+.2}  kırmızı {:.3} · mavi {:.3}",
                                x, v.x, v.z
                            ));
                        }
                        ui.separator();
                        ui.colored_label(
                            gizmo::egui::Color32::from_rgb(230, 160, 80),
                            "matematik motorda, ama RENDER'A BAĞLI DEĞİL",
                        );
                        ui.label("hiçbir shader SH okumuyor; bu demo örneklemeyi");
                        ui.label("CPU'da yapıp albedoya kendisi işliyor.");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// `SHCoeffs::lerp`'in var olduğunu derleyiciye de gösteren küçük bir kullanım.
#[allow(dead_code)]
fn blend(a: &SHCoeffs, b: &SHCoeffs, t: f32) -> SHCoeffs {
    a.lerp(b, t)
}
