//! # Çizelgenin iç yapısı
//!
//! Bu demo dört ayrı yeteneği birden ele alıyor, çünkü dördünün de cevabı aynı yerden —
//! **çizelgenin kendisinden** — geliyor: uygulamaya özel çizelge eklemek, yürütücüyü
//! değiştirmek, kısıtsız sistem sırasının değişkenliği, ve sistem sistem adımlama.
//!
//! ## Motorda çizelgenin neresi açık
//!
//! | yetenek | Gizmo |
//! |---------|-------|
//! | uygulamaya özel bir faz eklemek | **var** — `Phase::User(konum)`, aşağıda ölçüldü |
//! | yürütücüyü seçmek ya da değiştirmek | **yok** — yürütücü `Schedule::run_batches`, özel ve değiştirilemez |
//! | kısıtsız sistem sırasının değişkenliği | **kısmen** — aşağıda ölçüldü |
//! | sistem sistem ilerletme (adımlama) | **yok** — ama `Schedule::run` açık, adımlamayı uygulama kurabilir |
//!
//! ## Ölçülen 0: kullanıcı fazı yerleşiklerin **arasına** giriyor
//!
//! `Phase` eskiden beş varyantlı kapalı bir enum'du ve bu satır "uygulamaya özel çizelge: yok"
//! diyordu. Artık `Phase::User(u16)` var, ve taşıdığı sayı yerleşiklerin oturduğu ölçekteki
//! **konumu**: `PreUpdate`=1000, `Update`=2000, `Physics`=3000, `PostUpdate`=4000, `Render`=5000.
//! Yani `User(3500)` "fizik oturduktan sonra, dönüşümler yayılmadan önce" demek — sona eklemek
//! değil, araya girmek.
//!
//! Bu demo on probe koşuyor: beş yerleşik faz, ve aralarına + iki ucuna serpilmiş beş kullanıcı
//! fazı (`500`, `1500`, `2500`, `3500`, `4500`). Ekleme sırası kasten karışık. Ölçüldü
//! (2026-08-23, 600 kare):
//!
//! | | |
//! |---|---|
//! | beklenen dizi | `0P1U2F3O4R` |
//! | beklenen sırayla koşan kare | **599** |
//! | sapan kare | **0** |
//!
//! Diziyi okuyan sistemin kendisi de ölçümün parçası: `Phase::User(5001)`'de, yani
//! **`Render`'dan sonra**. Motorda `Render`'dan sonra iş yapmanın yolu daha önce yoktu.
//!
//! Sıralama türetilmiş `Ord`'dan gelemezdi — türetilmiş bir `Ord` veri taşıyan varyantı bildirim
//! sırasına göre *her* yerleşik fazın arkasına atardı ve yukarıdaki tablo kurulamazdı. Bu yüzden
//! `Ord` elle yazılı ve tek ölçüsü `Phase::position()`.
//!
//! ## Ölçülen 1: yığın düzeni deterministik, yığın **içi** değil
//!
//! "Kısıt koymazsan sıra değişir" kuralı Gizmo'da **ikiye ayrılıyor**, ve ayrımı çizelgenin
//! kendi belgesi yazıyor:
//!
//! * **Yığınlara dağılım deterministik.** `build()` topolojik sıralamayı Kahn ile yapıp
//!   sistemleri açgözlü paketliyor, ve tek eşitlik bozucu **ekleme sırası** — belgenin kendi
//!   ifadesiyle *"insertion order is the only tie-break, which is what makes the layout
//!   deterministic."* Yani aynı program aynı düzeni kuruyor.
//!
//! * **Bir yığının içi değil.** Aynı yığındaki sistemler rayon ile **eşzamanlı ve sırasız**
//!   koşuyor. Erişimleri çakışmadığı için bu normalde görünmez — ama belge bir deliği açıkça
//!   adlandırıyor: `Commands` alan iki sistem yalnız komut kuyruğunun **okunduğunu** bildiriyor,
//!   yani aynı yığına düşebiliyorlar ve **kuyruğa yazma sıraları yeniden üretilemez**.
//!
//! Demo tam olarak o deliği ölçüyor: kısıtsız, `Commands` alan iki sistem, her karede hangisinin
//! önce yazdığını sayıyor.
//!
//! ## Ölçüldü
//!
//! Sekiz kısıtsız yazıcı, aralarında hiçbir `before`/`after` yok, hepsi `Commands` alıyor.
//! Ölçüldü (2026-08-23, 600 kare, iki ayrı koşu):
//!
//! | | farklı sıralama | sıra değişimi | en sık sıralama |
//! |---|-----------------|---------------|-----------------|
//! | koşu 1 | **78** | 373 / 600 | `ABCDEFGH` ×321 (%53) |
//! | koşu 2 | **54** | 309 / 600 | `ABCDEFGH` ×379 (%63) |
//! | `GIZMO_SCHEDULE_CHAIN=1` | **1** | **0** / 600 | `ABCDEFGH` ×600 (%100) |
//!
//! Yani kısıtsız sıra gerçekten yeniden üretilemez: 600 karede onlarca farklı sıralama, ve iki
//! koşu birbirini tutmuyor. Ekleme sırası (`ABCDEFGH`) en sık çıkan hâl ama yarıdan biraz fazla —
//! **garanti değil**.
//!
//! ### Ölçüm notu: iki yazıcı bunu göstermiyor
//!
//! İlk kurulumda yalnız **iki** kısıtsız yazıcı vardı ve sonuç iki koşuda da 600/600 aynıydı —
//! sıfır değişim. Bu, sıranın garantili olduğu anlamına gelmiyordu; iki öğelik bir dilimde
//! rayon'un iş çalmasına gerek kalmıyor, yani ikisi de çağıran iş parçacığında sırayla koşuyor.
//! Yazıcı sayısı sekize çıkınca havuz gerçekten bölünüyor ve değişkenlik ortaya çıkıyor.
//!
//! Dersi genel: **"ölçtüm, değişmedi" ile "değişmez" aynı şey değil.** Küçük bir çizelgede
//! kısıtsız sıra kararlı görünüp üretimde bozulabilir.
//!
//! ### Ölçüm notu: aynı ders ikinci kez, ters yönden
//!
//! Yukarıdaki tablo (78 ve 54 farklı sıralama) faz probe'ları **eklenmeden önceki** koşulardan.
//! On probe eklendikten sonra aynı sekiz yazıcı 600 karede **180** farklı sıralama üretti, 575
//! sıra değişimi — üç kattan fazla. Yazıcılara dokunulmadı; değişen tek şey çizelgede başka
//! sistemlerin olması, yani havuzun daha çok bölünmesi.
//!
//! Bu ilk ölçüm notunun ikinci yarısı: sekiz yazıcı da "değişkenliğin tamamını" göstermiyordu.
//! Bir sayıyı düşürmek için değil, **yükseltmek** için de komşu iş yeter — yani kısıtsız sıranın
//! değişkenliği o sistemin kendi özelliği değil, çizelgenin o anki yükünün özelliği.
//!
//! ### Ve çare gerçekten çalışıyor
//!
//! Aynı sekiz sistem `after` zinciriyle bağlandığında (`GIZMO_SCHEDULE_CHAIN=1`) 600 karede
//! **tek** sıralama görülüyor, sıfır değişim. Yani `label`/`after` kısıtları sırayı gerçekten
//! sabitliyor — sadece elle yazmak gerekiyor: bir sistem listesini tek çağrıda zincirleyen
//! kolaylık **yok**.
//!
//! Bir uyarı: kısıtı **çalıştırma anında** takıp çıkarmak mümkün değil, çizelge kurulurken
//! belirleniyor. Karşılaştırma bu yüzden iki ayrı koşu.
//!
//! ## Kontroller
//!   * `GIZMO_SCHEDULE_CHAIN=1` ile başlatın — sekiz yazıcı `after` zinciriyle bağlanır
//!   * **Sağ-tık + fare / WASDQE** — kamera

use gizmo::core::input::Input;
use gizmo::core::system::{IntoSystemConfig, Phase, Res, ResMut};
use gizmo::core::Commands;
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};
use std::sync::{Mutex, OnceLock};

/// İki sistemin karede hangi sırayla yazdığını tutan defter.
///
/// Kaynak (`ResMut`) DEĞİL, bilerek: iki sistem aynı kaynağa yazsaydı erişimleri çakışır ve
/// çizelge onları ayrı yığınlara koyardı — yani ölçmek istediğim durumu ölçüm aracı yok ederdi.
static ORDER_LOG: OnceLock<Mutex<Vec<u8>>> = OnceLock::new();

fn order_log() -> &'static Mutex<Vec<u8>> {
    ORDER_LOG.get_or_init(|| Mutex::new(Vec::new()))
}

/// Faz sırasını kaydeden ayrı defter. Yazıcılarınkinden ayrı, çünkü ölçtükleri şey farklı:
/// bu, faz*lar arası* sırayı ölçüyor, yazıcılar faz *içi* sırayı.
static PHASE_LOG: OnceLock<Mutex<Vec<u8>>> = OnceLock::new();

fn phase_log() -> &'static Mutex<Vec<u8>> {
    PHASE_LOG.get_or_init(|| Mutex::new(Vec::new()))
}

/// On probe'un beklenen sırası: yerleşikler ve aralarına yerleştirilmiş beş kullanıcı fazı.
const EXPECTED_PHASE_ORDER: &str = "0P1U2F3O4R";

/// Kaç kısıtsız yazıcı var. İkisi rayon'un iş çalmasına yetmiyor — aşağıdaki ölçüm notuna bakın.
const WRITERS: usize = 8;

/// Ölçüm defteri.
#[derive(Default, Clone)]
struct ScheduleReport {
    frame: u32,
    /// Bu koşuda görülen farklı sıralamalar ve kaçar kez görüldükleri.
    seen: Vec<(String, u32)>,
    /// Sıranın bir önceki kareye göre değiştiği kare sayısı.
    flips: u32,
    last: String,
    /// Faz probe'larının beklenen sırayla koştuğu kare sayısı.
    phase_ok: u32,
    /// Beklenenden sapan kare sayısı.
    phase_bad: u32,
    /// Sapma görüldüyse son sapan dizi.
    phase_last_bad: String,
}
gizmo::core::impl_component!(ScheduleReport);

/// Zincir modu açıksa sisteme `after` kısıtını takar, kapalıysa dokunmaz.
///
/// Kısıtı **çalıştırma anında** eklemek mümkün değil — çizelge kurulurken belirleniyor. Bu yüzden
/// karşılaştırma iki ayrı koşuyla yapılıyor, tek koşu içinde bir tuşla değil.
fn chained(
    config: gizmo::core::system::SystemConfig,
    after: Option<&'static str>,
) -> gizmo::core::system::SystemConfig {
    match (chain_enabled(), after) {
        (true, Some(label)) => config.after(label),
        _ => config,
    }
}

/// Zincir modu açık mı.
fn chain_enabled() -> bool {
    std::env::var("GIZMO_SCHEDULE_CHAIN").is_ok()
}

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Schedule Internals", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;
            let cube = AssetManager::create_cube(device);

            // Beş faz, beş küp — `Phase` kapalı bir enum, altıncısı eklenemiyor.
            for (i, phase) in Phase::ALL.iter().enumerate() {
                let x = (i as f32 - 2.0) * 1.6;
                let _ = phase;
                scene.world.spawn_bundle((
                    Transform::new(Vec3::new(x, 0.0, 0.0)).with_scale(Vec3::splat(0.55)),
                    GlobalTransform::default(),
                    cube.clone(),
                    Material::new(white.clone()).with_pbr(
                        Vec4::new(0.30 + i as f32 * 0.13, 0.55, 0.85 - i as f32 * 0.11, 1.0),
                        0.5,
                        0.0,
                    ),
                    MeshRenderer::new(),
                ));
            }
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -1.0, 0.0)),
                GlobalTransform::default(),
                AssetManager::create_plane(device, 30.0),
                Material::new(white).with_pbr(Vec4::new(0.12, 0.13, 0.15, 1.0), 1.0, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.5) * Quat::from_rotation_x(-0.6),
                intensity: 2.5,
                ..Default::default()
            });
            scene.world.insert_resource(ScheduleReport::default());
            scene.spawn_camera(state, Vec3::new(0.0, 2.5, 9.0), Vec3::ZERO);
        })
        // Kasıtlı olarak KISITSIZ ve hepsi `Commands` alıyor: belgenin adlandırdığı delik bu.
        // `GIZMO_SCHEDULE_CHAIN=1` ile aynı sekiz sistem `after` zinciriyle bağlanıyor —
        // karşılaştırma böyle kuruluyor.
        .add_update_system(chained(writer_a.in_phase(Phase::Update).label("w_a"), None))
        .add_update_system(chained(writer_b.in_phase(Phase::Update).label("w_b"), Some("w_a")))
        .add_update_system(chained(writer_c.in_phase(Phase::Update).label("w_c"), Some("w_b")))
        .add_update_system(chained(writer_d.in_phase(Phase::Update).label("w_d"), Some("w_c")))
        .add_update_system(chained(writer_e.in_phase(Phase::Update).label("w_e"), Some("w_d")))
        .add_update_system(chained(writer_f.in_phase(Phase::Update).label("w_f"), Some("w_e")))
        .add_update_system(chained(writer_g.in_phase(Phase::Update).label("w_g"), Some("w_f")))
        .add_update_system(chained(writer_h.in_phase(Phase::Update).label("w_h"), Some("w_g")))
        // Sırayı okuyan sistem `PostUpdate`'te — faz sınırı, yığın sınırından güçlü.
        .add_update_system(tally.in_phase(Phase::PostUpdate))
        // Faz probe'ları: beş yerleşik fazın arasına ve iki ucuna yerleştirilmiş beş kullanıcı
        // fazı. Ekleme sırası kasten karışık — sıra fazdan gelmeli, buradan değil.
        .add_update_system(probe_render.in_phase(Phase::Render))
        .add_update_system(probe_physics_to_post.in_phase(Phase::User(3500)))
        .add_update_system(probe_pre.in_phase(Phase::PreUpdate))
        .add_update_system(probe_post_to_render.in_phase(Phase::User(4500)))
        .add_update_system(probe_update.in_phase(Phase::Update))
        .add_update_system(probe_before_pre.in_phase(Phase::User(500)))
        .add_update_system(probe_post.in_phase(Phase::PostUpdate))
        .add_update_system(probe_update_to_physics.in_phase(Phase::User(2500)))
        .add_update_system(probe_physics.in_phase(Phase::Physics))
        .add_update_system(probe_pre_to_update.in_phase(Phase::User(1500)))
        // Ve okuyucu `Render`'dan sonra.
        .add_update_system(phase_tally.in_phase(Phase::User(5001)))
        .set_ui(|world, _state, ctx| {
            let Some(r) = world.get_resource::<ScheduleReport>().map(|r| r.clone()) else {
                return;
            };
            gizmo::egui::Area::new("sched".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(480.0);
                        ui.heading("Çizelgenin iç yapısı");
                        ui.label("Phase: 5 yerleşik + Phase::User(konum) — özel faz VAR.");
                        ui.label("yürütücü: Schedule::run_batches, özel ve değiştirilemez.");
                        ui.label("adımlama (stepping) API'si yok.");
                        ui.separator();
                        ui.monospace(format!("faz sırası beklenen: {EXPECTED_PHASE_ORDER}"));
                        ui.label(format!(
                            "tutan kare: {} · sapan: {}",
                            r.phase_ok, r.phase_bad
                        ));
                        if !r.phase_last_bad.is_empty() {
                            ui.colored_label(
                                gizmo::egui::Color32::from_rgb(230, 160, 80),
                                format!("son sapma: {}", r.phase_last_bad),
                            );
                        }
                        ui.separator();
                        ui.label(format!(
                            "kare: {} · {} yazıcı · zincir: {}",
                            r.frame,
                            WRITERS,
                            if chain_enabled() { "AÇIK" } else { "kapalı" }
                        ));
                        ui.label(format!("farklı sıralama: {}", r.seen.len()));
                        ui.label(format!("sıra değişimi  : {} kez", r.flips));
                        let mut top = r.seen.clone();
                        top.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
                        for (order, n) in top.iter().take(4) {
                            ui.monospace(format!("  {order}  ×{n}"));
                        }
                        ui.separator();
                        if r.seen.len() > 1 {
                            ui.colored_label(
                                gizmo::egui::Color32::from_rgb(230, 160, 80),
                                "-> yığın içi sıra YENİDEN ÜRETİLEMEZ",
                            );
                        } else if r.frame > 60 {
                            ui.label("-> bu koşuda tek sıralama görüldü (garanti değil)");
                        }
                        ui.separator();
                        ui.label("yığınlara dağılım deterministik (ekleme sırası eşitlik bozucu),");
                        ui.label("yığın İÇİ rayon ile eşzamanlı ve sırasız.");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Kısıtsız yazıcılar. Sekiz tane, aralarında **hiçbir** sıralama kısıtı yok, ve hepsi
/// `Commands` alıyor — yani hepsi yalnız komut kuyruğunun *okunduğunu* bildiriyor, dolayısıyla
/// çizelge hepsini aynı yığına koyabiliyor.
macro_rules! writer {
    ($name:ident, $tag:expr) => {
        fn $name(mut commands: Commands, _input: Res<Input>) {
            order_log().lock().unwrap().push($tag);
            // Komutu gerçekten kuyruğa koy: parametrenin ölü olmadığından emin ol.
            commands.spawn().despawn();
        }
    };
}
writer!(writer_a, b'A');
writer!(writer_b, b'B');
writer!(writer_c, b'C');
writer!(writer_d, b'D');
writer!(writer_e, b'E');
writer!(writer_f, b'F');
writer!(writer_g, b'G');
writer!(writer_h, b'H');

/// Faz probe'ları. Her biri kendi fazında tek harf yazıyor; sırayı `phase_tally` okuyor.
///
/// Rakamlar kullanıcı fazları, harfler yerleşikler — beklenen dizi
/// [`EXPECTED_PHASE_ORDER`]. Bunlar hiçbir kaynağa **yazmıyor**: yazsalardı erişimleri çakışır ve
/// çizelge onları zaten ayırırdı, yani ölçüm aracı ölçülecek şeyi kendisi kurmuş olurdu.
macro_rules! phase_probe {
    ($name:ident, $tag:expr) => {
        fn $name(_input: Res<Input>) {
            phase_log().lock().unwrap().push($tag);
        }
    };
}
phase_probe!(probe_before_pre, b'0');
phase_probe!(probe_pre, b'P');
phase_probe!(probe_pre_to_update, b'1');
phase_probe!(probe_update, b'U');
phase_probe!(probe_update_to_physics, b'2');
phase_probe!(probe_physics, b'F');
phase_probe!(probe_physics_to_post, b'3');
phase_probe!(probe_post, b'O');
phase_probe!(probe_post_to_render, b'4');
phase_probe!(probe_render, b'R');

/// Faz dizisini okur. Kendisi `Phase::User(5001)`'de, yani **`Render`'dan sonra** — okuduğu şeyin
/// tamamlanmış olması bu yüzden garanti, ve bu sistemin var olabilmesi ölçümün kendisi:
/// `Render`'dan sonra iş yapmanın yolu daha önce yoktu.
fn phase_tally(mut report: ResMut<ScheduleReport>) {
    let order: String = {
        let mut log = phase_log().lock().unwrap();
        let s = String::from_utf8_lossy(&log).to_string();
        log.clear();
        s
    };
    if order.len() != EXPECTED_PHASE_ORDER.len() {
        return; // yarım kare — sayma
    }
    if order == EXPECTED_PHASE_ORDER {
        report.phase_ok += 1;
    } else {
        report.phase_bad += 1;
        report.phase_last_bad = order;
    }
}

/// Kareyi kapatır ve o karenin sıralamasını deftere işler. Ayrı bir **fazda**, yani yazıcılarla
/// aynı yığına düşemez.
fn tally(mut report: ResMut<ScheduleReport>) {
    let order: String = {
        let mut log = order_log().lock().unwrap();
        let s = String::from_utf8_lossy(&log).to_string();
        log.clear();
        s
    };
    if order.len() != WRITERS {
        return; // yarım kare — sayma
    }

    report.frame += 1;
    if !report.last.is_empty() && order != report.last {
        report.flips += 1;
    }
    report.last = order.clone();
    if let Some(entry) = report.seen.iter_mut().find(|(o, _)| *o == order) {
        entry.1 += 1;
    } else {
        report.seen.push((order, 1));
    }

    if std::env::var("GIZMO_SCHEDULE_SELFTEST").is_ok() && report.frame.is_multiple_of(200) {
        let mut top = report.seen.clone();
        top.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        gizmo::gizmo_log!(
            Info,
            "faz sırası: beklenen {} · tutan {} kare · sapan {} kare{}",
            EXPECTED_PHASE_ORDER,
            report.phase_ok,
            report.phase_bad,
            if report.phase_last_bad.is_empty() {
                String::new()
            } else {
                format!(" · son sapma {}", report.phase_last_bad)
            }
        );
        gizmo::gizmo_log!(
            Info,
            "kare {:>4} · zincir {} · farklı sıralama {} · sıra değişimi {} · en sık {:?}",
            report.frame,
            if chain_enabled() { "AÇIK" } else { "kapalı" },
            report.seen.len(),
            report.flips,
            &top[..top.len().min(3)]
        );
    }
}
