//! # Motorun neyi ayıkladığı, neyi ayıklamadığı
//!
//! Çizilmeyecek geometriyi çizmemek. İki ayrı soru var ve motorun cevabı ikisinde farklı:
//!
//!   * **Görüş alanı dışında mı?** (frustum culling) — motorda **var**.
//!   * **Önünde bir şey mi duruyor?** (occlusion culling) — motorda **yok**.
//!
//! ## Var olan taraf
//!
//! [`visible_in_frustum`] bir AABB'yi kameranın frustum'una karşı sınıyor, ve `batching.rs` bunu
//! her nesne için çağırıyor. Gölge tarafında da çalışıyor: bir gölge düşürücü hiçbir kademenin
//! içine girmiyorsa gölge haritasına da girmiyor. Kendi test takımı var (`frustum_cull.rs`).
//!
//! ## Olmayan taraf
//!
//! Hiyerarşik derinlik tamponu (Hi-Z), sorgu tabanlı ayıklama, ya da geçen karenin derinliğini
//! kullanan bir görünürlük tahmini yok. Yani **tam olarak bir duvarın arkasında duran on bin küp,
//! görünmedikleri hâlde çizilir.**
//!
//! ## Ölçüldü — ve asıl bulgu beklediğim yerde değildi
//!
//! Dört yapılandırma, 60 000 küp, release, kare 620 (2026-08-23):
//!
//! | kip | frustum'u geçen | ort ms |
//! |-----|-----------------|--------|
//! | görünür (taban) | 60 000 | 18,54 |
//! | frustum dışında | **0** | **18,94** |
//! | yalnız duvar (0 varlık) | 1 | 5,82 |
//!
//! İkinci satır sürpriz: frustum sınaması **hiçbir** küpü geçirmiyor, ama kare süresi görünür
//! hâlden daha kısa değil. Yani ayıklama **çalışıyor ve hiçbir şey kazandırmıyor**.
//!
//! ## Maliyet nerede — dört kip onu ayrıştırıyor
//!
//! `yalnız dönüşüm` kipi küpleri doğuruyor ama `MeshRenderer` takmıyor: dönüşüm sistemleri onları
//! geziyor, yığınlayıcı hiç görmüyor. Aradaki farklar maliyeti üçe bölüyor (60 000 varlık):
//!
//! | | ms | fark | varlık başına |
//! |---|----|------|---------------|
//! | yalnız duvar (0 varlık) | 5,82 | taban | — |
//! | yalnız dönüşüm | 10,48 | **+4,66** dönüşüm sistemleri | 0,078 µs |
//! | frustum dışında (hepsi ayıklanmış) | 18,94 | **+8,46** yığınlayıcı gezinmesi | 0,141 µs |
//! | görünür (hepsi çiziliyor) | 18,54 | **−0,40** gerçek çizim | gürültü içinde |
//!
//! Son satır çarpıcı: **60 000 küpü çizmek, çizilip çizilmeyeceklerine karar vermenin yanında
//! ölçülemiyor.** Baskın terim yığınlayıcının nesne başına yürüyüşü, ve o yürüyüş ayıklama
//! kararından **önce** matris çarpımını, kamera kilidi düzeltmesini ve bir `Aabb::transform`'u
//! yapıyor — `continue` ancak ondan sonra geliyor.
//!
//! Ölçek de doğrusal. Hepsi ayıklanmışken, yani sıfır çizimle:
//!
//! | küp | ort ms |
//! |-----|--------|
//! | 15 000 | 6,43 |
//! | 30 000 | 11,20 |
//! | 60 000 | 18,78 |
//! | 120 000 | 38,34 |
//!
//! Eğim **varlık başına 0,30 µs**, taban 1,68 ms. Hiçbiri çizilmiyor.
//!
//! ## Ve tavanı kaldıracak şey motorda **zaten var**
//!
//! `gizmo_renderer::visibility` içinde tam bir uzamsal indeks duruyor: [`RenderAabbTree`] —
//! `insert` / `remove` / `retain` / `query_frustum` / `query_frusta` / `query_aabb`, yanında
//! `VisibleSet`, bir kıyaslama takımı, bağımsız bir doğrulama koşumu (`visibility_independent.rs`)
//! ve indeksli yolun doğrusal yolla varlık-varlık aynı sonucu verdiğini kanıtlayan
//! `differential.rs`.
//!
//! **Hiçbir çizim yolu onu çağırmıyor.** Doğrulandı (2026-08-23): `visibility/` dışında
//! `RenderAabbTree` geçen her yer ya `lib.rs`'teki `pub use` ve doküman örneği, ya kıyaslama, ya
//! da kendi testi. Her iki çizim yolu da (`batching.rs:424` ve stüdyonunki) `classify_visibility_world`'ü
//! önünde indeks olmadan, her mesh için doğrusal çağırıyor. Testin kendisi bunu satır 1411'de
//! ekrana basıyor.
//!
//! Modülün kendi ölçüm tablosu, yalnız CPU ayıklama süresi için:
//!
//! | mesh | doğrusal | indeksli | |
//! |------|----------|----------|---|
//! | 4 096 | 38 µs | 108 µs | doğrusal 2,8× kazanıyor |
//! | 8 192 | 77 µs | 170 µs | doğrusal 2,2× kazanıyor |
//! | 32 768 | 731 µs | 510 µs | **indeks 1,4× kazanıyor** |
//!
//! Yani dönüm noktası 8 k ile 32 k arasında bir bant — ve bu demonun ölçtüğü 60 000, bandın
//! üstünde. Ama abartmamak gerek: o tablo **yalnız ayıklama sınamasının** süresi (32 k'da 731 µs
//! = varlık başına 0,022 µs), oysa buradaki yığınlayıcı yürüyüşü varlık başına 0,141 µs. İndeks
//! sınamayı hızlandırmakla kalmaz, ayıklananlar için **yürüyüşün tamamını** atlatır — kazanç
//! oradan gelir, sınamanın kendisinden değil.
//!
//! Modülün kendi belgesi dürüst olan tarafı da söylüyor: *"Measure on your own scene before
//! believing any of the above"*, ve indeks bir **occlusion** yapısı değil, bunu iddia da etmiyor.
//!
//! ## Occlusion ayıklaması için sonuç
//!
//! Yok. Ama bu ölçüm, eklemenin **bu ölçekte tek başına işe yaramayacağını** söylüyor: occlusion
//! sınaması da aynı nesne-başına döngünün içine düşerdi, yani frustum sınamasının kazandırdığı
//! kadar kazandırırdı — ölçülene göre hiç. Sıra önce yürüyüşü kısaltmakta, ve onun aracı
//! zaten depoda.
//!
//! ### Ölçüm notu: ilk kurulumda ölçen şey ölçülen şeydi
//!
//! Frustum sayımını **her karede** yapıyordum ve o döngü 60 000 nesneyi geziyordu. Sonuç: dört
//! kipin dördü de birbirine yakın çıktı, çünkü baskın maliyet benim ölçüm aracımdı. Sayım 200
//! karede bire indirildi; yukarıdaki sayılar ondan sonrası. Aracın kendisi ölçüme karışıyorsa,
//! ölçtüğü fark da aracın farkı olur.
//!
//! ## Kontroller
//!   * `GIZMO_CULL_MODE=görünür|gizli|arkada|duvar|transform` — beş yapılandırma
//!   * `GIZMO_CULL_COUNT=<n>` — küp sayısı (varsayılan 12000)
//!   * **Sağ-tık + fare / WASDQE** — kamera (ölçüm için dokunmayın)

use gizmo::analysis::{AnalysisPlugin, Analyzer};
use gizmo::core::query::{Mut, Query, With};
use gizmo::core::system::{IntoSystemConfig, Phase, Res, ResMut};
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Kare süresinin yazıldığı seri.
const METRIC: &str = "occlusion_culling.frame_ms";
/// Kendi kendine ölçümün periyodu.
const PROBE_PERIOD: u32 = 200;
/// Küp bulutunun merkezi ve yarıçapı.
const CLOUD_CENTRE: Vec3 = Vec3::new(0.0, 0.0, -30.0);
const CLOUD_RADIUS: f32 = 9.0;

/// Üç yapılandırma. Üçünde de **aynı sayıda küp** dünyada duruyor; değişen tek şey kameranın
/// nereye baktığı ve araya bir duvar girip girmediği.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Küpler görüş alanında ve önlerinde hiçbir şey yok. Taban ölçüm.
    Visible,
    /// Küpler görüş alanında ama **tamamen bir duvarın arkasında**. Occlusion culling olsaydı
    /// bu, `Hidden` kadar ucuz olurdu.
    Behind,
    /// Kamera ters yöne bakıyor — küpler frustum'un dışında. Frustum culling'in ölçüsü bu.
    Away,
    /// **Yalnız duvar, sıfır küp.** `Behind` ile arasındaki fark, tamamen görünmez 
    /// geometrinin maliyetidir — duvarın kendi maliyeti iki tarafta da aynı olduğu için düşer.
    WallOnly,
    /// Küpler var ama **`MeshRenderer` taşımıyorlar**: dönüşüm sistemleri onları geziyor,
    /// çizim yığınlayıcısı hiç görmüyor. Maliyetin hangi yarısının nerede olduğunu ayırıyor.
    TransformOnly,
}

impl Mode {
    fn from_env() -> Self {
        match std::env::var("GIZMO_CULL_MODE").as_deref() {
            Ok("arkada") => Mode::Behind,
            Ok("gizli") => Mode::Away,
            Ok("duvar") => Mode::WallOnly,
            Ok("transform") => Mode::TransformOnly,
            _ => Mode::Visible,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Mode::Visible => "görünür (taban)",
            Mode::Behind => "duvarın arkasında",
            Mode::Away => "frustum dışında",
            Mode::WallOnly => "yalnız duvar",
            Mode::TransformOnly => "yalnız dönüşüm",
        }
    }
}

/// Ölçüm defteri.
#[derive(Clone, Copy)]
struct Bench {
    mode: Mode,
    cubes: usize,
    frame: u32,
    /// Frustum sınamasının bu karede kaç küpü geçirdiği — motorun kendi işlevi çağrılarak.
    in_frustum: usize,
}
gizmo::core::impl_component!(Bench);

fn cube_count() -> usize {
    std::env::var("GIZMO_CULL_COUNT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60_000usize)
        .clamp(1, 200_000)
}

fn main() {
    let mode = Mode::from_env();
    let cubes = cube_count();

    App::<SimpleSceneState>::new("Gizmo Engine - Occlusion Culling", 1280, 720)
        .add_plugin(AnalysisPlugin::new())
        .with_simple_scene(move |scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;
            let cube = AssetManager::create_cube(device);

            // Küp bulutu. `WallOnly` kipinde hiç doğmuyor — karşılaştırmanın öteki ucu o.
            let spawn = if mode == Mode::WallOnly { 0 } else { cubes };
            for i in 0..spawn {
                let p = CLOUD_CENTRE + shell_point(i, cubes.max(2)) * CLOUD_RADIUS;
                let entity = scene.world.spawn_bundle((
                    Transform::new(p).with_scale(Vec3::splat(0.16)),
                    GlobalTransform::default(),
                    cube.clone(),
                    Material::new(white.clone()).with_pbr(
                        Vec4::new(0.85, 0.55, 0.30, 1.0),
                        0.5,
                        0.0,
                    ),
                ));
                if mode != Mode::TransformOnly {
                    // `MeshRenderer` yoksa yığınlayıcı bu varlığı hiç görmüyor.
                    scene.world.add_component(entity, MeshRenderer::new());
                }
            }

            // Duvar: yalnız `Behind` kipinde, ve bulutu tamamen kapatacak kadar büyük.
            if matches!(mode, Mode::Behind | Mode::WallOnly) {
                scene.world.spawn_bundle((
                    Transform::new(Vec3::new(0.0, 0.0, -12.0))
                        .with_scale(Vec3::new(60.0, 60.0, 0.5)),
                    GlobalTransform::default(),
                    AssetManager::create_cube(device),
                    Material::new(white.clone()).with_pbr(
                        Vec4::new(0.20, 0.22, 0.26, 1.0),
                        0.9,
                        0.0,
                    ),
                    MeshRenderer::new(),
                ));
            }

            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.4) * Quat::from_rotation_x(-0.7),
                intensity: 2.4,
                ..Default::default()
            });
            let _ = white;

            // Kamera: `Away` kipinde ters yöne bakıyor, ötekilerde buluta.
            let look = if mode == Mode::Away {
                Vec3::new(0.0, 0.0, 60.0)
            } else {
                CLOUD_CENTRE
            };
            scene.spawn_camera(state, Vec3::new(0.0, 0.0, 6.0), look);

            gizmo::gizmo_log!(Info, "kip: {} · küp: {}", mode.label(), cubes);
            scene.world.insert_resource(Bench {
                mode,
                cubes,
                frame: 0,
                in_frustum: 0,
            });
        })
        .add_update_system(measure.in_phase(Phase::Update))
        .set_ui(|world, _state, ctx| {
            let Some(b) = world.get_resource::<Bench>().map(|b| *b) else {
                return;
            };
            let stats = world
                .get_resource::<Analyzer>()
                .and_then(|a| a.stats(METRIC));
            gizmo::egui::Area::new("cull".into())
                .anchor(gizmo::egui::Align2::RIGHT_TOP, [-12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(430.0);
                        ui.heading("Ayıklama");
                        ui.label(format!("kip: {} · {} küp", b.mode.label(), b.cubes));
                        if let Some(s) = stats {
                            ui.monospace(format!(
                                "kare {:.2} ms · p95 {:.2} ms · ~{:.0} FPS",
                                s.mean,
                                s.p95,
                                1000.0 / s.mean.max(0.001)
                            ));
                        }
                        ui.monospace(format!("frustum'u geçen: {} / {}", b.in_frustum, b.cubes));
                        ui.separator();
                        ui.label("frustum ayıklaması VAR (visible_in_frustum),");
                        ui.label("gölge kademeleri için de çalışıyor.");
                        ui.separator();
                        ui.colored_label(
                            gizmo::egui::Color32::from_rgb(230, 160, 80),
                            "occlusion ayıklaması YOK",
                        );
                        ui.label("Hi-Z yok, sorgu yok, geçen kareden tahmin yok.");
                        ui.label("bir duvarın arkasındaki küpler yine çiziliyor.");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Kare süresini deftere yazar, ve motorun kendi frustum sınamasını çağırarak kaç küpün
/// geçtiğini sayar.
fn measure(
    cubes: Query<(&Transform, With<MeshRenderer>)>,
    mut cameras: Query<(Mut<Camera>, Mut<Transform>)>,
    mut bench: ResMut<Bench>,
    mut analyzer: ResMut<Analyzer>,
    time: Res<Time>,
) {
    bench.frame += 1;
    analyzer.sample(METRIC, (time.dt() * 1000.0) as f64);

    // Frustum'u kameradan kur ve motorun kendi işleviyle say. Bu, batcher'ın çağırdığı işlevin
    // aynısı — yani "kaç nesne ayıklandı" sorusunun motordaki cevabı.
    //
    // **Seyrek**: bu döngü 60 000 nesneyi geziyor ve her karede koşarsa ölçümün kendisi oluyor.
    // İlk kurulumda öyleydi ve sonuçları çöpe çevirdi — bkz. başlıktaki ölçüm notu.
    if !bench.frame.is_multiple_of(PROBE_PERIOD) {
        return;
    }
    let cam = cameras
        .iter_mut()
        .next()
        .map(|(_, (c, t))| (c.get_projection(16.0 / 9.0), c.get_view(t.position)));
    if let Some((proj, view)) = cam {
        let frustum = gizmo::math::Frustum::from_matrix(&(proj * view));
        let unit = gizmo::math::Aabb {
            min: Vec3::splat(-0.5).into(),
            max: Vec3::splat(0.5).into(),
        };
        let mut n = 0usize;
        for (_e, (t, _)) in cubes.iter() {
            let m = Mat4::from_scale_rotation_translation(t.scale, t.rotation, t.position);
            if gizmo::renderer::frustum_cull::visible_in_frustum(&frustum, &m, unit) {
                n += 1;
            }
        }
        bench.in_frustum = n;
    }

    if std::env::var("GIZMO_CULL_SELFTEST").is_ok() {
        if let Some(s) = analyzer.stats(METRIC) {
            gizmo::gizmo_log!(
                Info,
                "kip {:<20} · küp {} · frustum'u geçen {} · ort {:.2} ms · p95 {:.2} ms",
                bench.mode.label(),
                bench.cubes,
                bench.in_frustum,
                s.mean,
                s.p95
            );
        }
    }
}

/// `i`. noktanın küre kabuğundaki yeri — altın açı sarmalı, yani düzgün dağılım.
fn shell_point(i: usize, total: usize) -> Vec3 {
    use std::f32::consts::PI;
    let golden = PI * (3.0 - 5.0f32.sqrt());
    let y = 1.0 - (i as f32 / (total.max(2) - 1) as f32) * 2.0;
    let r = (1.0 - y * y).max(0.0).sqrt();
    let theta = golden * i as f32;
    Vec3::new(theta.cos() * r, y, theta.sin() * r)
}
