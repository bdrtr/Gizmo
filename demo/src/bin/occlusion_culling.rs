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
//! zaten depoda — ama "Ölçülen 2"nin gösterdiği gibi, aracı takmak tek başına kazanç değil.
//!
//! ## Ölçülen 2: indeksi bağlamak **her zaman** kazanç değil
//!
//! Yukarıdaki tablo "yürüyüşü kısaltmanın aracı zaten depoda" diyordu — `RenderAabbTree`
//! (`CAPABILITY_GAPS.md` §F1). Demo artık o aracı aynı karede doğrusal yürüyüşle yan yana
//! ölçüyor, ve cevap tek yönlü değil.
//!
//! Ölçüldü (2026-08-24, 60 000 küp, kare 620, ağaç **bir kez** kurulmuş —
//! `GIZMO_CULL_INDEX_STATIC=1`):
//!
//! | kip | indeks sorgusu | doğrusal yürüyüş | |
//! |-----|----------------|------------------|---|
//! | görünür (hiçbir şey elenmiyor) | **5,020 ms** | 1,864 ms | ağaç **2,7× yavaş** |
//! | gizli (tamamı eleniyor) | **0,001 ms** | 1,241 ms | ağaç **1240× hızlı** |
//!
//! Uçlar arası fark bir milyon kat. Sebebi basit: eleme yapıldığında kök tek bir düzlem testinde
//! reddediliyor ve sorgu biter; eleme yapılmadığında ağaç 60 000 anahtarı bir `Vec`'e yazmak
//! zorunda, ve **yazmak testten pahalı**.
//!
//! ### Ve ağacı güncel tutmak kazancın çoğunu yiyor
//!
//! Aynı ölçüm, ağaç **her kare** güncellenirse:
//!
//! | kip | güncelleme | sorgu | toplam | doğrusal |
//! |-----|------------|-------|--------|----------|
//! | görünür | 2,143 ms | 4,987 ms | **7,130 ms** | 2,278 ms |
//! | gizli | 1,413 ms | 0,001 ms | **1,415 ms** | 1,652 ms |
//!
//! Elemenin tam olduğu kipte bile kazanç **%14**'e düşüyor. 12 000 küpte ağaç her iki kipte de
//! kaybediyor (0,671 ms'ye karşı 0,224 ms).
//!
//! `insert` hareketsiz bir nesne için kısayol taşıyor — yeni kutu yaprağın içindeyse ağaca
//! dokunmuyor — ve bu sahnedeki küpler hiç hareket etmiyor. Yani 1,4 ms'nin tamamı **kısayolun
//! kendi maliyeti**: 60 000 çağrı, her biri bir arama ve bir kapsama testi.
//!
//! ## Ölçülen 3: gölge kaskadları kararı veriyor
//!
//! Yukarıdaki sayılar tek bir frustum içindi. Yığınlayıcı ise **kamera ve dört kaskadın
//! birleşimini** soruyor, çünkü kadraj dışındaki bir gölge atıcı da gölge haritasına çizilmeli.
//! `VisibilityIndex` kaynağı takılıp aday sayısı izlendiğinde (2026-08-24, 60 000 küp):
//!
//! | sahne | aday | sorgu |
//! |-------|------|-------|
//! | güneş açık, hepsi kadrajda | 60 000 | 12,3 ms |
//! | güneş açık, hepsi **kameranın arkasında** | **60 000** | 5,4 ms |
//! | güneş kapalı, hepsi kameranın arkasında | **0** | **0,005 ms** |
//!
//! Gölge atan bir güneş varken indeks **hiçbir şey elemiyor** — ikisinde de. Kaskadlar sahneyi
//! kapsıyor, yani birleşimleri kameranın reddettiği her şeyi kabul ediyor. Üstüne 60 000 anahtarı
//! bir `Vec`'e yazıp sıralamanın bedeli biniyor.
//!
//! Kare süresine yansıması (`GIZMO_CULL_USE_INDEX=1`):
//!
//! | sahne | indekssiz | indeksli | |
//! |-------|-----------|----------|---|
//! | güneş açık, görünür | 18,84 ms | **35,75 ms** | %90 **yavaş** |
//! | güneş açık, gizli | 19,73 ms | **27,23 ms** | %38 **yavaş** |
//! | güneş kapalı, gizli | 12,72 ms | **10,14 ms** | %20 **hızlı** |
//!
//! ### Kararı ne veriyor
//!
//! `CAPABILITY_GAPS.md` §F1 şöyle diyordu: *"an index in front of it is precisely what removes
//! that walk for culled objects"*. Bu, **kamera frustumu için doğru, yığınlayıcının gerçekte
//! sorduğu birleşim için yanlış** — ve aradaki fark çoğu sahnede kararı tersine çeviriyor.
//!
//! Yani indeks bir kapı, varsayılan değil, ve koşulları üstünde yazılı:
//!
//!   * sahne statik olmalı (her kare güncelleme kazancı yiyor: %14'e düşüyor),
//!   * eleme oranı yüksek olmalı (düşükse çıktıyı yazmak testten pahalı),
//!   * ve **kaskadlar sahnenin tamamını kapsamamalı** — beni şaşırtan koşul bu oldu.
//!
//! Ağacın kendi modül belgesi zaten uyarıyordu: *"Measure on your own scene before believing any
//! of the above"*. Bu demo o ölçümü motorun kendi sahnesinde yaptı ve uyarı haklı çıktı.
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
//!   * `GIZMO_CULL_INDEX_STATIC=1` — uzamsal indeksi her kare değil bir kez kur
//!   * `GIZMO_CULL_USE_INDEX=1` — indeksi motorun çizim yoluna gerçekten tak
//!   * `GIZMO_CULL_NO_SUN=1` — güneşi gölgesiz dolgu ışığına çevir (kaskad yok)
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
    /// Uzamsal indeksi bu karede güncellemenin süresi, ms.
    index_update_ms: f32,
    /// Onu sorgulamanın süresi, ms.
    index_query_ms: f32,
    /// Doğrusal yürüyüşün aynı karedeki süresi, karşılaştırma için.
    linear_ms: f32,
    /// İndeksin döndürdüğü aday sayısı.
    index_hits: usize,
    /// Kamera + kaskad birleşimini sorgulamanın süresi — yığınlayıcının gerçekte yaptığı iş.
    index_union_ms: f32,
    /// O birleşimin aday sayısı.
    index_union_hits: usize,
}
gizmo::core::impl_component!(Bench);

/// Uzamsal indeks ve sorgu tamponu.
///
/// `Bench`'in içinde değil: `Component` `Clone` istiyor ve bir BVH'yi kare başına klonlamak
/// ölçülmek istenen şeyin yanında anlamsız bir maliyet olurdu.
static INDEX: std::sync::OnceLock<
    std::sync::Mutex<(gizmo::renderer::visibility::RenderAabbTree, Vec<u32>)>,
> = std::sync::OnceLock::new();

fn index() -> &'static std::sync::Mutex<(gizmo::renderer::visibility::RenderAabbTree, Vec<u32>)> {
    INDEX.get_or_init(|| {
        std::sync::Mutex::new((
            gizmo::renderer::visibility::RenderAabbTree::new(),
            Vec::new(),
        ))
    })
}

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
                // `GIZMO_CULL_NO_SUN=1` güneşi gölgesiz dolgu ışığına çeviriyor: kaskad yok, yani
                // yığınlayıcı yalnız kamera frustumunu soruyor. İndeksin eleme oranı buna bağlı.
                role: if std::env::var("GIZMO_CULL_NO_SUN").is_ok() {
                    gizmo::renderer::components::LightRole::Generic
                } else {
                    gizmo::renderer::components::LightRole::Sun
                },
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
            // `GIZMO_CULL_USE_INDEX=1` indeksi motorun çizim yoluna gerçekten takıyor:
            // `VisibilityIndex` kaynağı varsa yığınlayıcı her mesh'i gezmek yerine ağacın
            // adaylarını geziyor. Bir kez kuruluyor — bu sahne hareketsiz, ve "Ölçülen 2"
            // her kare kurmanın kazancı yediğini gösteriyor.
            if std::env::var("GIZMO_CULL_USE_INDEX").is_ok() {
                gizmo::systems::ensure_global_transforms(scene.world);
                let mut index =
                    gizmo::systems::render::visibility_index::VisibilityIndex::default();
                index.rebuild_from(scene.world);
                gizmo::gizmo_log!(Info, "uzamsal indeks kuruldu: {} varlık", index.len());
                scene.world.insert_resource(index);
            }

            scene.world.insert_resource(Bench {
                mode,
                cubes,
                frame: 0,
                in_frustum: 0,
                index_update_ms: 0.0,
                index_query_ms: 0.0,
                linear_ms: 0.0,
                index_hits: 0,
                index_union_ms: 0.0,
                index_union_hits: 0,
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
///
/// Ayrıca **uzamsal indeksin** aynı işi ne kadar sürede yaptığını ölçer: `RenderAabbTree` motorda
/// yazılı ama hiçbir çizim yolu onu çağırmıyor (`CAPABILITY_GAPS.md` §F1). Bağlamaya değip
/// değmediğini söyleyecek olan şey, ağacı **güncel tutmanın** maliyetinin sorguda kazandırdığından
/// az olup olmadığı — ve bu ölçülmeden bilinmiyor.
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
        // ── Doğrusal yürüyüş: batcher'ın yaptığı işin aynısı ────────────────
        let t_linear = std::time::Instant::now();
        let mut n = 0usize;
        for (_e, (t, _)) in cubes.iter() {
            let m = Mat4::from_scale_rotation_translation(t.scale, t.rotation, t.position);
            if gizmo::renderer::frustum_cull::visible_in_frustum(&frustum, &m, unit) {
                n += 1;
            }
        }
        bench.in_frustum = n;
        bench.linear_ms = t_linear.elapsed().as_secs_f32() * 1000.0;

        // ── Uzamsal indeks: güncelle, sonra sorgula ─────────────────────────
        //
        // Güncelleme her varlık için bir `insert` — ve `insert` ucuz bir kısayol taşıyor: yeni
        // kutu mevcut yaprağın içinde kalıyorsa ağaca dokunmuyor. Yani hareketsiz bir sahnede
        // güncelleme neredeyse bedava, hareketli bir sahnede değil. Ölçülecek şey tam olarak bu.
        let mut guard = index().lock().expect("index mutex");
        let (tree, out) = &mut *guard;

        // `GIZMO_CULL_INDEX_STATIC=1` ağacı yalnız bir kez kuruyor. Sahne hareketsiz olduğu için
        // sonucu değiştirmiyor — değiştirdiği tek şey, güncelleme maliyetinin ölçüme girip
        // girmediği. İkisi de ölçülmeli, çünkü "ağaç kazanır mı" sorusunun cevabı buna bağlı.
        let rebuild_every_frame = std::env::var("GIZMO_CULL_INDEX_STATIC").is_err();
        let t_update = std::time::Instant::now();
        if rebuild_every_frame || tree.is_empty() {
            for (e, (t, _)) in cubes.iter() {
                let m = Mat4::from_scale_rotation_translation(t.scale, t.rotation, t.position);
                tree.insert(e, unit.transform(&m));
            }
        }
        bench.index_update_ms = t_update.elapsed().as_secs_f32() * 1000.0;

        let t_query = std::time::Instant::now();
        out.clear();
        tree.query_frustum(&frustum, out);
        bench.index_query_ms = t_query.elapsed().as_secs_f32() * 1000.0;
        bench.index_hits = out.len();

        // Yığınlayıcının gerçekten yaptığı sorgu bu değil: o, kamera **ve dört kaskad**
        // frustumunun birleşimini istiyor, çünkü kadraj dışındaki bir gölge atıcı da çizilmeli.
        // Aradaki fark ölçülmeli — kaskadlar sahnenin çoğunu kapsıyorsa eleme oranı çöker.
        let cascade_frusta: Vec<gizmo::math::Frustum> = (0..4)
            .map(|_| frustum) // kaba yaklaşım: gerçek kaskadlar daha geniş, yani bu bir ALT sınır
            .collect();
        let mut all = Vec::with_capacity(1 + cascade_frusta.len());
        all.push(frustum);
        all.extend_from_slice(&cascade_frusta);
        let t_union = std::time::Instant::now();
        let mut union_out = Vec::new();
        tree.query_frusta(&all, &mut union_out);
        bench.index_union_ms = t_union.elapsed().as_secs_f32() * 1000.0;
        bench.index_union_hits = union_out.len();
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
            gizmo::gizmo_log!(
                Info,
                "  indeks: güncelleme {:.3} ms · sorgu {:.3} ms · toplam {:.3} ms · aday {} \
                 | doğrusal {:.3} ms",
                bench.index_update_ms,
                bench.index_query_ms,
                bench.index_update_ms + bench.index_query_ms,
                bench.index_hits,
                bench.linear_ms
            );
            gizmo::gizmo_log!(
                Info,
                "  birleşim (kamera+4 kaskad): {:.3} ms · aday {} — yığınlayıcının yaptığı bu",
                bench.index_union_ms,
                bench.index_union_hits
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
